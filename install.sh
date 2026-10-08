#!/usr/bin/env bash
# Mira installer.
#
#   curl -fsSL https://runmira.dev/install.sh | bash
#
# Environment overrides:
#   MIRA_VERSION   pin to a release tag (default: latest)
#   MIRA_INSTALL_DIR override install directory (default: /usr/local/bin or ~/.local/bin)

set -euo pipefail

REPO="runmira/mira"
BIN_NAME="mira"
VERSION="${MIRA_VERSION:-latest}"

err() { printf 'error: %s\n' "$*" >&2; exit 1; }
info() { printf '%s\n' "$*"; }

need() {
  command -v "$1" >/dev/null 2>&1 || err "missing dependency: $1"
}

need curl
need tar
need uname
need mktemp

detect_platform() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"

  case "$os" in
    Darwin) os="darwin" ;;
    Linux)  os="linux" ;;
    *) err "unsupported OS: $os" ;;
  esac

  case "$arch" in
    x86_64 | amd64) arch="x86_64" ;;
    arm64 | aarch64) arch="arm64" ;;
    *) err "unsupported architecture: $arch" ;;
  esac

  printf '%s-%s' "$os" "$arch"
}

resolve_version() {
  if [ "$VERSION" = "latest" ]; then
    # Follow the /releases/latest redirect to grab the tag without needing jq.
    local url
    url="$(curl -fsSLI -o /dev/null -w '%{url_effective}' \
      "https://github.com/${REPO}/releases/latest")"
    VERSION="${url##*/}"
    [ -n "$VERSION" ] || err "could not resolve latest version"
  fi
}

pick_install_dir() {
  if [ -n "${MIRA_INSTALL_DIR:-}" ]; then
    printf '%s' "$MIRA_INSTALL_DIR"
    return
  fi

  if [ "$(id -u)" = "0" ] || [ -w "/usr/local/bin" ]; then
    printf '/usr/local/bin'
  else
    printf '%s/.local/bin' "$HOME"
  fi
}

# Check the tarball against the release's published SHA-256 (what
# `mira update` does too). Fails on a mismatch; only warns when the release
# has no checksum file or this machine has no SHA-256 tool.
verify_checksum() {
  local tmp="$1" tarball="$2" url="$3" expected actual
  if ! curl -fsSL "${url}.sha256" -o "$tmp/$tarball.sha256"; then
    info "warning: no checksum published for ${tarball}; skipping verification"
    return
  fi
  expected="$(awk '{print $1}' "$tmp/$tarball.sha256")"
  if command -v shasum >/dev/null 2>&1; then
    actual="$(shasum -a 256 "$tmp/$tarball" | awk '{print $1}')"
  elif command -v sha256sum >/dev/null 2>&1; then
    actual="$(sha256sum "$tmp/$tarball" | awk '{print $1}')"
  else
    info "warning: no shasum/sha256sum found; skipping checksum verification"
    return
  fi
  [ "$expected" = "$actual" ] || err "checksum mismatch for ${tarball} (expected ${expected}, got ${actual})"
  info "checksum ok"
}

main() {
  local platform tarball url tmp install_dir sudo
  platform="$(detect_platform)"
  resolve_version

  tarball="${BIN_NAME}-${platform}.tar.gz"
  url="https://github.com/${REPO}/releases/download/${VERSION}/${tarball}"

  install_dir="$(pick_install_dir)"
  mkdir -p "$install_dir"

  sudo=""
  if [ ! -w "$install_dir" ]; then
    command -v sudo >/dev/null 2>&1 || err "$install_dir is not writable and sudo is unavailable"
    sudo="sudo"
  fi

  tmp="$(mktemp -d)"
  # Expand now: `tmp` is local, so it's gone by the time EXIT fires.
  # shellcheck disable=SC2064
  trap "rm -rf '$tmp'" EXIT

  info "downloading ${tarball} (${VERSION})"
  curl -fsSL "$url" -o "$tmp/$tarball" \
    || err "download failed: $url"

  verify_checksum "$tmp" "$tarball" "$url"

  info "extracting"
  tar -C "$tmp" -xzf "$tmp/$tarball"

  [ -f "$tmp/$BIN_NAME" ] || err "binary '$BIN_NAME' missing from tarball"
  chmod +x "$tmp/$BIN_NAME"

  info "installing to ${install_dir}/${BIN_NAME}"
  $sudo mv "$tmp/$BIN_NAME" "${install_dir}/${BIN_NAME}"

  info ""
  info "installed $($sudo "${install_dir}/${BIN_NAME}" --version 2>/dev/null || echo "${BIN_NAME} ${VERSION}")"

  # install.sh is served from main, so it can be newer than the release.
  if "${install_dir}/${BIN_NAME}" service --help >/dev/null 2>&1; then
    info "keep it running in the background: mira service install"
    info "update later with: mira update"
  fi

  case ":$PATH:" in
    *":$install_dir:"*) ;;
    *) info "note: ${install_dir} is not on your PATH. Add it to your shell profile." ;;
  esac
}

main "$@"
