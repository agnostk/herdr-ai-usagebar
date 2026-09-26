# Security policy

herdr-ai-usagebar runs as you, starts a background process, executes other
programs, and can edit your herdr `config.toml`. This page lists exactly what
it does, so you can judge it before installing.

## Reporting a vulnerability

Report privately through GitHub: open the repository's
[Security tab](https://github.com/agnostk/herdr-ai-usagebar/security) and
choose **Report a vulnerability**. Please do not open a public issue for
anything exploitable. A description, the affected commit or version, and
steps to reproduce are enough to start. Expect a first response within seven
days.

Before you paste logs or config anywhere, remove credentials. The plugin never
handles provider credentials itself, but ai-usagebar error messages or your
own config may mention them.

## Supported versions

Only the latest commit on `main` (the version `herdr plugin install` fetches)
receives security fixes.

## What the plugin executes

Every program runs with an argv array, never through a shell, so values from
config files or other programs' output cannot become shell syntax.

| Program | When | Found at |
|---------|------|----------|
| `herdr` | listing agents and workspaces, reporting sidebar tokens, reloading config, opening the dashboard pane | `HERDR_BIN_PATH`, which herdr sets for plugin commands |
| `ai-usagebar usage --json` | every `refresh_secs` (default 60 s) | `ai_usagebar` in the plugin config, else `PATH`, then `~/.cargo/bin`, `~/.local/bin`, `/opt/homebrew/bin`, `/usr/local/bin`, `/usr/bin` and the Nix profile directories |
| `ai-usagebar-tui` | only when you open the dashboard pane | next to the configured `ai-usagebar`, else the same search |
| the plugin binary itself | the startup hook and event hooks start it as a detached background process | its own path |

Each `ai-usagebar` run has a deadline (`fetch_timeout_secs`, default 90 s).
When it expires, the plugin kills the run's whole process group, so a stuck
child cannot pile up.

The fallback search directories are ordinary user or package-manager
locations. Anyone who can write to them can already run code as you, so the
search adds no new exposure. Set `ai_usagebar` to an absolute path to skip the
search.

## What it reads

- `config.toml` in the plugin config directory (`herdr plugin config-dir agnostk.ai-usagebar`).
- The JSON printed by `ai-usagebar usage --json`: plan names, usage
  percentages, reset times, and error messages. The plugin never reads
  provider credentials, keychains, or ai-usagebar's own config. ai-usagebar
  handles all of that.
- Agent and workspace lists from `herdr agent list` and `herdr workspace list`.
- Your herdr `config.toml`, only when you run the `setup-sidebar` action.

## What it writes

| Path | Contents | Unix mode |
|------|----------|-----------|
| `<state dir>/sessions/<id>/daemon.log` | the refresher's log; moved to `daemon.log.1` when a refresher starts and finds it past 1 MiB | `0600` |
| `<state dir>/sessions/<id>/status.json` | pid, version, last refresh time, last error, entry ids | `0600` |
| `<state dir>/sessions/<id>/daemon.lock`, `stop`, `refresh` | empty coordination files | umask |
| `<config dir>/config.toml` | a commented example, written only if absent | umask |
| herdr `config.toml` | the `$ai_usage` sidebar rows, only through `setup-sidebar` | unchanged |
| `config.toml.bak-ai-usagebar-<time>` beside it | the previous herdr config | same as the original |

`<state dir>` is herdr's plugin state directory (`HERDR_PLUGIN_STATE_DIR`,
usually `~/.local/state/herdr/plugins/agnostk.ai-usagebar`). `<id>` is a hash
of the herdr server socket path, one per herdr session.

`setup-sidebar` edits the herdr config conservatively:

1. It edits with `toml_edit`, which keeps your formatting and comments.
2. It writes the result to a randomly named temporary file created exclusively
   (`O_EXCL`) in the same directory, with your file's permissions.
3. It runs `herdr config check` on that temporary file and changes nothing if
   herdr rejects it.
4. It writes the backup with `create_new`, so it never writes through an
   existing file or symlink.
5. It atomically renames the new file into place. If `config.toml` is a
   symlink, as with dotfile managers, it edits the file the link points to and
   leaves the link intact.

## What leaves your machine

Nothing, from this plugin: it opens no network connections. ai-usagebar
contacts your AI providers' usage endpoints with its own credentials, exactly
as it does in a status bar. The sidebar values the plugin reports stay inside
your herdr server's memory, and herdr does not persist them across restarts.

## Redaction

Error text from ai-usagebar is shown in the sidebar and written to the log and
`status.json`. Before that, the plugin masks anything that looks like a
credential: bearer tokens, `api_key=…`/`token=…`-style pairs, well-known key
prefixes (`sk-`, `ghp_`, `github_pat_`, …), and long opaque strings. Sidebar
rows also show only the error's leading category (`⚠ credentials error`), not
the full message.

## The background refresher

- One per herdr session, enforced with an exclusive file lock that the OS
  releases if the process dies.
- It stops on the `stop` action, within a second or as soon as an in-flight
  ai-usagebar run finishes. It also stops when you disable or uninstall the
  plugin (checked on each refresh), or after herdr has been unreachable for
  60 seconds.
- Every value it reports expires on its own after
  `3 × (refresh_secs + fetch_timeout_secs + scan_secs)` seconds (about 8
  minutes with defaults), so nothing stale lingers if it is killed.

## Build and supply chain

- `herdr plugin install` builds with `cargo build --release --locked`, so only
  the dependency versions in the committed `Cargo.lock` are used. The plugin
  crate has no `build.rs` and no `rust-toolchain.toml` (which would make rustup
  download a toolchain on install). The only procedural macro in the tree is
  `serde_derive`; the build scripts that run are those of `anyhow`, `serde`,
  `serde_json`, `libc`, `rustix`, `getrandom`, `proc-macro2`, `quote` and
  `zmij`.
- CI runs [`cargo deny`](deny.toml) for RustSec advisories, licenses and
  crate sources on every push and weekly. Dependabot proposes dependency and
  GitHub Actions updates.
- GitHub Actions are pinned to commit SHAs. The CI token is read-only except
  for the job that publishes the coverage badge to the `badges` branch.
- CI downloads herdr release binaries and verifies them against the SHA-256
  digests GitHub records for each release asset.

## Switching things off

| To stop | Do |
|---------|----|
| the refresher, now | `herdr plugin action invoke agnostk.ai-usagebar.stop` |
| everything, until re-enabled | `herdr plugin disable agnostk.ai-usagebar` |
| agent or space rows | `agent_rows = false` or `workspace_rows = "off"` in the plugin config |
| the plugin entirely | `herdr plugin uninstall agnostk.ai-usagebar`, then remove the `$ai_usage` rows or restore the backup |
