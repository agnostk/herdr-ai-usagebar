# Contributing

Issues and pull requests are welcome. Use the
[issue forms](https://github.com/agnostk/herdr-ai-usagebar/issues/new/choose)
for bugs and ideas, and open pull requests against `main`. Report security
problems privately, as described in [SECURITY.md](SECURITY.md).

## Redact before you paste

ai-usagebar talks to your AI providers with your credentials. Before you paste
a log, `status` output, `ai-usagebar usage --json` output or a config file,
remove API keys, tokens, and anything else you would not publish. Plan names
and percentages are fine.

## Setup

You need:

- Rust 1.89 or newer (the `rust-version` in `Cargo.toml`)
- [herdr](https://herdr.dev/docs/install/) 0.9.0 or newer to try the plugin
  and run the end-to-end check
- [ai-usagebar](https://github.com/akitaonrails/ai-usagebar#install), optional.
  The tests and demo use the fake in `demo/fake-ai-usagebar`.

Link your checkout into herdr instead of installing it. `herdr plugin link`
does not build, so build first:

```sh
cargo build --release
herdr plugin link "$PWD"
herdr plugin action invoke agnostk.ai-usagebar.refresh
```

A running refresher restarts itself onto each new build on its next refresh.
If you installed the plugin from GitHub before, run
`herdr plugin uninstall agnostk.ai-usagebar` first: herdr refuses to link over
an install.

## Checks

CI runs all of these; run the relevant ones before pushing.

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets        # unit + integration tests
scripts/e2e.sh                           # against a real herdr, in a sandbox
cargo llvm-cov --all-targets --summary-only   # coverage (cargo install cargo-llvm-cov)
cargo deny check                         # advisories, licenses (cargo install cargo-deny)
shellcheck demo/*.sh demo/fake-ai-usagebar demo/shell scripts/*.sh
```

- **Unit tests** live next to the code in `src/`.
- **`tests/cli.rs`** runs the compiled binary the way herdr does, against a
  fake `herdr` script. It covers the refresher's whole lifecycle.
- **`tests/manifest.rs`** validates `herdr-plugin.toml`.
- **`scripts/e2e.sh`** starts an isolated herdr server (`demo/sandbox.sh`),
  links the plugin with a fake ai-usagebar, and checks the real sidebar
  tokens, `setup-sidebar`, `refresh` and `stop`. Set `HERDR=/path/to/herdr` to
  test a specific version. `scripts/install-herdr.sh v0.9.0 <dest>` fetches
  one with checksum verification.

The sandbox never touches your own herdr session. It uses separate XDG
directories under `/tmp/herdr-ai-usagebar-sandbox*`.

## Code map

| File | Role |
|------|------|
| `src/main.rs` | subcommands herdr runs (`start`, `refresh`, `setup-sidebar`, …) |
| `src/daemon.rs` | the background refresher: lock, loop, lifecycle |
| `src/usage.rs` | model of `ai-usagebar usage --json` (pure) |
| `src/format.rs` | sidebar text: pie glyph, windows, errors (pure) |
| `src/plan.rs` | which tokens each pane and workspace gets (pure) |
| `src/publisher.rs` | diffs plans and reports only changes |
| `src/herdr.rs` | herdr CLI calls and response parsing |
| `src/ai_usagebar.rs` | finding and running ai-usagebar |
| `src/sidebar.rs` | `setup-sidebar`'s config edit |
| `src/session.rs` | per-session files: lock, markers, status |
| `src/config.rs`, `src/redact.rs`, `src/proc.rs`, `src/log.rs`, `src/context.rs` | supporting pieces |

## Design rules

- **No shell.** Run programs with argv arrays. Values from config or from
  another program's output must never become shell syntax.
- **Keep decisions pure.** Parsing (`usage`), formatting (`format`) and
  planning (`plan`) do no I/O and are tested with fixtures. Add a redacted
  fixture when ai-usagebar's output grows a new shape.
- **Treat the `usage --json` contract as tolerant.** Every field defaults and
  unknown fields are ignored; only `schema_version` is checked.
- **Never leave stale numbers.** Every reported token carries a TTL, and
  targets that leave the plan are cleared. Keep the TTL longer than one
  refresher pass (see `Config::token_ttl`).
- **Touch user config only through `setup-sidebar`,** and only after
  `herdr config check` accepts the result, with a backup, atomically.
- **Redact** anything from another program before logging or displaying it.
- **Stay inside herdr's plugin API.** Call herdr through `HERDR_BIN_PATH` and
  keep state under `HERDR_PLUGIN_STATE_DIR`.
- **Keep install-time builds lean.** herdr builds the plugin on the user's
  machine. Don't add a `rust-toolchain.toml`, and think twice before adding a
  dependency.

## Commits and pull requests

- Use [Conventional Commits](https://www.conventionalcommits.org/) subjects:
  `feat:`, `fix:`, `docs:`, `test:`, `refactor:`, `chore:`, `ci:`. Explain the
  why in the body.
- Add tests for every behavior change and cover the failure paths.
- Record user-visible changes under *Unreleased* in [CHANGELOG.md](CHANGELOG.md).
- Call out anything a user sees or relies on: sidebar text, token names,
  config keys and defaults, action ids. Say why it changed. These don't fail a
  build, so they are easy to miss in review.
- If the sidebar output changes, re-record the demo with `demo/record.sh`
  (needs [vhs](https://github.com/charmbracelet/vhs)).
- Bump `version` in both `Cargo.toml` and `herdr-plugin.toml` together;
  `tests/manifest.rs` fails if they differ.

## AI-assisted contributions

AI help is welcome. Say so in the pull request, say which parts you verified
yourself and how, and flag anything you could not check. Agents filing on
someone's behalf should write in the first person as the agent, name the tool
and model, and quote the operator's request.

## Code of conduct

This project follows the [Code of Conduct](CODE_OF_CONDUCT.md).
