#!/usr/bin/env bash
# Build a Linux x86_64 `mira` on macOS (or any host) for cloud tasks.
# E2B sandboxes are Linux x86_64; point `cloud.binary` at the result.
#
#   scripts/build-linux-mira.sh
#   → target/x86_64-unknown-linux-gnu/release/mira
set -euo pipefail

TARGET=x86_64-unknown-linux-gnu
command -v zig >/dev/null 2>&1 || { echo "error: needs zig (brew install zig)" >&2; exit 1; }
cargo zigbuild --version >/dev/null 2>&1 || cargo install --locked cargo-zigbuild
rustup target add "$TARGET" >/dev/null
cargo zigbuild --release -p mira-cli --target "$TARGET"

BIN="$(pwd)/target/$TARGET/release/mira"
echo
echo "built $BIN"
echo "use it with:  export MIRA_CLOUD_BINARY=$BIN"
echo "or in ~/.mira/mira.yaml:  cloud: { binary: $BIN }"
