<!-- Title: a Conventional Commit subject, e.g. "fix: keep stale rows visible" -->

## Summary

<!-- What changes and why. Link issues with "closes #123". -->

## User-visible changes

<!-- Sidebar text, token names, config keys or defaults, action ids. Say why each changed, or write "none". -->

## Checklist

- [ ] `cargo fmt --all -- --check`, `cargo clippy --locked --all-targets -- -D warnings` and `cargo test --locked --all-targets` pass
- [ ] `scripts/e2e.sh` passes (or explain why it does not apply)
- [ ] Tests cover the change, including failure paths
- [ ] `CHANGELOG.md` updated under *Unreleased* for user-visible changes
- [ ] Pasted output is redacted

## AI involvement

- [ ] no AI
- [ ] AI-assisted, I reviewed everything
- [ ] an agent wrote this; what was verified and how is described above
