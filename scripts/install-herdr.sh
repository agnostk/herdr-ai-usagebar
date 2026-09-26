#!/usr/bin/env bash
# Download a herdr release binary for this machine into DEST and verify it
# against the SHA-256 digest GitHub records for the release asset.
#
#   scripts/install-herdr.sh v0.9.0 ~/.local/bin/herdr
#   scripts/install-herdr.sh latest ~/.local/bin/herdr
#
# Needs the GitHub CLI (`gh`), authenticated or with GH_TOKEN set.
set -euo pipefail

VERSION=${1:?version tag or "latest"}
DEST=${2:?destination path}
REPO=ogulcancelik/herdr

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) ASSET=herdr-linux-x86_64 ;;
  Linux-aarch64 | Linux-arm64) ASSET=herdr-linux-aarch64 ;;
  Darwin-arm64) ASSET=herdr-macos-aarch64 ;;
  Darwin-x86_64) ASSET=herdr-macos-x86_64 ;;
  *) echo "no herdr release asset for $(uname -s)-$(uname -m)" >&2; exit 1 ;;
esac

if [ "$VERSION" = latest ]; then
  VERSION=$(gh release view --repo "$REPO" --json tagName --jq .tagName)
fi

expected=$(gh release view "$VERSION" --repo "$REPO" --json assets \
  --jq ".assets[] | select(.name == \"$ASSET\") | .digest" | sed 's/^sha256://')
[ -n "$expected" ] || { echo "$VERSION has no digest for $ASSET" >&2; exit 1; }

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
gh release download "$VERSION" --repo "$REPO" --pattern "$ASSET" --dir "$tmp"
if command -v sha256sum >/dev/null; then
  actual=$(sha256sum "$tmp/$ASSET" | cut -d' ' -f1)
else
  actual=$(shasum -a 256 "$tmp/$ASSET" | cut -d' ' -f1)
fi
[ "$actual" = "$expected" ] || {
  echo "checksum mismatch for $ASSET $VERSION: $actual != $expected" >&2
  exit 1
}

mkdir -p "$(dirname "$DEST")"
install -m 0755 "$tmp/$ASSET" "$DEST"
echo "installed herdr $VERSION ($ASSET) to $DEST"
