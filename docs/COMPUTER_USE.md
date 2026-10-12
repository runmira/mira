# Computer use and browser tools

Mira has two optional tools for work that happens outside the terminal:

- **`computer`**: sees the screen and drives the mouse and keyboard.
  You can use it to click through a GUI installer, check the app Mira
  just built, or anything else that only works in a window.
- **`browser`**: drives a real Chrome / Chromium / Edge / Brave over
  the DevTools protocol, in **Mira's own profile** (never your everyday
  one). You can use it for web flows: logging into a dashboard, filling
  a form, reproducing a UI bug.

Both are **off by default**.

`computer` drives *your* desktop by default. It can also drive a
**remote** machine through
[cua-spacesd](https://github.com/trycua/cua) — another box on the
network, a sandbox, or a machine you cannot reach at all — and on a
guest that supports it, deliver input without taking your pointer. See
[Driving a remote machine](#driving-a-remote-machine-cua).

## Turning them on

For one run:

```sh
mira --computer          # desktop control
mira --browser           # browser automation
mira serve --browser     # works with the web UI too
```

Permanently, in `~/.mira/mira.yaml`:

```yaml
computer:
  enabled: true
  # screenshot_after_action: true  # screenshot after every action (default)
  # max_long_edge: 1568            # longest screenshot edge sent to the model
  # settle_ms: 600                 # pause before that screenshot
  # cua:                          # drive a remote machine instead (below)
  #   endpoint: http://127.0.0.1:3211
  #   token_env: CUA_ENV_TOKEN
browser:
  enabled: true
  # executable: /path/to/chrome    # default: auto-detect
  # headless: false                # default: headed when a display exists
  # profile_dir: ~/.mira/browser/profile
```

`enabled`, `executable` and `profile_dir` are only read from the
global file. A repo's `.mira/config.yaml` can tune the other knobs but
can't switch these tools on or point them at a different binary.

Run `mira doctor --computer --browser` to check readiness.

## Platform support

| Platform | `computer` | Needs |
| --- | --- | --- |
| macOS | ✅ | **Screen Recording** and **Accessibility** granted to your terminal app (System Settings → Privacy & Security). `mira doctor --computer` checks both. |
| Linux, X11 | ✅ | `xdotool`, plus one of `maim`, ImageMagick (`import`) or `xwd` (x11-apps). |
| Linux, Wayland | ❌ | Not supported yet. Use an X11 session, or point `DISPLAY` at an Xvfb/Xephyr display. |
| Windows | ❌ | Not yet. |

`browser` works anywhere a Chromium-family browser is installed.

## Driving a remote machine (cua)

Point `computer.cua.endpoint` at a
[cua-spacesd](https://github.com/trycua/cua) daemon and `computer`
drives that machine instead of this one. The tool surface is
identical — same actions, same coordinates, same policy rules — so the
only thing that changes is whose screen it is.

```yaml
# ~/.mira/mira.yaml — global file only
computer:
  enabled: true
  cua:
    endpoint: http://192.168.1.20:3211
    token_env: CUA_ENV_TOKEN   # default; a token is required off-loopback
```

Run the daemon on the target machine with
[`cua host setup`](https://cua.ai/docs) (or `cua-spacesd serve` by
hand), then check it:

```sh
mira doctor --computer
```

```
✔ computer · cua endpoint      http://192.168.1.20:3211 (reachable)
✔ computer · cua daemon        cua-spacesd 0.5.3 · cua.env.v1 1·8 on studio-mac
✔ computer · background input  supported
✔ computer · display           2560x1440 logical (2560x1440 physical)
```

### Delivery: who sees the pointer

This is the setting that decides what the agent's input looks like
from where you are sitting.

| `--cua-delivery` | Behaviour |
| --- | --- |
| `auto` (default) | Deliver in the background where the guest allows it; fall back to foreground when it does not. |
| `background` | Never move the pointer or change focus. **Fails** rather than falling back, so an agent cannot quietly start taking over. |
| `foreground` | Always deliver through the global input stream — the real pointer and focus move, same as the local backends. |

```sh
mira --computer --cua-delivery background
```

Or `MIRA_CUA_DELIVERY=background` for one shell session. It is a
flag rather than a setting in `mira.yaml` on purpose: whether an agent
may take your pointer is a decision about right now, not about the
project, and a repo must not be able to grant it.

Under `background`, the guest reports a *limitation* rather than
failing when it can only manage part of the surface — for example
"pointer/key down-up pairs are not available; use click, drag, press
or hotkey" — which `mira doctor` prints.

### Notes

- **`cua` is global-file only.** A repo's `.mira/config.yaml` can
  tune `settle_ms` and `max_long_edge`, but it cannot redirect the
  `computer` tool at a different machine — same rule as `enabled`.
- **No silent fallback.** If the daemon is unreachable, `computer` is
  disabled with a warning naming `computer.cua.endpoint`. It does not
  quietly fall back to your local desktop, because "the remote box was
  down so the agent clicked on my real screen" is not a fallback
  anyone wants.
- **The primary display.** `computer` screenshots and drives the
  guest's primary display. A cursor parked on a second monitor reads
  back clamped to the primary's edge rather than as a coordinate
  outside the screenshot.
- **Coordinates are unchanged.** Model-space pixels, Retina scaling
  and out-of-bounds refusal all work the same as on a local desktop;
  see [How it works](#how-it-works).
- **Licensing.** Mira depends on cua-spacesd's `cua.env.v1` *contract*
  (MIT, vendored under `crates/mira-computer/proto`). The daemon
  itself is source-available under FSL-1.1-MIT and is not vendored.

## Permissions

Both tools use the policy engine. The rule target is `verb` or
`verb:detail`:

```yaml
permissions:
  allow:
    - "Computer(screenshot)"                 # all screenshots
    - "Computer(click:*)"                    # every *click verb
    - "Computer(key:ctrl+*)"                 # ctrl chords only
    - "Browser(navigate:https://github.com/*)"
  deny:
    - "Computer(type:*password*)"
    - "Browser(evaluate)"                    # no page JavaScript
```

`*` matches any run of characters, `/` included. A bare verb such as
`Computer(type)` covers every detail. The verb `click` is shorthand
for all click verbs.

Defaults when no rule matches:

| | plan / manual | auto | edit / yolo |
| --- | --- | --- | --- |
| `computer` screenshot, zoom, cursor_position, wait | ask | allow | allow |
| `computer` anything else | ask | ask | **ask** |
| `browser` snapshot, screenshot, list_tabs, wait | allow | allow | allow |
| `browser` anything else | ask | ask | allow |

Desktop input **always asks, even in `yolo`**, unless an explicit
`Computer(...)` allow rule covers it: a stray click can land on
anything on your screen. Picking "allow for session" on an approval
allows that whole verb (`Computer(left_click)`), not one pixel. For
`browser navigate` it allows the whole site
(`Browser(navigate:https://site/*)`).

Subagents never inherit `computer` or `browser`.

## How it works

- **Coordinates.** Screenshots are downscaled to fit provider image
  limits (1568 px long edge, ~1.15 MP), and the model's coordinates are
  in that screenshot's pixels. Mira maps them back to screen
  coordinates, which handles Retina/HiDPI. Clicks outside the last
  screenshot are refused, not clamped.
- **Token cost.** Only the three most recent screenshots stay in the
  conversation. Older ones are replaced with a short note.
- **Browser snapshots.** `navigate`, `click`, `type` and the other
  actions that change the page return a text snapshot that lists
  interactive elements as `[e12] button "Sign in"`. The model targets
  them by ref, which costs far fewer tokens than a screenshot per step.
  `screenshot` and coordinate clicks are there for canvas-heavy pages.
- **Providers.** Screenshots go to Anthropic as native image blocks
  inside `tool_result`. OpenAI-compatible providers get them in a user
  message right after the tool results. The model needs vision.

## Known gaps

- No native passthrough of Anthropic's `computer_*` tool types yet.
  Mira sends a regular function tool that uses the same action
  vocabulary, so it works with any vision model.
- The TUI approval card doesn't preview the screenshot yet. The web UI
  shows screenshots inline in the tool card.
- No Wayland or Windows backend. The backend trait
  (`mira_computer::ComputerBackend`) is where they would plug in. The
  cua backend does reach a Windows or Linux guest, but Mira itself
  still runs on macOS or Linux.
- The cua backend implements `ComputerService` only. cua-spacesd also
  serves processes, files, windows and streaming, which would be the
  natural way to close the "agent on a remote box can't run the code
  it writes" gap; none of it is wired up here.
