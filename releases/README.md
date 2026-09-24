# Mira release process

How Mira releases are cut these days. Everything is automated by
[`scripts/release.sh`](../scripts/release.sh) — this doc explains what it
does, what it needs from you, and how to recover when a step fails.

## TL;DR

```sh
# from a clean main, up to date with origin:
./scripts/release.sh 0.3.9 --title "short headline"
```

That single command runs the whole pipeline: preflight → version bump →
build → commit + tag → push → CI build → hash verification → Homebrew
formula patch → tap sync → landing-site update.

## Prerequisites

- On `main`, working tree **clean**, in sync with `origin/main`
  (the script hard-fails otherwise — commit or stash first).
- `gh` authenticated against `runmira/mira`.
- On PATH: `git`, `gh`, `cargo`, `curl`, `shasum`, `python3`.
- The landing repo checked out at `../landing` (sibling of this repo).
- Open PRs merged **before** starting — the script does not merge.
- Write release notes: drop a `releases/vX.Y.Z.md` file in this folder
  and include it in the release commit (see `v0.3.8.md` for the format).

## The steps, in order

1. **Preflight** — verifies branch, clean tree, sync with origin, that
   the tag and the GitHub release don't exist yet, and that all tools
   are on PATH. Also preflights the landing repo.
2. **Version bump** — rewrites `version` in the root `Cargo.toml`
   (`[workspace.package]`, single source for all crates) and `version`
   in `Formula/mira.rb`.
3. **Build** — `cargo build --workspace` so `Cargo.lock` records the
   new version.
4. **Commit + tag + push** — commit subject `vX.Y.Z · <title>`
   (`--title`), annotated tag `vX.Y.Z`, both pushed to `origin/main`.
   Pushing the tag triggers the release workflow.
5. **CI build** — `.github/workflows/release.yml` builds four
   tarballs: macOS arm64/x86_64, Linux arm64/x86_64, each with a
   `.sha256` sidecar. The script waits with `gh run watch`.
6. **Formula hashes** — downloads the four `.sha256` sidecars and
   patches the placeholders in `Formula/mira.rb`.
7. **Independent verification** — downloads one tarball and hashes it
   locally, comparing against the sidecar (`--skip-verify` to skip;
   don't).
8. **Formula push** — commits the hash-filled formula to `runmira/mira`.
9. **Tap sync** — copies the formula into `runmira/homebrew-tap` and
   pushes. This is what makes `brew install runmira/tap/mira` serve
   the new version.
10. **Landing site** — updates the version pill in the separate
    landing repo, commits, pushes.

## Flags

| Flag           | Effect                                          |
| -------------- | ----------------------------------------------- |
| `--dry-run`    | Print every action, mutate nothing.             |
| `--skip-verify`| Skip the independent tarball SHA verification.  |
| `--title MSG`  | One-liner for the release commit subject.       |

## Files the release touches

| File                                   | Why                              |
| -------------------------------------- | -------------------------------- |
| `Cargo.toml`                           | workspace version                |
| `Cargo.lock`                           | rebuilt by the build step        |
| `Formula/mira.rb`                      | version + per-platform sha256    |
| `runmira/homebrew-tap/Formula/mira.rb` | synced copy of the formula       |
| landing repo version pill              | marketing site                   |

## Conventions

- Tags are `vX.Y.Z`, annotated, exactly matching the version string.
- Release commits are `vX.Y.Z · <title>`.
- User-facing changes get a `releases/vX.Y.Z.md` note in the release
  commit — grouped New / Changed / Fixed, written for users of the
  binary, not for reviewers.

## When something fails

- **"working tree is dirty"** — commit or stash. The script never
  carries uncommitted changes into a release.
- **"tag already exists locally" / "release already published"** — the
  version is taken; bump or delete the stray tag deliberately.
- **CI red after the tag** — fix forward: revert or patch, delete the
  bad tag/release with `gh release delete` + `git push :refs/tags/vX.Y.Z`,
  and re-run with the same or bumped version.
- **Homebrew users report a bad hash** — the sidecar download raced a
  re-upload; re-run steps 6–9 by hand or re-run the script after
  deleting the release.

## Past releases

Notes live in this folder: [`v0.3.9.md`](./v0.3.9.md), [`v0.3.8.md`](./v0.3.8.md).
