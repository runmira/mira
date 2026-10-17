# Devices

The Device pane shows a live iOS Simulator or Android Emulator inside Mira,
and agents drive the same devices. This page is the contract between the
server (`crates/mira-server/src/devices/`) and the pane.

## Moving parts

- **Device hub** (`expo-device-hub`, pinned): discovery, boot/shutdown and
  video streaming for both platforms. It vendors serve-sim (iOS) and
  serve-emu (Android).
- **agent-device** (pinned): the command line agents drive devices with.
  Installed only after the user allows agent access.

Both are npm packages that need Node 22.12+. Mira uses the system Node when
it is new enough, otherwise it downloads a pinned official Node build and
checks it against the release's `SHASUMS256.txt`.

Nothing is downloaded until the user turns the hub on. Each tool installs
into `~/.mira/tools/<name>/<version>/` through a staging directory and only
counts as installed once `.install-complete` exists, so an interrupted
install is wiped and redone. Versions live in
`crates/mira-server/src/devices/tool_versions.json`; a new pin installs the
next time it is needed, with no fallback to an older version.

## Security

The hub binds `127.0.0.1` on a free port and runs with `--require-token`.
The random token is read from its stdout and never leaves the server. Every
request reaches it through `/api/device-hub/*`, which:

- sits behind the pairing guard like every other route, and serves only
  callers on this computer for now;
- allows a fixed list of paths and refuses everything else with 404 (the
  hub's shell-exec and device-settings routes are never reachable);
- allows only GET, except screenshot capture and stream tuning;
- drops the caller's cookie/authorization headers and adds the hub token;
- marks responses `Content-Encoding: identity` and
  `Cache-Control: no-transform, no-store` so the compression layer never
  buffers a stream.

Device settings (appearance, permissions, location, push…) never go through
the hub: `POST /api/devices/action` runs one typed `simctl`/`adb` command per
control and returns the value read back.

## HTTP API

All bodies are JSON. Mutating routes answer 403 to anyone but this computer.

### `GET /api/devices/state` and `GET /api/devices/events` (SSE)

The current state, and the same object streamed on every change:

```jsonc
{
  "hub": "off" | "installing" | "starting" | "ready" | "failed",
  "hubDetail": "Installing device hub… (npm)",   // or the failure reason
  "hubEnabled": true,
  "agentAccess": false,
  "agentTools": "absent" | "installing" | "ready" | "failed",
  "agentDetail": null,
  "platforms": {
    "ios":     { "available": true,  "reason": null },
    "android": { "available": false, "reason": "Android SDK Command-line Tools (latest) are missing from …" }
  },
  "versions": { "hub": "0.15.3", "agentDevice": "0.21.25", "node": "v22.23.3 (system)" }
}
```

### `POST /api/devices/settings`

`{ "hubEnabled"?: bool, "agentAccess"?: bool }`. Turning the hub on starts
the download and the hub. Turning it off stops the hub and the agent tools
(devices keep running) and clears `agentAccess`. Saved to `~/.mira/mira.yaml`
under `devices:`; a repo's `.mira/config.yaml` cannot set these. Returns the
new state.

### `GET /api/devices/list`

```jsonc
{
  "devices": [
    { "platform": "ios", "id": "91D0…", "name": "iPhone 17 Pro", "os": "iOS 26.2",
      "booted": false, "frame": "ios:iphone-17-pro" }
  ],
  "notes": ["Xcode is installed, but no iOS Simulator is available. Install a runtime in Xcode Settings → Components."]
}
```

### `POST /api/devices/boot` · `POST /api/devices/shutdown`

`{ "platform": "ios" | "android", "id": "…" }`. Boot waits until the device
is ready to stream and returns `{ "stream": { … } }` (see below).

### `POST /api/devices/screenshot`

`{ "platform", "id" }` → `image/png`.

### `POST /api/devices/action`

`{ "platform", "id", "action": "appearance", "value": "dark" }` → the
settings read back. Actions: `appearance`, `textSize`, `location`,
`permission` (`{ app, service, grant }`), `push` (iOS: `{ app, payload }`),
`orientation` (Android), `network` (Android), `foregroundApp` (read only).

### `GET /api/devices/tool-versions`

Installed and pinned versions, read without installing or starting anything.

## Streaming

`boot` and `list` give each booted device a `stream` descriptor whose URLs
are under `/api/device-hub`:

- **iOS**: `video` (AVCC over HTTP, decode with WebCodecs), `mjpeg`
  (fallback when `VideoDecoder.isConfigSupported` rejects the H.264 High 5.1
  profile or the page isn't a secure context), `input` (binary WebSocket),
  `config`.
- **Android**: `ws` (SEMU-framed H.264 and JSON gestures on one socket).

## Agents

With agent access on, agents get four MCP tools: `device_list`,
`device_open`, `device_screenshot` and `device_close`. They're listed only
while the hub is on, access is allowed and this machine can run a platform.
`device_open` boots or attaches to a device, shows it in the Device pane
(`device_active` over `/ws`) and returns how to drive it.

Driving is the `agent-device` command line, not a tool. Mira writes a
launcher to `~/.mira/tools/bin/agent-device` (Mira's Node, the pinned
agent-device, and a state directory under `~/.mira/tools`) while access is
on and deletes it when access is turned off. That directory is on the PATH
of external agents and of Mira's own shell commands. The first iOS command
builds agent-device's test runner, about a minute once per machine; no
`idb` is involved.
