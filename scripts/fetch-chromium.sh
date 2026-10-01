#!/usr/bin/env bash
# Stage Chrome for Testing for the desktop release to ship.
#
#   scripts/fetch-chromium.sh <dest-dir> [<rust-target-triple>]
#
# Writes <dest>/<version>/chrome-<platform>/… and <dest>/current — the same
# layout Mira's own download uses (crates/mira-browser/src/managed.rs), so
# the app's server can run it from the bundle with no first-run download.
# Reuses an existing stage of the same version. Exits 0 without staging on
# a platform Chrome for Testing doesn't build for (the app then downloads
# nothing and uses an installed browser, as before).
set -euo pipefail

DEST="${1:?usage: $0 <dest-dir> [target-triple]}"
TRIPLE="${2:-$(rustc -vV | sed -n 's/^host: //p')}"
MANIFEST="https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions-with-downloads.json"

case "$TRIPLE" in
  aarch64-apple-darwin) PLATFORM=mac-arm64 ;;
  x86_64-apple-darwin) PLATFORM=mac-x64 ;;
  x86_64-unknown-linux-gnu) PLATFORM=linux64 ;;
  x86_64-pc-windows-msvc) PLATFORM=win64 ;;
  i686-pc-windows-msvc) PLATFORM=win32 ;;
  *)
    echo "chromium: no Chrome for Testing build for $TRIPLE; not bundling one"
    exit 0
    ;;
esac

read -r VERSION URL < <(curl -fsSL "$MANIFEST" | python3 -c '
import json, sys
stable = json.load(sys.stdin)["channels"]["Stable"]
url = next(d["url"] for d in stable["downloads"]["chrome"] if d["platform"] == sys.argv[1])
print(stable["version"], url)
' "$PLATFORM")

if [ "$(cat "$DEST/current" 2>/dev/null || true)" = "$VERSION" ] && [ -d "$DEST/$VERSION/chrome-$PLATFORM" ]; then
  echo "chromium: $VERSION already staged"
  exit 0
fi

echo "chromium: fetching Chrome for Testing $VERSION ($PLATFORM)"
rm -rf "$DEST"
mkdir -p "$DEST/$VERSION"
ZIP="$(mktemp -t chromium.XXXXXX).zip"
trap 'rm -f "$ZIP"' EXIT
curl -fL --progress-bar -o "$ZIP" "$URL"
if [ "$(uname)" = Darwin ]; then
  # ditto keeps the app bundle's symlinks and attributes; unzip doesn't.
  ditto -x -k "$ZIP" "$DEST/$VERSION"
else
  unzip -q "$ZIP" -d "$DEST/$VERSION"
fi
echo "$VERSION" > "$DEST/current"
echo "chromium: staged $DEST ($(du -sh "$DEST" | cut -f1))"
