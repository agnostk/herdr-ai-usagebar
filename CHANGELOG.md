# Changelog

All notable changes to this plugin are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Documentation: configuration reference with sidebar layout recipes, how it
  works, troubleshooting, security policy, contributing guide, code of
  conduct, and issue and pull request templates.
- A demo GIF recorded from a reproducible `vhs` tape in an isolated herdr
  sandbox (`demo/`).
- CI on Linux and macOS: formatting, clippy, tests, the Rust 1.89 MSRV,
  coverage with a README badge, `cargo deny`, and an end-to-end check against
  real herdr 0.9.0 and the latest herdr release.
- Integration tests that run the plugin binary against a fake herdr, and
  validation of `herdr-plugin.toml`.

## [0.1.0] - 2026-09-26

First release.

### Added

- `$ai_usage` and `$ai_usage_pct` sidebar tokens from `ai-usagebar usage
  --json`: each agent row shows its own provider's usage windows, and each
  space row summarises the providers of its agents.
- A background refresher per herdr session, started by the startup hook and
  agent and workspace events. It only reports changes, sets TTLs so values
  never go stale, and stops when the plugin is disabled or herdr goes away.
- `setup-sidebar`, which adds the usage rows to herdr's `config.toml` after
  `herdr config check` accepts them, with a backup.
- `refresh`, `status`, `stop` and `dashboard` actions, and an
  `ai-usagebar-tui` popup pane.
- A configurable agent-to-provider mapping, window count, refresh interval,
  and space-row mode.
- Redaction of credential-like text in errors, and owner-only state files.

[Unreleased]: https://github.com/agnostk/herdr-ai-usagebar/compare/cc0ef6b...HEAD
[0.1.0]: https://github.com/agnostk/herdr-ai-usagebar/commit/cc0ef6b
