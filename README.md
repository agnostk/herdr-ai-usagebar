# herdr-ai-usagebar

A [herdr](https://herdr.dev) plugin that puts
[ai-usagebar](https://github.com/akitaonrails/ai-usagebar) plan usage in the
herdr sidebar. Each agent row shows the quota of the provider that agent runs
on, and each space row sums up the providers its agents use.

```
spaces                        agents
● my-api                      ● my-api · 1
  main                          claude
  ◔ cld 14% · gpt 0%            ◔ 5h 3% · 7d 14%
                              ● my-api · 2
                                codex
                                ○ 5h 0% · 7d 0%
```

The pie fills with the highest usage across the provider's windows (`○` under
13%, then `◔`, `◑`, `◕`, and `●` from 88%). The sidebar rows are colored
the same way, green through red.

## Requirements

- herdr 0.9.0 or newer, on Linux or macOS
- [ai-usagebar](https://github.com/akitaonrails/ai-usagebar#install) installed
  and configured, so that `ai-usagebar usage --json` lists your providers
- A Rust toolchain (`cargo`); herdr builds the plugin on install

## Install

```sh
herdr plugin install agnostk/herdr-ai-usagebar
herdr plugin action invoke agnostk.ai-usagebar.setup-sidebar
```

`setup-sidebar` adds a `$ai_usage` row to the agent and space layouts in your
herdr `config.toml`. It validates the result with `herdr config check`, keeps a
`config.toml.bak-ai-usagebar-<time>` backup, and reloads herdr. It does not
change your file if you already use `$ai_usage` somewhere. If you would rather
edit the layout yourself, see [Sidebar layout](#sidebar-layout).

The refresher starts with herdr, and on first install as soon as an agent is
detected or you switch workspaces. Run the `refresh` action to start it
immediately.

## What it shows

The plugin reports two tokens, for agent rows (pane metadata) and space rows
(workspace metadata):

| Token           | Example              | Meaning                                         |
| --------------- | -------------------- | ----------------------------------------------- |
| `$ai_usage`     | `◔ 5h 3% · 7d 14%`   | Usage windows, or `⚠ <error>` if a provider fails |
| `$ai_usage_pct` | `14`                 | Highest usage percentage, for numeric style rules |

Agent rows show the first `max_windows` windows of the provider mapped to that
agent. For Claude these are the 5-hour session and the 7-day week. Space rows
show one `short-name percent` pair per provider. `(stale)` means the latest
ai-usagebar run failed and the previous numbers are still shown. The tokens
expire on their own if the plugin stops, so old numbers never linger.

Agents map to ai-usagebar providers like this by default:

| herdr agent | ai-usagebar entry          |
| ----------- | -------------------------- |
| `claude`    | `anthropic`, `anthropic_api` |
| `codex`     | `openai`                   |
| `copilot`   | `copilot`                  |
| `cursor`    | `cursor`                   |
| `kimi`      | `kimi`, `moonshot`         |
| `kilo`      | `kilo`                     |
| `kiro`      | `kiro`                     |
| `grok`      | `grok`, `supergrok`        |
| `agy`       | `antigravity`              |
| `opencode`  | `opencode-go`              |

An entry also matches its named accounts, so `anthropic` covers
`anthropic@work`. Change the mapping in the plugin config.

## Actions

| Action                                | What it does                                         |
| ------------------------------------- | ---------------------------------------------------- |
| `agnostk.ai-usagebar.setup-sidebar`   | Add the `$ai_usage` rows to the sidebar layout       |
| `agnostk.ai-usagebar.refresh`         | Re-run ai-usagebar now, starting the refresher if needed |
| `agnostk.ai-usagebar.dashboard`       | Open `ai-usagebar-tui` in a popup                    |
| `agnostk.ai-usagebar.status`          | Print refresher state to the plugin log              |
| `agnostk.ai-usagebar.stop`            | Stop the refresher and clear the tokens              |

Run one with `herdr plugin action invoke <action>`, or bind a key in your
herdr `config.toml`:

```toml
[[keys.command]]
key = "prefix+u"
type = "plugin_action"
command = "agnostk.ai-usagebar.dashboard"
description = "AI usage dashboard"
```

Output from `status` and the other actions appears in
`herdr plugin log list --plugin agnostk.ai-usagebar`.

## Configuration

Settings live in `config.toml` inside the plugin config directory:

```sh
$EDITOR "$(herdr plugin config-dir agnostk.ai-usagebar)/config.toml"
```

The plugin writes a commented copy of [config.example.toml](config.example.toml)
there on first run. Changes apply on the next refresh.

| Key                  | Default    | Meaning                                                  |
| -------------------- | ---------- | -------------------------------------------------------- |
| `ai_usagebar`        | search     | Path to the `ai-usagebar` binary                         |
| `refresh_secs`       | `60`       | Seconds between `ai-usagebar usage --json` runs          |
| `scan_secs`          | `5`        | Seconds between checks for new or closed agents          |
| `fetch_timeout_secs` | `90`       | Timeout for one ai-usagebar run                          |
| `max_windows`        | `2`        | Windows shown per agent row                              |
| `agent_rows`         | `true`     | Report `$ai_usage` on agent rows                         |
| `workspace_rows`     | `"agents"` | `"agents"`, `"all"` (every provider on every space) or `"off"` |
| `[agents]`           | see above  | herdr agent id to ai-usagebar entry ids, tried in order  |

Without `ai_usagebar`, the plugin searches `PATH`, then `~/.cargo/bin`,
`~/.local/bin`, `/opt/homebrew/bin`, `/usr/local/bin` and the Nix profile
directories.

## Sidebar layout

`setup-sidebar` appends this row to `[ui.sidebar.agents]` and
`[ui.sidebar.spaces]` (starting from herdr's default rows when you have none),
and to every `rows_by_agent` override:

```toml
[ui.sidebar.agents]
rows = [
  ["state_icon", "machine", "workspace", "tab"],
  ["agent"],
  [{ token = "$ai_usage", fg = "#a6e3a1", rules = [{ starts_with = "●", fg = "#f38ba8", bold = true }, { starts_with = "◕", fg = "#fab387" }, { starts_with = "◑", fg = "#f9e2af" }, { starts_with = "⚠", fg = "#f38ba8" }] }],
]
```

Put `$ai_usage` anywhere you like instead. For example, `["agent", "$ai_usage"]`
keeps it on the agent-name line. Style rules can only match a token's own
value, which is why the colors key off the leading pie glyph. Use
`$ai_usage_pct` with `gt`/`lt` rules if you prefer numeric thresholds.

## How it works

herdr v1 plugins have no timers or long-lived processes, so the `[[startup]]`
hook (and a few event hooks) start a small background refresher and exit. The
refresher:

- runs once per herdr session, guarded by a file lock in the plugin state
  directory;
- runs `ai-usagebar usage --json` every `refresh_secs`, relying on
  ai-usagebar's own cache and rate-limit backoff;
- reads `herdr agent list` / `herdr workspace list` every `scan_secs` and
  reports tokens with `herdr pane|workspace report-metadata`, only when a value
  changes or its TTL needs renewing;
- sets a TTL on every token, so values disappear if the refresher dies;
- exits when the plugin is disabled or uninstalled, when the herdr server has
  been unreachable for a minute, or on the `stop` action;
- restarts itself when its binary changes (rebuild or reinstall).

Its log is in the plugin state directory, under `sessions/<id>/daemon.log`.
The `status` action prints the exact path. Set `HERDR_AI_USAGEBAR_DEBUG=1` in
herdr's environment for verbose logging.

## Uninstall

```sh
herdr plugin action invoke agnostk.ai-usagebar.stop
herdr plugin uninstall agnostk.ai-usagebar
```

Unreported tokens render as nothing, so the `$ai_usage` rows can stay in your
config. Remove them, or restore the `config.toml.bak-ai-usagebar-*` backup, to
tidy up.

## Development

```sh
cargo test
cargo build --release
herdr plugin link "$PWD"
herdr plugin action invoke agnostk.ai-usagebar.refresh
```

`herdr plugin link` does not run build commands, so build before linking. A
running refresher picks up new builds on its next refresh.

## License

MIT. [ai-usagebar](https://github.com/akitaonrails/ai-usagebar) is by
AkitaOnRails and contributors; this plugin only runs it.
