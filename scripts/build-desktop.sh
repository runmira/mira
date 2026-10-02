#!/usr/bin/env bash
# Build the Mira desktop app (apps/desktop).
#
#   scripts/build-desktop.sh           # release bundle (.app/.dmg, .msi, .deb/.AppImage),
#                                      # with Mira's Chromium inside (see fetch-chromium.sh)
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
# `${CARGO_FLAGS[@]+"${CARGO_FLAGS[@]}"}` rather than a bare
# `"${CARGO_FLAGS[@]}"`: dev mode leaves the array empty, and bash 3.2 (what
# macOS ships) treats expanding an empty array under `set -u` as a fatal
# "unbound variable" error. The `[@]+` guard makes the expansion conditional
# on the array being set, which is a no-op on bash 4.4+ and rescues 3.2.
(cd "$ROOT" && cargo build ${CARGO_FLAGS[@]+"${CARGO_FLAGS[@]}"} -p mira-cli)

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
    # Ship Mira's browser in the bundle so it works on first run, offline.
    # macOS goes through `macOS.files`, which copies directories with their
    # symlinks intact; `resources` would flatten them and break Chrome's
    # framework. MIRA_NO_BUNDLED_CHROMIUM=1 skips it (~350 MB smaller app;
    # the browser then downloads on first use).
    TAURI_ARGS=()
    if [ -z "${MIRA_NO_BUNDLED_CHROMIUM:-}" ]; then
      "$ROOT/scripts/fetch-chromium.sh" "$DESKTOP/chromium" "$TRIPLE"
      if [ -f "$DESKTOP/chromium/current" ]; then
        case "$TRIPLE" in
          *apple-darwin) BUNDLE='{"bundle":{"macOS":{"files":{"Resources/chromium":"chromium"}}}}' ;;
          *) BUNDLE='{"bundle":{"resources":{"chromium/":"chromium/"}}}' ;;
        esac
        TAURI_ARGS=(--config "$BUNDLE")
      fi
    fi
    cargo tauri build ${TAURI_ARGS[@]+"${TAURI_ARGS[@]}"}
    echo
    echo "bundles: $DESKTOP/target/release/bundle/"
    ;;
esac
