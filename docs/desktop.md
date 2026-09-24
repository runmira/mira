# Mira desktop app

`apps/desktop` is a [Tauri 2](https://tauri.app) app that puts the web UI
in a native window. It bundles the `mira` binary, starts `mira serve` on a
loopback port, and loads the UI from it, so everything the web UI does
(sessions, worktrees, remote environments, settings) works unchanged.

What the app adds over `mira serve --open`:

- **A native window.** Nothing to start in a terminal, and a second
  launch focuses the running app instead of starting another server.
- **Sign-in in your browser.** Google refuses OAuth inside embedded
  webviews, so the app opens sign-in in the system browser, which returns
  to the app through a `mira://auth-callback` link.
- **Links open in your browser.** Pull request links, docs and provider
  sign-in pages leave the app instead of replacing the UI.
- **Your shell's environment.** Apps started from the Dock or a launcher
  get a minimal `PATH` and none of the keys exported in your shell
  profile. The app reads your login shell's environment (with a 5-second
  cap) and passes it to the server, so `git`, `cargo`, `gh` and your API
  keys resolve the same way they do in a terminal.
- **Cleanup.** The server stops when the app quits. After a crash or force
  quit, the next launch stops the leftover server before starting a new
  one. On Linux the kernel also stops it when the app dies.

The server listens on `127.0.0.1:8797` (the next free port if that one is
taken). The port is fixed on purpose: the UI keeps its sign-in session in
the page's storage, which is scoped to the port. Server output goes to
`~/.mira/logs/desktop-server.log`.

## Build it

Needs Rust, Node/npm, and the Tauri CLI:

```bash
cargo install tauri-cli --version "^2" --locked
```

On Linux, also the WebKitGTK and GTK development packages, for example on
Debian/Ubuntu:

```bash
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libayatana-appindicator3-dev
```

The web UI reads `VITE_SUPABASE_URL` and `VITE_SUPABASE_PUBLISHABLE_KEY`
at build time, from the environment or
`crates/mira-server/frontend/.env`.

Then, from the repo root:

```bash
scripts/build-desktop.sh --dev   # run from source, with a debug server
scripts/build-desktop.sh         # release bundles
```

The script builds the web UI, builds `mira`, copies it to
`apps/desktop/binaries/mira-<target-triple>` (where Tauri's bundler looks
for the sidecar), and then runs `cargo tauri dev` or `cargo tauri build`.
Release bundles land in `apps/desktop/target/release/bundle/`: `.app` and
`.dmg` on macOS, `.msi` and `.exe` on Windows, `.deb`, `.rpm` and
`.AppImage` on Linux.

To try the app against a `mira` you built some other way, set `MIRA_BIN`:

```bash
MIRA_BIN=$PWD/target/debug/mira cargo run --manifest-path apps/desktop/Cargo.toml
```

`apps/desktop` is its own Cargo workspace, so `cargo build --workspace`
at the root still needs no GUI libraries.

## One-time setup: Supabase redirect URL

In the Supabase dashboard, under **Authentication → URL Configuration →
Redirect URLs**, add:

```
mira://auth-callback
```

Without it, Supabase sends the browser back to the site URL instead of to
the app.

On macOS, `mira://` links only reach the app once it's installed as a
bundle (for example, moved to `/Applications`), not when it runs through
`cargo tauri dev`. On Windows and Linux, the app registers the scheme
when it starts.

## Not done yet

- **Signing and notarization.** Unsigned macOS builds need right-click →
  Open the first time. Unsigned Windows builds show a SmartScreen warning.
  Both need certificates configured in CI.
- **Auto-update**, with Tauri's updater and GitHub Releases.
- **Release CI** that builds macOS arm64/x64, Windows and Linux bundles.
- **macOS permissions for computer use.** Screen Recording and
  Accessibility are granted to the app you launch, which will be Mira
  itself; the Info.plist usage strings still need to be added.
