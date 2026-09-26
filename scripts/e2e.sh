#!/usr/bin/env bash
# End-to-end check against a real herdr: a sandbox server with the plugin
# linked and a fake ai-usagebar (demo/sandbox.sh). Builds the plugin first.
# HERDR=/path/to/herdr selects the herdr binary under test.
set -euo pipefail
cd "$(dirname "$0")/.."

export SANDBOX=${SANDBOX:-/tmp/herdr-ai-usagebar-sandbox-e2e}
PLUGIN=agnostk.ai-usagebar
TIMEOUT_SECS=${TIMEOUT_SECS:-30}

h() { demo/sandbox.sh exec "${HERDR:-herdr}" "$@"; }

py() { # expression over the JSON on stdin, bound to `d`
  python3 -c "import json, sys; d = json.load(sys.stdin); print($1)"
}

fail() {
  echo "FAIL: $*" >&2
  echo "--- plugin command log" >&2
  h plugin log list --plugin "$PLUGIN" --limit 10 >&2 || true
  echo "--- refresher log" >&2
  cat "$SANDBOX"/state/herdr/plugins/"$PLUGIN"/sessions/*/daemon.log >&2 || true
  echo "--- herdr server log" >&2
  cat "$SANDBOX/server.log" >&2 || true
  exit 1
}

workspace_id() { # label
  h workspace list | py "next(w['workspace_id'] for w in d['result']['workspaces'] if w['label'] == '$1')"
}

ai_usage() { # workspace_label agent|-
  local ws
  ws=$(workspace_id "$1")
  if [ "$2" = - ]; then
    h workspace get "$ws" | py "(d['result']['workspace'].get('tokens') or {}).get('ai_usage', '')"
  else
    local pane
    pane=$(h agent list | py "next(a['pane_id'] for a in d['result']['agents'] if a['workspace_id'] == '$ws' and a['agent'] == '$2')")
    h pane get "$pane" | py "(d['result']['pane'].get('tokens') or {}).get('ai_usage', '')"
  fi
}

expect() { # workspace_label agent|- expected ("" = no token)
  local deadline=$((SECONDS + TIMEOUT_SECS)) got
  while :; do
    got=$(ai_usage "$1" "$2")
    if [ "$got" = "$3" ]; then
      echo "ok: $1/$2 ai_usage='$3'"
      return 0
    fi
    [ "$SECONDS" -lt "$deadline" ] || fail "$1/$2: expected ai_usage='$3', got '$got'"
    sleep 0.5
  done
}

action() { # id: invoke and wait for this invocation to succeed
  local log_id status deadline=$((SECONDS + TIMEOUT_SECS))
  log_id=$(h plugin action invoke "$PLUGIN.$1" | py "d['result']['log']['log_id']")
  while :; do
    status=$(h plugin log list --plugin "$PLUGIN" --limit 50 |
      py "next((l['status'] for l in d['result']['logs'] if l['log_id'] == '$log_id'), '')")
    case "$status" in
      succeeded) echo "ok: action $1"; return 0 ;;
      failed) fail "action $1 failed" ;;
    esac
    [ "$SECONDS" -lt "$deadline" ] || fail "action $1 did not finish"
    sleep 0.5
  done
}

cargo build --release --locked
trap 'demo/sandbox.sh down' EXIT
demo/sandbox.sh up || fail "the sandbox herdr did not come up"
h --version

warnings=$(h plugin list --plugin "$PLUGIN" --json | py "d['result']['plugins'][0].get('warnings') or ''")
[ -z "$warnings" ] || fail "herdr reported manifest warnings: $warnings"
echo "ok: manifest linked without warnings"

# The startup hook started the refresher; the first fake fetch is stage 1.
expect api claude "◑ 5h 38% · 7d 54%"
expect api codex "◕ 5h 64% · 7d 81%"
expect docs copilot "◔ premium 18%"
expect api - "◕ cld 54% · gpt 81%"
expect docs - "◔ ghc 18%"

action setup-sidebar
grep -qF "token = \"\$ai_usage\"" "$SANDBOX/config/herdr/config.toml" ||
  fail "setup-sidebar did not add the usage row"
h config check >/dev/null || fail "herdr rejects the config setup-sidebar wrote"
echo "ok: sidebar layout installed and accepted by herdr"

# The next fake fetch is stage 2.
action refresh
expect api codex "● 5h 93% · 7d 86%"
expect api - "● cld 71% · gpt 93%"

action stop
expect api claude ""
expect api - ""
echo "e2e passed"
