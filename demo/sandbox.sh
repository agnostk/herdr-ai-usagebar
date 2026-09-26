#!/usr/bin/env bash
# An isolated herdr for the demo recording and the integration check: its own
# XDG config/state/data/cache roots, so its socket, plugin registry and config
# never touch the herdr session you are working in.
#
#   demo/sandbox.sh up            start a server with the plugin and demo agents
#   demo/sandbox.sh exec CMD...   run CMD inside the sandbox (e.g. `herdr`)
#   demo/sandbox.sh stage N       make the next fake ai-usagebar call print snapshot N
#   demo/sandbox.sh down          stop the sandbox server
set -euo pipefail

REPO=$(cd "$(dirname "$0")/.." && pwd)
# A short default: the server socket lives under it and Unix socket paths
# are limited to about 100 bytes.
SANDBOX=${SANDBOX:-/tmp/herdr-ai-usagebar-sandbox}
SANDBOX=${SANDBOX%/}
HERDR=${HERDR:-herdr}
AGENT_SOURCE=demo

enter() {
  unset HERDR_ENV HERDR_PANE_ID HERDR_TAB_ID HERDR_WORKSPACE_ID \
    HERDR_SOCKET_PATH HERDR_BIN_PATH HERDR_SESSION HERDR_CONFIG_PATH
  export XDG_CONFIG_HOME="$SANDBOX/config" XDG_STATE_HOME="$SANDBOX/state"
  export XDG_DATA_HOME="$SANDBOX/data" XDG_CACHE_HOME="$SANDBOX/cache"
  export DEMO_STAGE_FILE="$SANDBOX/stage"
}

json() { # extract a value from JSON on stdin: json '["result"]["workspace"]["workspace_id"]'
  python3 -c "import json, sys; print(json.load(sys.stdin)$1)"
}

wait_for_server() {
  # An API call, not `herdr status server`: the server can report itself
  # running before its API socket accepts requests.
  local deadline=$((SECONDS + ${SANDBOX_START_TIMEOUT:-30}))
  until "$HERDR" workspace list >/dev/null 2>&1; do
    if [ "$SECONDS" -ge "$deadline" ]; then
      echo "sandbox herdr server did not start; see $SANDBOX/server.log" >&2
      return 1
    fi
    sleep 0.1
  done
}

agent() { # pane agent state
  "$HERDR" pane report-agent "$1" --source "$AGENT_SOURCE" --agent "$2" --state "$3" >/dev/null
}

project() { # name -> a git checkout under the sandbox, so space rows show a branch
  local dir="$SANDBOX/projects/$1"
  mkdir -p "$dir"
  git -C "$dir" init -q -b main
  echo "$dir"
}

workspace() { # label -> root pane id
  "$HERDR" workspace create --cwd "$(project "$1")" --label "$1" --no-focus |
    json '["result"]["root_pane"]["pane_id"]'
}

split() { # pane direction ratio -> new pane id
  "$HERDR" pane split "$1" --direction "$2" --ratio "$3" |
    json '["result"]["pane"]["pane_id"]'
}

caption() { # pane text: a label in an agent pane, since no real agent runs there
  "$HERDR" pane run "$1" "clear; printf '\\033[2m%s\\033[0m\\n' '$2'" >/dev/null
}

up() {
  # Only ever delete a directory named for the sandbox, whatever SANDBOX says.
  case "$(basename "$SANDBOX")" in
    herdr-ai-usagebar-sandbox*) rm -rf "$SANDBOX" ;;
    *) echo "refusing to reset SANDBOX=$SANDBOX: its name must start with herdr-ai-usagebar-sandbox" >&2; exit 1 ;;
  esac
  mkdir -p "$XDG_CONFIG_HOME/herdr" "$XDG_STATE_HOME" "$XDG_DATA_HOME" "$XDG_CACHE_HOME"
  echo 1 > "$DEMO_STAGE_FILE"
  sed "s|@REPO@|$REPO|g" "$REPO/demo/herdr-config.toml" > "$XDG_CONFIG_HOME/herdr/config.toml"

  "$HERDR" plugin link "$REPO" >/dev/null
  local plugin_config
  plugin_config=$("$HERDR" plugin config-dir agnostk.ai-usagebar)
  cat > "$plugin_config/config.toml" <<EOF
ai_usagebar = "$REPO/demo/fake-ai-usagebar"
# Only the startup fetch and explicit refreshes, so the demo is deterministic.
refresh_secs = 3600
scan_secs = 1
EOF

  nohup "$HERDR" server > "$SANDBOX/server.log" 2>&1 &
  wait_for_server

  local api claude codex web docs
  api=$(workspace api)
  claude=$(split "$api" down 0.45)
  codex=$(split "$claude" right 0.5)
  agent "$claude" claude working
  agent "$codex" codex working
  caption "$claude" "claude (simulated agent)"
  caption "$codex" "codex (simulated agent)"
  web=$(workspace web)
  agent "$web" claude idle
  caption "$web" "claude (simulated agent)"
  docs=$(workspace docs)
  agent "$docs" copilot idle
  caption "$docs" "copilot (simulated agent)"

  # Close the server's initial empty workspace so the demo starts on `api`.
  local initial
  initial=$("$HERDR" workspace list | json '["result"]["workspaces"][0]["workspace_id"]')
  if [ "$("$HERDR" workspace get "$initial" | json '["result"]["workspace"]["label"]')" != api ]; then
    "$HERDR" workspace close "$initial" >/dev/null || true
  fi
  "$HERDR" workspace focus "$("$HERDR" workspace list | json '["result"]["workspaces"][0]["workspace_id"]')" >/dev/null
  "$HERDR" pane focus "$api" >/dev/null 2>&1 || true
}

down() {
  "$HERDR" plugin action invoke agnostk.ai-usagebar.stop >/dev/null 2>&1 || true
  "$HERDR" server stop >/dev/null 2>&1 || true
}

enter
case "${1:-}" in
  up) up ;;
  down) down ;;
  stage) echo "${2:?stage number}" > "$DEMO_STAGE_FILE" ;;
  exec) shift; exec "$@" ;;
  *) sed -n '2,10p' "$0" >&2; exit 2 ;;
esac
