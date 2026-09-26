# Configuration

There are two places to configure:

- **The plugin's `config.toml`** decides what the plugin reports: which
  providers, how often, and how many windows.
- **Your herdr `config.toml`** decides where and how the sidebar shows it.
  `setup-sidebar` writes a sensible default there, and you can change it
  freely.

## Plugin settings

The file is `config.toml` in the plugin config directory:

```sh
$EDITOR "$(herdr plugin config-dir agnostk.ai-usagebar)/config.toml"
```

The first run writes a fully commented copy of
[config.example.toml](../config.example.toml) there. Every key is optional.
The refresher rereads the file on each refresh, so run the `refresh` action to
apply changes immediately. A file with a typo or an unknown key is rejected as
a whole: the refresher logs the error and keeps its previous settings.

| Key | Default | Range | Meaning |
|-----|---------|-------|---------|
| `ai_usagebar` | searched | path | The `ai-usagebar` binary. `ai-usagebar-tui` is expected next to it. |
| `refresh_secs` | `60` | 15–3600 | Seconds between `ai-usagebar usage --json` runs. ai-usagebar caches each provider for 60 s, so going lower only rereads its cache. |
| `scan_secs` | `5` | 1–300 | Seconds between checks for new, moved or closed agents and workspaces. New agents get their row within this time. |
| `fetch_timeout_secs` | `90` | 5–600 | Deadline for one ai-usagebar run; the run and its children are killed after it. |
| `max_windows` | `2` | 1–6 | Usage windows per agent row, in ai-usagebar's order. |
| `agent_rows` | `true` | bool | Report `$ai_usage` on agent rows. |
| `workspace_rows` | `"agents"` | `"agents"`, `"all"`, `"off"` | What space rows get; see below. |
| `[agents]` | see below | table | herdr agent id to ai-usagebar entry ids. |

### Space rows

- `"agents"`: each workspace lists the providers of the agents running in it
  (`◑ cld 54% · gpt 81%`). Workspaces without a tracked agent show nothing.
- `"all"`: every workspace lists every provider ai-usagebar reports, whether
  or not an agent is running.
- `"off"`: space rows get nothing.

### Agent mapping

Keys are herdr's canonical agent ids, as `herdr agent list` shows them.
Values are ai-usagebar entry ids, as `ai-usagebar usage --json` shows them in
`entries[].id`. The plugin tries the ids in order and uses the first one that
reports usage. If none does, it shows the first one's error, so a
misconfigured provider is visible rather than silently missing.

An id also matches ai-usagebar's named accounts: `anthropic` matches
`anthropic@work`. Name the account explicitly to pick one.

| herdr agent | Default entries |
|-------------|-----------------|
| `claude` | `anthropic`, `anthropic_api` |
| `codex` | `openai` |
| `copilot` | `copilot` |
| `cursor` | `cursor` |
| `kimi` | `kimi`, `moonshot` |
| `kilo` | `kilo` |
| `kiro` | `kiro` |
| `grok` | `grok`, `supergrok` |
| `agy` | `antigravity` |
| `opencode` | `opencode-go` |

Entries in your `[agents]` table replace the default for that agent only:

```toml
[agents]
claude = ["anthropic@work"]   # this machine uses the work account
amp = ["openrouter"]          # an agent with no default
codex = []                    # never show usage for codex
```

## Sidebar layout

herdr shows a custom token only where your layout names it. `setup-sidebar`
appends this row to `[ui.sidebar.agents]` and `[ui.sidebar.spaces]`. It starts
from herdr's default rows if you have none, and also adds the row to every
`rows_by_agent` override:

```toml
[{ token = "$ai_usage", fg = "#a6e3a1", rules = [
  { starts_with = "●", fg = "#f38ba8", bold = true },
  { starts_with = "◕", fg = "#fab387" },
  { starts_with = "◑", fg = "#f9e2af" },
  { starts_with = "⚠", fg = "#f38ba8" },
] }]
```

`setup-sidebar` writes it on a single line; both forms are valid.

herdr style rules can only match a token's own value, so the colors key off
the leading pie glyph. Some variations:

**Usage on the agent-name line** instead of a row of its own:

```toml
[ui.sidebar.agents]
rows = [
  ["state_icon", "machine", "workspace", "tab"],
  ["agent", "$ai_usage"],
]
```

**Numeric thresholds** with `$ai_usage_pct`, the highest percentage as a bare
number. herdr's numeric rules need a pure number, which is why it is a
separate token:

```toml
[ui.sidebar.agents]
rows = [
  ["state_icon", "agent", { token = "$ai_usage_pct", rules = [
    { gt = 89, fg = "#f38ba8", bold = true },
    { gt = 49, fg = "#f9e2af" },
  ] }],
  ["workspace", "tab"],
]
```

**Warnings only**: hide the row until something is close to a limit. `hide`
rules need herdr 0.9.1 or newer:

```toml
{ token = "$ai_usage", rules = [
  { starts_with = "●", fg = "#f38ba8", bold = true },
  { starts_with = "◕", fg = "#fab387" },
  { starts_with = "○", hide = true },
  { starts_with = "◔", hide = true },
  { starts_with = "◑", hide = true },
] }
```

Run `herdr server reload-config` (or herdr's *reload config* menu item) after
editing. See herdr's
[sidebar row layouts](https://herdr.dev/docs/configuration/#sidebar-row-layouts)
for every token and rule.

## The token values

| Token | Example | Content |
|-------|---------|---------|
| `$ai_usage` on agent rows | `◑ 5h 38% · 7d 54%` | pie, then `max_windows` windows |
| `$ai_usage` on space rows | `◕ cld 54% · gpt 81%` | pie, then each provider's highest window |
| `$ai_usage_pct` | `81` | highest percentage across all windows |

- **Pie:** fills with the highest usage across *all* of a provider's windows,
  including ones not displayed. It shows `○` under 13%, `◔` to 37%, `◑` to
  62%, `◕` to 87%, and `●` above that. A per-model weekly limit near
  exhaustion therefore still turns the row red.
- **Window labels:** come from the window length (`5h`, `7d`, `30d`), or from
  the label for calendar windows (`mo`, `wk`, `day`, or its first word).
  Balances show their value (`$4.20`), and unlimited quotas show `∞`.
- **`⚠ <category>`:** ai-usagebar could not read that provider, for example
  `⚠ credentials error` or `⚠ rate limited`. The `status` action shows the
  full error.
- **`(stale)`:** the last ai-usagebar run failed, so the previous numbers are
  still shown.
- **`⚠ ai-usagebar not found`** / **`⚠ usage unavailable`:** the plugin could
  not run ai-usagebar at all. See
  [troubleshooting](troubleshooting.md#ai-usagebar-not-found).

## Keybindings

herdr binds keys to plugin actions in its own `config.toml`. `prefix+u` is
free by default:

```toml
[[keys.command]]
key = "prefix+u"
type = "plugin_action"
command = "agnostk.ai-usagebar.dashboard"
description = "AI usage dashboard"

[[keys.command]]
key = "prefix+shift+u"
type = "plugin_action"
command = "agnostk.ai-usagebar.refresh"
description = "refresh AI usage"
```

## Environment

| Variable | Effect |
|----------|--------|
| `HERDR_AI_USAGEBAR_DEBUG=1` | verbose refresher log; set it in the environment you start herdr from |
