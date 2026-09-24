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
  (`mira_computer::ComputerBackend`) is where they, or a VM-sandboxed
  backend, would plug in.
