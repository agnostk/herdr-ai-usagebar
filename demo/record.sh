#!/usr/bin/env bash
# Re-record docs/media/demo.gif and docs/media/sidebar.png. Needs herdr, vhs
# (https://github.com/charmbracelet/vhs) and a Rust toolchain. Everything runs
# in an isolated sandbox herdr with made-up usage numbers.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build --release --locked
demo/sandbox.sh up
trap 'demo/sandbox.sh down' EXIT
vhs demo/demo.tape
