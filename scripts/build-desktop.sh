#!/usr/bin/env bash
# Build the Mira desktop app (apps/desktop).
#
#   scripts/build-desktop.sh           # release bundle (.app/.dmg, .msi, .deb/.AppImage),
#                                      # with Mira's Chromium inside (see fetch-chromium.sh)
#   scripts/build-desktop.sh --dev     # run the app from source with a debug server
#   scripts/build-desktop.sh --sidecar # only build + stage the bundled `mira`
#
# Release options:
#   --channel stable|beta|alpha   which app to build (default: from --version's
#                                 suffix, else stable). Alpha and beta are
#                                 separate apps — "Mira Alpha", its own bundle
#                                 id, icon, mira-alpha:// scheme and port — so
#                                 they install next to stable.
#   --version X.Y.Z[-beta.N]      version to stamp on the bundle
#   --sign                        macOS: sign with your Developer ID and notarize
#                                 (run `scripts/macos-signing.sh setup` once first)
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
CHANNEL="${MIRA_CHANNEL:-}"
VERSION=""
SIGN=0
usage() { echo "usage: $0 [--dev|--sidecar] [--channel stable|beta|alpha] [--version V] [--sign]" >&2; exit 2; }
while [ $# -gt 0 ]; do
  case "$1" in
    --dev) MODE=dev ;;
    --sidecar) MODE=sidecar ;;
    --channel) [ $# -ge 2 ] || usage; CHANNEL="$2"; shift ;;
    --version) [ $# -ge 2 ] || usage; VERSION="$2"; shift ;;
    --sign) SIGN=1 ;;
    *) usage ;;
  esac
  shift
done
if [ -z "$CHANNEL" ]; then
  case "$VERSION" in
    *-alpha*) CHANNEL=alpha ;;
    *-beta*) CHANNEL=beta ;;
    *) CHANNEL=stable ;;
  esac
fi
case "$CHANNEL" in stable | beta | alpha) ;; *) echo "unknown channel: $CHANNEL" >&2; usage ;; esac
if [ -n "$VERSION" ] && ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-(alpha|beta)(\.[0-9]+)?)?$ ]]; then
  echo "version must look like 1.2.3, 1.2.3-beta.1 or 1.2.3-alpha.4 (got: $VERSION)" >&2
  exit 2
fi
# The desktop binary bakes the channel in (its scheme, port and the page's
# __MIRA_CHANNEL__); build.rs reruns when this changes.
if [ "$CHANNEL" = stable ]; then unset MIRA_CHANNEL; else export MIRA_CHANNEL="$CHANNEL"; fi
echo "==> channel: $CHANNEL${VERSION:+, version $VERSION}"

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
    TAURI_ARGS=()
    [ "$CHANNEL" = stable ] || TAURI_ARGS+=(--config "channels/$CHANNEL.json")
    [ -z "$VERSION" ] || TAURI_ARGS+=(--config "{\"version\":\"$VERSION\"}")

    if [ "$SIGN" = 1 ]; then
      case "$TRIPLE" in *apple-darwin) ;; *) echo "--sign is macOS-only" >&2; exit 2 ;; esac
      "$ROOT/scripts/macos-signing.sh" check
      # APPLE_SIGNING_IDENTITY signs the app and the sidecar; APPLE_ID,
      # APPLE_PASSWORD and APPLE_TEAM_ID make Tauri notarize and staple it.
      eval "$("$ROOT/scripts/macos-signing.sh" env)"
      # The in-app updater's key, if this Mac has one (CI sets it directly).
      [ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" ] || eval "$("$ROOT/scripts/macos-signing.sh" updater-env)"
    fi
    # With an update key, also build the signed bundle the in-app updater
    # downloads (`<App>.app.tar.gz` + `.sig`).
    [ -z "${TAURI_SIGNING_PRIVATE_KEY:-}" ] || TAURI_ARGS+=(--config '{"bundle":{"createUpdaterArtifacts":true}}')

    # Ship Mira's browser in the bundle so it works on first run, offline.
    # macOS goes through `macOS.files`, which copies directories with their
    # symlinks intact; `resources` would flatten them and break Chrome's
    # framework. MIRA_NO_BUNDLED_CHROMIUM=1 skips it (~350 MB smaller app;
    # the browser then downloads on first use).
    if [ -z "${MIRA_NO_BUNDLED_CHROMIUM:-}" ]; then
      "$ROOT/scripts/fetch-chromium.sh" "$DESKTOP/chromium" "$TRIPLE"
      if [ -f "$DESKTOP/chromium/current" ]; then
        CHROMIUM_DIR=chromium
        if [ "$SIGN" = 1 ]; then
          # Notarization rejects Chrome for Testing's ad-hoc signature:
          # bundle a copy signed with our Developer ID instead.
          "$ROOT/scripts/macos-signing.sh" sign-chromium "$DESKTOP/chromium" "$DESKTOP/target/chromium-signed"
          CHROMIUM_DIR=target/chromium-signed
        fi
        case "$TRIPLE" in
          *apple-darwin) BUNDLE="{\"bundle\":{\"macOS\":{\"files\":{\"Resources/chromium\":\"$CHROMIUM_DIR\"}}}}" ;;
          *) BUNDLE='{"bundle":{"resources":{"chromium/":"chromium/"}}}' ;;
        esac
        TAURI_ARGS+=(--config "$BUNDLE")
      fi
    fi
    cargo tauri build ${TAURI_ARGS[@]+"${TAURI_ARGS[@]}"}

    if [ "$SIGN" = 1 ]; then
      # Tauri notarized the .app inside; the disk image is its own
      # download and needs its own ticket.
      for dmg in "$DESKTOP"/target/release/bundle/dmg/*.dmg; do
        [ -e "$dmg" ] && "$ROOT/scripts/macos-signing.sh" notarize "$dmg"
      done
    fi
    echo
    echo "bundles: $DESKTOP/target/release/bundle/"
    ;;
esac
