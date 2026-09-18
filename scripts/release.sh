#!/usr/bin/env bash
# Cut a new Mira release end-to-end.
#
#   ./scripts/release.sh 0.3.1
#
# What it does, in order:
#
#   1. Sanity-check the tree: on main, clean, up-to-date with origin.
#   2. Bump `version = "X.Y.Z"` in Cargo.toml (every occurrence, including
#      the workspace path deps for internal crates) and in Formula/mira.rb.
#   3. `cargo build --workspace` so Cargo.lock picks up the new version.
#   4. Commit "vX.Y.Z · <title>", tag `vX.Y.Z` annotated, push both.
#   5. Wait for the release workflow to finish (`gh run watch`) — CI
#      builds macOS arm64/x86_64 + Linux arm64/x86_64 tarballs.
#   6. Download the four `.sha256` sidecars, patch Formula/mira.rb with
#      the real hashes.
#   7. Independently verify one tarball's SHA by downloading it and
#      hashing it locally (guards against a broken sidecar upload).
#   8. Push the SHA-filled Formula to `main` on runmira/mira.
#   9. Sync the same Formula into runmira/homebrew-tap and push there
#      so `brew install runmira/tap/mira` resolves to the new version.
#
# What it deliberately does NOT do:
#
#   - Bump the landing-site pill (separate repo, out of tree). Use the
#     `--landing DIR` flag to point at a checkout if you want it done
#     inline; otherwise handle it yourself.
#   - Merge open PRs. Do that with `gh pr merge` before running.
#
# Flags:
#
#   --dry-run           Print every action, don't mutate anything.
#   --skip-verify       Skip the independent SHA verification (fast path
#                       for when you trust CI).
#   --landing DIR       Also bump the v-pill in landing/src/components/
#                       {Hero,Header}.tsx under this directory. No commit;
#                       print a diff line and let the caller push it.
#   --title MESSAGE     Short one-liner for the release commit subject
#                       (default: "vX.Y.Z release").
#
# Requires: git, gh (authed), cargo, curl, shasum. Run from the mira
# repo root or anywhere inside the working tree.

set -euo pipefail

# ---------- args + defaults ---------------------------------------------

DRY_RUN=0
SKIP_VERIFY=0
LANDING_DIR=""
COMMIT_TITLE=""
VERSION=""

usage() {
  sed -n '2,45p' "$0" | sed 's/^# \{0,1\}//'
  exit 1
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run)     DRY_RUN=1;         shift ;;
    --skip-verify) SKIP_VERIFY=1;     shift ;;
    --landing)     LANDING_DIR="$2";  shift 2 ;;
    --title)       COMMIT_TITLE="$2"; shift 2 ;;
    -h|--help)     usage ;;
    -*)            echo "unknown flag: $1" >&2; usage ;;
    *)
      if [[ -z "$VERSION" ]]; then VERSION="$1"
      else echo "unexpected positional arg: $1" >&2; usage
      fi
      shift ;;
  esac
done

[[ -n "$VERSION" ]] || { echo "usage: release.sh <X.Y.Z> [--dry-run] [--skip-verify] [--landing DIR] [--title MSG]"; exit 2; }
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "version must look like X.Y.Z (got: $VERSION)" >&2; exit 2; }

TAG="v$VERSION"
COMMIT_TITLE="${COMMIT_TITLE:-$TAG release}"

# ---------- helpers -----------------------------------------------------

# Everything user-facing goes through log(); mutating actions go through
# run(). run() prints the command and either executes it or dry-echoes.
step() { printf '\n\033[1;36m→\033[0m \033[1m%s\033[0m\n' "$*"; }
info() { printf '  %s\n' "$*"; }
warn() { printf '\033[33m!\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[31m✗\033[0m %s\n' "$*" >&2; exit 1; }

run() {
  if [[ $DRY_RUN -eq 1 ]]; then
    printf '  \033[2m$ %s\033[0m\n' "$*"
  else
    eval "$@"
  fi
}

# In-place sed that works on both BSD (macOS) and GNU. macOS's `sed -i`
# needs an explicit '' after the flag; GNU refuses it. Detect once.
if sed --version >/dev/null 2>&1; then SED_INPLACE=(sed -i);        # GNU
else                                    SED_INPLACE=(sed -i '');    # BSD/macOS
fi

sed_replace() {
  # sed_replace <pattern> <replacement> <file...>
  local pat="$1" rep="$2"; shift 2
  if [[ $DRY_RUN -eq 1 ]]; then
    printf '  \033[2m$ %s -e s|%s|%s|g %s\033[0m\n' "${SED_INPLACE[*]}" "$pat" "$rep" "$*"
  else
    "${SED_INPLACE[@]}" -e "s|${pat}|${rep}|g" "$@"
  fi
}

# ---------- repo root + preflight ---------------------------------------

REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null)" || die "not inside a git repo"
cd "$REPO_ROOT"

step "Preflight"
[[ "$(git rev-parse --abbrev-ref HEAD)" == "main" ]] || die "must be on main (got: $(git rev-parse --abbrev-ref HEAD))"
[[ -z "$(git status --porcelain)" ]] || die "working tree is dirty — commit or stash first"
info "fetching remote…"
run git fetch --tags --quiet origin
BEHIND="$(git rev-list --count HEAD..origin/main 2>/dev/null || echo 0)"
[[ "$BEHIND" == "0" ]] || die "local main is $BEHIND commits behind origin/main — pull first"
if git rev-parse "$TAG" >/dev/null 2>&1; then
  die "tag $TAG already exists locally — did you mean to release a different version?"
fi
if gh release view "$TAG" -R runmira/mira >/dev/null 2>&1; then
  die "release $TAG already published on GitHub — nothing to do"
fi
command -v cargo >/dev/null   || die "cargo not on PATH"
command -v gh    >/dev/null   || die "gh not on PATH"
command -v curl  >/dev/null   || die "curl not on PATH"
command -v shasum >/dev/null  || die "shasum not on PATH"
info "on main, clean, up-to-date; $TAG is unused"

# ---------- 1. bump versions --------------------------------------------

step "Bumping version → $VERSION"

CURRENT="$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -1)"
[[ -n "$CURRENT" ]] || die "couldn't read current version from Cargo.toml"
info "Cargo.toml: $CURRENT → $VERSION"
sed_replace "version = \"$CURRENT\"" "version = \"$VERSION\"" Cargo.toml

info "Formula/mira.rb: $CURRENT → $VERSION"
sed_replace "version \"$CURRENT\"" "version \"$VERSION\"" Formula/mira.rb

# Reset Formula SHAs to the placeholder pattern so a broken tap-push
# can't accidentally serve the previous tag's binaries under the new
# version.
info "Formula/mira.rb: SHAs → placeholders (will backfill after CI)"
sed_replace 'sha256 "[a-f0-9]\{64\}"' 'sha256 "REPLACE_ME"' Formula/mira.rb

if [[ -n "$LANDING_DIR" ]]; then
  if [[ ! -d "$LANDING_DIR" ]]; then die "--landing dir doesn't exist: $LANDING_DIR"; fi
  info "landing pills in $LANDING_DIR"
  # Match either the Hero variant (`v0.3.0 — alpha…`) or the Header
  # pill (`v0.3.0` inside a span). Version-only replacement is enough.
  for f in "$LANDING_DIR/src/components/Hero.tsx" "$LANDING_DIR/src/components/Header.tsx"; do
    [[ -f "$f" ]] || { warn "no such landing file: $f (skipping)"; continue; }
    sed_replace "v$CURRENT" "v$VERSION" "$f"
  done
  info "landing pills bumped in-place; commit + deploy them yourself"
fi

# ---------- 2. rebuild lockfile ----------------------------------------

step "Refreshing Cargo.lock"
run cargo build --workspace --quiet

# ---------- 3. commit + tag + push -------------------------------------

step "Committing + tagging $TAG"
run git add Cargo.toml Cargo.lock Formula/mira.rb
COMMIT_MSG="$TAG · $COMMIT_TITLE"
if [[ $DRY_RUN -eq 1 ]]; then
  printf '  \033[2m$ git commit -m "%s"\033[0m\n' "$COMMIT_MSG"
  printf '  \033[2m$ git tag -a %s -m "%s"\033[0m\n' "$TAG" "$COMMIT_MSG"
else
  git commit -q -m "$COMMIT_MSG"
  git tag -a "$TAG" -m "$COMMIT_MSG"
fi

step "Pushing $TAG to origin"
run git push origin main
run git push origin "$TAG"

# ---------- 4. wait for CI ---------------------------------------------

step "Waiting for release workflow"
if [[ $DRY_RUN -eq 1 ]]; then
  info "(dry-run: would `gh run watch` the release run for $TAG)"
else
  # Grab the newest run for the tag we just pushed. Small sleep so the
  # webhook has time to enqueue the job.
  sleep 5
  RUN_ID="$(gh run list --workflow release.yml --limit 5 --json databaseId,headBranch,event \
             --jq ".[] | select(.headBranch==\"$TAG\") | .databaseId" | head -1)"
  [[ -n "$RUN_ID" ]] || die "couldn't find a release run for tag $TAG — check gh manually"
  info "watching run $RUN_ID (typical: 12–18 min)"
  gh run watch "$RUN_ID" -R runmira/mira --exit-status --interval 30
fi

# ---------- 5. fetch SHAs -----------------------------------------------

step "Fetching SHA sidecars"
SHA_DIR="$(mktemp -d -t mira-sha-XXXXX)"
trap 'rm -rf "$SHA_DIR"' EXIT
run gh release download "$TAG" -R runmira/mira -p "*.sha256" -D "$SHA_DIR"

read_sha() {
  local name="$1"
  local file="$SHA_DIR/${name}.sha256"
  [[ -f "$file" ]] || die "missing sidecar: $file"
  awk '{print $1}' "$file"
}

SHA_DARWIN_ARM="$(read_sha mira-darwin-arm64.tar.gz)"
SHA_DARWIN_X86="$(read_sha mira-darwin-x86_64.tar.gz)"
SHA_LINUX_ARM="$(read_sha mira-linux-arm64.tar.gz)"
SHA_LINUX_X86="$(read_sha mira-linux-x86_64.tar.gz)"

for pair in \
  "darwin-arm64:$SHA_DARWIN_ARM" \
  "darwin-x86_64:$SHA_DARWIN_X86" \
  "linux-arm64:$SHA_LINUX_ARM" \
  "linux-x86_64:$SHA_LINUX_X86"; do
  info "$pair"
done

# ---------- 6. patch Formula -------------------------------------------

step "Patching Formula/mira.rb"

# Each URL line uniquely identifies its target; we replace the placeholder
# `sha256 "REPLACE_ME"` on the line immediately after each URL. Python
# handles the two-line window cleanly; awk/sed contortions are worse.
patch_formula() {
  python3 - "$1" "$SHA_DARWIN_ARM" "$SHA_DARWIN_X86" "$SHA_LINUX_ARM" "$SHA_LINUX_X86" <<'PY'
import sys, re, pathlib
path = pathlib.Path(sys.argv[1])
darw_arm, darw_x86, lin_arm, lin_x86 = sys.argv[2:]
text = path.read_text()
mapping = [
    (r'mira-darwin-arm64\.tar\.gz"\s*\n\s*sha256 "REPLACE_ME"',
     f'mira-darwin-arm64.tar.gz"\n      sha256 "{darw_arm}"'),
    (r'mira-darwin-x86_64\.tar\.gz"\s*\n\s*sha256 "REPLACE_ME"',
     f'mira-darwin-x86_64.tar.gz"\n      sha256 "{darw_x86}"'),
    (r'mira-linux-arm64\.tar\.gz"\s*\n\s*sha256 "REPLACE_ME"',
     f'mira-linux-arm64.tar.gz"\n      sha256 "{lin_arm}"'),
    (r'mira-linux-x86_64\.tar\.gz"\s*\n\s*sha256 "REPLACE_ME"',
     f'mira-linux-x86_64.tar.gz"\n      sha256 "{lin_x86}"'),
]
for pat, rep in mapping:
    new, n = re.subn(pat, rep, text)
    if n != 1:
        sys.exit(f"failed to patch pattern {pat!r} — matched {n} times, expected 1")
    text = new
path.write_text(text)
print(f"  patched {path}")
PY
}
if [[ $DRY_RUN -eq 1 ]]; then
  info "(dry-run: would patch Formula/mira.rb with the four SHAs above)"
else
  patch_formula Formula/mira.rb
fi

# ---------- 7. independent SHA verify -----------------------------------

if [[ $SKIP_VERIFY -eq 1 ]]; then
  warn "skipping independent SHA verification (--skip-verify)"
else
  step "Verifying darwin-arm64 SHA end-to-end"
  URL="https://github.com/runmira/mira/releases/download/$TAG/mira-darwin-arm64.tar.gz"
  TARBALL="$SHA_DIR/mira-darwin-arm64.tar.gz"
  if [[ $DRY_RUN -eq 1 ]]; then
    info "(dry-run: would download $URL and re-hash)"
  else
    info "downloading (10+ MB, uses --retry 3)…"
    curl -sSL --retry 3 --retry-connrefused -o "$TARBALL" "$URL"
    COMPUTED="$(shasum -a 256 "$TARBALL" | awk '{print $1}')"
    if [[ "$COMPUTED" != "$SHA_DARWIN_ARM" ]]; then
      die "SHA mismatch on darwin-arm64! computed=$COMPUTED sidecar=$SHA_DARWIN_ARM"
    fi
    info "computed = sidecar ✓"
  fi
fi

# ---------- 8. push Formula to mira ------------------------------------

step "Pushing Formula SHAs to runmira/mira"
run git add Formula/mira.rb
if [[ $DRY_RUN -eq 1 ]]; then
  printf '  \033[2m$ git commit -m "Formula: fill in %s SHA256s"\033[0m\n' "$TAG"
else
  git commit -q -m "Formula: fill in $TAG SHA256s"
fi
run git push origin main

# ---------- 9. sync homebrew-tap ---------------------------------------

step "Syncing runmira/homebrew-tap"
TAP_DIR="$(mktemp -d -t mira-tap-XXXXX)"
trap 'rm -rf "$SHA_DIR" "$TAP_DIR"' EXIT
run gh repo clone runmira/homebrew-tap "$TAP_DIR" -- --quiet
run cp Formula/mira.rb "$TAP_DIR/Formula/mira.rb"

if [[ $DRY_RUN -eq 1 ]]; then
  printf '  \033[2m$ (cd %s && git add Formula/mira.rb && git commit -m "mira %s" && git push)\033[0m\n' "$TAP_DIR" "$VERSION"
else
  (cd "$TAP_DIR" && git add Formula/mira.rb && git commit -q -m "mira $VERSION" && git push origin HEAD 2>&1 | tail -3)
fi

step "Done"
info "mira release: https://github.com/runmira/mira/releases/tag/$TAG"
info "tap update:   https://github.com/runmira/homebrew-tap/commits/main"
info "install:      brew install runmira/tap/mira    (or: brew upgrade)"
if [[ -n "$LANDING_DIR" ]]; then
  info "landing:      pills bumped in $LANDING_DIR — commit + redeploy separately"
fi
