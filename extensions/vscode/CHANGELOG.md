# Changelog

## 0.2.0

- Approvals from the chat: when Mira asks (manual mode), the diff shows
  inline and a prompt offers Allow, Allow for session, or Deny. No more
  switching to the browser.
- Auto-start: if nothing answers at `mira.baseUrl` (loopback only),
  the extension starts `mira serve` and stops it when the window
  closes. New settings `mira.autoStart` and `mira.path`, a
  *Mira: Start Server* command, and a "Mira" output channel.
- Fixed the approval message type to match the server.
- Ships a real LICENSE file.

## 0.1.1 — 2026-09-13

Store-page polish. No behaviour changes.

- Better description, keywords, and dark gallery banner colour (matches Mira's UI).
- Rewritten README that reads as a proper Marketplace listing.
- `preview: true` flag so the store shows an alpha badge.

## 0.1.0 — 2026-09-13

First release.

- `@mira` chat participant registered against VS Code's Chat API (1.93+).
- WebSocket client streams tokens and tool-call events from a
  user-run `mira serve`.
- Tool calls render inline as fenced code blocks with a one-line
  arg summary (preferring `path` / `command` / `target`).
- Slash commands: `/reset`, `/mode`, `/model`.
- File and selection references from VS Code's chat mechanics
  (`#file:...`, `#selection`, drag-and-drop) flatten into the prompt.
- Follow-active-folder: PUT `/api/cwd` on each send when the workspace
  folder differs from the last.
- Commands: *Mira: Open Web UI in Browser*, *Mira: Show Connection Status*.
- Config: `mira.baseUrl`, `mira.showToolCalls`, `mira.followActiveFolder`.
