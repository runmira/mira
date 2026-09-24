#!/usr/bin/env bash
# Build the Mira desktop app (apps/desktop).
#
#   scripts/build-desktop.sh           # release bundle (.app/.dmg, .msi, .deb/.AppImage)
#   scripts/build-desktop.sh --dev     # run the app from source with a debug server
#   scripts/build-desktop.sh --sidecar # only build + stage the bundled `mira`
#
# Steps: build the web UI (it's embedded into `mira`), build `mira`, copy
# it to apps/desktop/binaries/mira-<target-triple> where Tauri's bundler
# expects the sidecar, then build or run the app.
#
# Needs: node/npm, the Tauri CLI (`cargo install tauri-cli --version "^2"`),
# and on Linux the WebKitGTK dev packages (see docs/desktop.md).
# The web UI needs VITE_SUPABASE_URL and VITE_SUPABASE_PUBLISHABLE_KEY,
# from the environment or crates/mira-server/frontend/.env.
set -euo pipefail

MODE=release
case "${1:-}" in
  --dev) MODE=dev ;;
  --sidecar) MODE=sidecar ;;
  "") ;;
  *) echo "usage: $0 [--dev|--sidecar]" >&2; exit 2 ;;
esac

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
FRONTEND="$ROOT/crates/mira-server/frontend"
DESKTOP="$ROOT/apps/desktop"

echo "==> web UI"
(
  cd "$FRONTEND"
  [ -d node_modules ] || npm ci
  npm run build
)

PROFILE=release
CARGO_FLAGS=(--release)
if [ "$MODE" = dev ]; then
  PROFILE=debug
  CARGO_FLAGS=()
fi

echo "==> mira ($PROFILE)"
(cd "$ROOT" && cargo build "${CARGO_FLAGS[@]}" -p mira-cli)

TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
EXT=""
case "$TRIPLE" in *windows*) EXT=".exe" ;; esac
mkdir -p "$DESKTOP/binaries"
cp "$ROOT/target/$PROFILE/mira$EXT" "$DESKTOP/binaries/mira-$TRIPLE$EXT"
echo "staged $DESKTOP/binaries/mira-$TRIPLE$EXT"

cd "$DESKTOP"
case "$MODE" in
  sidecar) ;;
  dev) cargo tauri dev ;;
  release)
    cargo tauri build
    echo
    echo "bundles: $DESKTOP/target/release/bundle/"
    ;;
esac
