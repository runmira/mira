#!/usr/bin/env bash
# Sign and notarize the Mira desktop app for macOS.
#
#   scripts/macos-signing.sh setup            # once per Mac: pick the certificate, save credentials
#   scripts/macos-signing.sh check            # is this Mac ready to sign and notarize?
#   scripts/macos-signing.sh env              # print the APPLE_* exports for `cargo tauri build`
#   scripts/macos-signing.sh sign-chromium SRC OUT   # sign a copy of the bundled browser
#   scripts/macos-signing.sh notarize FILE    # notarize + staple a .dmg, .zip or .app
#   scripts/macos-signing.sh updater-key      # once: the key that signs in-app updates
#   scripts/macos-signing.sh ci-secrets P12   # give GitHub Actions everything the release needs
#
# You normally don't call these yourself: `scripts/build-desktop.sh --sign`
# does, in order. `setup` is the one step that needs you.
#
# What you need first (Apple's side, done once, in a browser):
#   1. An Apple ID enrolled in the Apple Developer Program (paid, per year).
#   2. A "Developer ID Application" certificate in this Mac's login keychain.
#      Easiest: Xcode → Settings → Accounts → (your team) → Manage
#      Certificates → + → Developer ID Application.
#   3. An app-specific password for notarization, made at
#      https://account.apple.com → Sign-In and Security → App-Specific Passwords.
#      This is NOT your Apple ID password, which notarization never takes.
#
# Where things are kept:
#   - The app-specific password: your login keychain only (service
#     "dev.runmira.notarize").
#   - The certificate name, Apple ID and team ID (not secret):
#     apps/desktop/.signing.env, which git ignores.
# Nothing secret is written to the repository.
#
# In CI the same commands read APPLE_SIGNING_IDENTITY, APPLE_ID,
# APPLE_TEAM_ID and APPLE_PASSWORD from the environment instead
# (see .github/workflows/desktop-release.yml).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ENV_FILE="$ROOT/apps/desktop/.signing.env"
ENTITLEMENTS="$ROOT/apps/desktop/entitlements/chromium.plist"
KEYCHAIN_SERVICE="dev.runmira.notarize"
# The in-app updater only installs updates signed with this key (separate
# from Apple's signature). The private key never enters the repository.
UPDATER_KEY="${MIRA_UPDATER_KEY:-$HOME/.tauri/mira-updater.key}"
UPDATER_SERVICE="dev.runmira.updater"

say() { printf '\033[1;36m→\033[0m %s\n' "$*" >&2; }
ok() { printf '\033[32m✓\033[0m %s\n' "$*" >&2; }
die() { printf '\033[31m✗\033[0m %s\n' "$*" >&2; exit 1; }

[ "$(uname -s)" = Darwin ] || die "signing and notarization only run on macOS"

load_env() {
  # Already in the environment (CI): nothing to read.
  if [ -n "${APPLE_SIGNING_IDENTITY:-}" ] && [ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_TEAM_ID:-}" ]; then return; fi
  [ -f "$ENV_FILE" ] || die "not set up yet: run scripts/macos-signing.sh setup"
  # shellcheck disable=SC1090
  . "$ENV_FILE"
  : "${APPLE_SIGNING_IDENTITY:?missing in $ENV_FILE}" "${APPLE_ID:?missing in $ENV_FILE}" "${APPLE_TEAM_ID:?missing in $ENV_FILE}"
}

identities() {
  security find-identity -v -p codesigning | sed -n 's/^ *[0-9]*) [0-9A-F]* "\(Developer ID Application: .*\)"$/\1/p'
}

keychain_password() {
  if [ -n "${APPLE_PASSWORD:-}" ]; then
    printf '%s' "$APPLE_PASSWORD"
    return
  fi
  security find-generic-password -s "$KEYCHAIN_SERVICE" -a "$APPLE_ID" -w 2>/dev/null \
    || die "no notarization password in the keychain for $APPLE_ID: run scripts/macos-signing.sh setup"
}

cmd_setup() {
  xcrun --find notarytool >/dev/null 2>&1 || die "notarytool not found: install Xcode (or its command line tools) first"

  say "Looking for a Developer ID Application certificate…"
  local ids identity
  ids="$(identities)"
  if [ -z "$ids" ]; then
    cat >&2 <<'EOF'
No "Developer ID Application" certificate in your keychain.

  1. Make sure your Apple ID is enrolled in the Apple Developer Program:
     https://developer.apple.com/programs/enroll/
  2. In Xcode: Settings → Accounts → add your Apple ID → select the team →
     Manage Certificates… → + → Developer ID Application.
     (Or create one at https://developer.apple.com/account/resources/certificates
     from a CSR made in Keychain Access, then double-click the .cer.)
  3. Run this again.
EOF
    exit 1
  fi
  if [ "$(printf '%s\n' "$ids" | wc -l | tr -d ' ')" = 1 ]; then
    identity="$ids"
  else
    say "More than one certificate; pick the one to sign Mira with:"
    local IFS=$'\n'
    select identity in $ids; do [ -n "$identity" ] && break; done
  fi
  ok "certificate: $identity"
  local team="${identity##*(}"
  team="${team%)}"

  local current_id=""
  [ -f "$ENV_FILE" ] && current_id="$(sed -n 's/^APPLE_ID="\(.*\)"$/\1/p' "$ENV_FILE")"
  local apple_id
  read -r -p "Apple ID (email) of the developer account${current_id:+ [$current_id]}: " apple_id
  apple_id="${apple_id:-$current_id}"
  [ -n "$apple_id" ] || die "an Apple ID is required"
  local team_in
  read -r -p "Team ID [$team]: " team_in
  team_in="${team_in:-$team}"
  # Apple only notarizes apps signed by the submitting team's certificate.
  if [ "$team_in" != "$team" ]; then
    die "team $team_in doesn't match the certificate's team ($team). Use $team, or create a
  Developer ID Application certificate for $team_in (Xcode → Settings → Accounts →
  Manage Certificates → + → Developer ID Application) and run setup again"
  fi
  case "$apple_id" in
    *@*.*) ;;
    *) die "\"$apple_id\" doesn't look like an email address" ;;
  esac

  cat >&2 <<'EOF'

Notarization needs an app-specific password — NOT your Apple ID password.
Make one at https://account.apple.com → Sign-In and Security →
App-Specific Passwords (it looks like abcd-efgh-ijkl-mnop).
EOF
  local password
  read -r -s -p "App-specific password: " password
  echo >&2
  [ -n "$password" ] || die "a password is required"

  say "Checking the credentials with Apple…"
  xcrun notarytool history --apple-id "$apple_id" --team-id "$team" --password "$password" >/dev/null \
    || die "Apple rejected these credentials (check the Apple ID, team and that it's an app-specific password)"
  ok "Apple accepted them"

  security add-generic-password -U -s "$KEYCHAIN_SERVICE" -a "$apple_id" -l "Mira notarization ($apple_id)" -w "$password"
  unset password
  ok "password saved to your login keychain"

  cat >"$ENV_FILE" <<EOF
# Written by scripts/macos-signing.sh setup. Not secret; git-ignored.
APPLE_SIGNING_IDENTITY="$identity"
APPLE_ID="$apple_id"
APPLE_TEAM_ID="$team"
EOF
  ok "settings saved to apps/desktop/.signing.env"
  echo >&2
  say "Ready. Build a signed, notarized app with:  scripts/build-desktop.sh --sign [--channel alpha|beta]"
}

cmd_check() {
  load_env
  if ! identities | grep -Fxq "$APPLE_SIGNING_IDENTITY"; then
    # Say why: a listed-but-invalid identity is almost always a missing
    # intermediate certificate, not a missing key.
    security find-identity -p codesigning >&2 || true
    die "certificate not usable for signing: $APPLE_SIGNING_IDENTITY (not in the keychain, or its Apple intermediate certificate is missing)"
  fi
  ok "certificate: $APPLE_SIGNING_IDENTITY"
  xcrun notarytool history --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" --password "$(keychain_password)" >/dev/null 2>&1 \
    || die "Apple rejected the saved notarization credentials: run setup again"
  ok "notarization: $APPLE_ID (team $APPLE_TEAM_ID)"
}

# The variables `cargo tauri build` reads to sign the app and its sidecar and
# to notarize and staple the .app.
cmd_env() {
  load_env
  printf 'export APPLE_SIGNING_IDENTITY=%q\n' "$APPLE_SIGNING_IDENTITY"
  printf 'export APPLE_ID=%q\n' "$APPLE_ID"
  printf 'export APPLE_TEAM_ID=%q\n' "$APPLE_TEAM_ID"
  printf 'export APPLE_PASSWORD=%q\n' "$(keychain_password)"
}

sign() {
  # sign <path> [entitlements]
  local args=(--force --timestamp --options runtime --sign "$APPLE_SIGNING_IDENTITY")
  [ -n "${2:-}" ] && args+=(--entitlements "$2")
  codesign "${args[@]}" "$1"
}

# Chrome for Testing ships ad-hoc signed, which notarization rejects. Sign a
# copy inside-out (libraries, helper tools, helper apps, the framework, then
# the app) with the Developer ID and the JIT entitlements V8 needs.
cmd_sign_chromium() {
  local src="${1:?usage: sign-chromium SRC OUT}" out="${2:?usage: sign-chromium SRC OUT}"
  if [ -z "${APPLE_SIGNING_IDENTITY:-}" ]; then load_env; fi
  [ -d "$src" ] || die "no such directory: $src"
  say "Signing Chromium (a few hundred files)…"
  rm -rf "$out"
  mkdir -p "$(dirname "$out")"
  ditto "$src" "$out" # keeps the framework's symlinks intact

  local f
  while IFS= read -r -d '' f; do sign "$f"; done < <(find "$out" -type f -name '*.dylib' -print0)
  while IFS= read -r -d '' f; do
    case "$f" in *.app/Contents/MacOS/*) continue ;; esac
    file -b "$f" | grep -q 'Mach-O' && sign "$f"
  done < <(find "$out" -type f -perm -111 ! -name '*.dylib' -path '*/Helpers/*' -print0)
  while IFS= read -r -d '' f; do sign "$f" "$ENTITLEMENTS"; done \
    < <(find "$out" -type d -name '*.app' -path '*/Helpers/*' -print0)
  while IFS= read -r -d '' f; do sign "$f"; done < <(find "$out" -type d -name '*.framework' -print0)
  while IFS= read -r -d '' f; do
    sign "$f" "$ENTITLEMENTS"
    codesign --verify --strict --deep "$f" || die "Chromium failed verification after signing: $f"
  done < <(find "$out" -maxdepth 4 -type d -name '*.app' ! -path '*/Contents/*' -print0)
  ok "Chromium signed"
}

cmd_notarize() {
  local file="${1:?usage: notarize FILE}"
  [ -e "$file" ] || die "no such file: $file"
  load_env
  local submit="$file" tmp=""
  if [ -d "$file" ]; then
    # notarytool takes archives, not bundles.
    tmp="$(mktemp -d)"
    submit="$tmp/$(basename "$file").zip"
    ditto -c -k --keepParent "$file" "$submit"
  fi
  say "Submitting $(basename "$file") to Apple (usually 1–10 minutes)…"
  local out id status
  local password
  password="$(keychain_password)"
  out="$(xcrun notarytool submit "$submit" --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" --password "$password" \
    --wait --output-format json)" || true
  id="$(printf '%s' "$out" | sed -n 's/.*"id" *: *"\([^"]*\)".*/\1/p' | head -1)"
  status="$(printf '%s' "$out" | sed -n 's/.*"status" *: *"\([^"]*\)".*/\1/p' | head -1)"
  [ -n "$tmp" ] && rm -rf "$tmp"
  if [ "$status" != Accepted ]; then
    if [ -n "$id" ]; then
      xcrun notarytool log "$id" --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" --password "$password" >&2 || true
    else
      printf '%s\n' "$out" >&2
    fi
    die "notarization ${status:-failed} for $(basename "$file")"
  fi
  ok "notarized ($id)"
  xcrun stapler staple "$file" >/dev/null
  xcrun stapler validate "$file" >/dev/null
  ok "ticket stapled: $(basename "$file") opens offline without warnings"
}

cmd_updater_key() {
  if [ -f "$UPDATER_KEY" ]; then
    ok "updater key exists: $UPDATER_KEY"
  else
    say "Creating the update-signing key…"
    mkdir -p "$(dirname "$UPDATER_KEY")"
    local password
    password="$(openssl rand -hex 24)"
    (cd "$ROOT/apps/desktop" && cargo tauri signer generate --ci -p "$password" -w "$UPDATER_KEY" >/dev/null)
    chmod 600 "$UPDATER_KEY"
    security add-generic-password -U -s "$UPDATER_SERVICE" -a mira -l "Mira update signing key password" -w "$password"
    ok "key: $UPDATER_KEY (password in your login keychain)"
    say "Back it up somewhere safe: losing it means installed apps can't verify new updates."
  fi
  echo "public key (goes in apps/desktop/tauri.conf.json → plugins.updater.pubkey):" >&2
  cat "$UPDATER_KEY.pub"
  echo
}

# The variables `cargo tauri build` reads to sign update bundles; prints
# nothing when there's no key, so local builds still work without one.
cmd_updater_env() {
  [ -f "$UPDATER_KEY" ] || return 0
  printf 'export TAURI_SIGNING_PRIVATE_KEY=%q\n' "$(cat "$UPDATER_KEY")"
  printf 'export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=%q\n' \
    "$(security find-generic-password -s "$UPDATER_SERVICE" -a mira -w 2>/dev/null)"
}

# Copies the local setup into the repository's Actions secrets, for
# .github/workflows/desktop-release.yml. Takes the certificate exported
# from Keychain Access (right-click it → Export → .p12).
cmd_ci_secrets() {
  local p12="${1:?usage: ci-secrets CERT.p12}"
  [ -f "$p12" ] || die "no such file: $p12"
  command -v gh >/dev/null || die "needs the GitHub CLI (gh), signed in"
  load_env
  local p12_password
  read -r -s -p "Password you gave the .p12 when exporting: " p12_password
  echo >&2
  local repo
  repo="$(cd "$ROOT" && gh repo view --json nameWithOwner -q .nameWithOwner)"
  read -r -p "Set APPLE_* secrets on $repo? [y/N] " yes
  [[ "$yes" =~ ^[Yy]$ ]] || die "cancelled"
  base64 <"$p12" | tr -d '\n' | gh secret set APPLE_CERTIFICATE -R "$repo"
  printf '%s' "$p12_password" | gh secret set APPLE_CERTIFICATE_PASSWORD -R "$repo"
  printf '%s' "$APPLE_SIGNING_IDENTITY" | gh secret set APPLE_SIGNING_IDENTITY -R "$repo"
  printf '%s' "$APPLE_ID" | gh secret set APPLE_ID -R "$repo"
  printf '%s' "$APPLE_TEAM_ID" | gh secret set APPLE_TEAM_ID -R "$repo"
  keychain_password | gh secret set APPLE_PASSWORD -R "$repo"
  if [ -f "$UPDATER_KEY" ]; then
    gh secret set TAURI_SIGNING_PRIVATE_KEY -R "$repo" <"$UPDATER_KEY"
    security find-generic-password -s "$UPDATER_SERVICE" -a mira -w | tr -d '\n' \
      | gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD -R "$repo"
  else
    say "no updater key yet (scripts/macos-signing.sh updater-key): CI builds won't offer in-app updates"
  fi
  # The web UI's sign-in settings, from the frontend's local env file.
  local env_file="$ROOT/crates/mira-server/frontend/.env.local" name value
  for name in VITE_SUPABASE_URL VITE_SUPABASE_PUBLISHABLE_KEY; do
    value="$(sed -n "s/^$name=//p" "$env_file" 2>/dev/null | tail -1)"
    if [ -n "$value" ]; then
      printf '%s' "$value" | gh secret set "$name" -R "$repo"
    else
      say "$name not found in $env_file: set it with gh secret set $name"
    fi
  done
  ok "secrets set on $repo"
}

case "${1:-}" in
  setup) cmd_setup ;;
  check) cmd_check ;;
  env) cmd_env ;;
  sign-chromium) shift; cmd_sign_chromium "$@" ;;
  notarize) shift; cmd_notarize "$@" ;;
  updater-key) cmd_updater_key ;;
  updater-env) cmd_updater_env ;;
  ci-secrets) shift; cmd_ci_secrets "$@" ;;
  *) sed -n '2,30p' "$0" | sed 's/^# \{0,1\}//'; exit 2 ;;
esac
