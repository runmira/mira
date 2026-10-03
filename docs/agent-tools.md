# Mira's tools for external agents

Status: **done** (phases 0–6) · Tracking PR: `agent-tools` → `windows`

## Why

External agents (Claude Code, Codex, OpenCode, Gemini, Grok…) run inside
Mira, but until recently they couldn't use what Mira has: the browser the
user watches in the browser pane, the Processes window, the device preview.
When asked to "check the page", they launch their own browser or give up —
they struggle. The fix is one tool server that every agent gets, in the
form each agent understands, with tools shaped so a model uses them well.

## Where each agent stands

| Agent | How Mira drives it | Mira tools today | Gap |
| --- | --- | --- | --- |
| Claude Code | native `claude -p` (stream-json) | `browser` + background processes, via `--mcp-config` | one multipurpose `browser` tool; every call prompts |
| OpenCode | ACP (`opencode acp`) | same, via `session/new` `mcpServers` (HTTP) | one multipurpose `browser` tool |
| Gemini / Grok / other ACP | ACP | same — over HTTP, or through `mira mcp-bridge` (stdio) for agents that don't take HTTP | — |
| Codex | `codex app-server` (JSON-RPC) | browser + processes, via `thread/start` `config.mcp_servers` | not yet tried against a live Codex |


## Target design

### One tool server, per chat

`POST /mcp`, authorized per agent session (see Security). Every tool
acts on that chat: background processes land in its Processes window; the
browser is the one in its pane.

### Injection per agent

| Agent | Mechanism |
| --- | --- |
| Claude Code | `--mcp-config` (HTTP) + `--allowedTools` for read-only Mira tools |
| ACP, HTTP-capable | `session/new` `mcpServers: [{ type: "http", … }]` |
| ACP, stdio-only | `mcpServers: [{ type: "stdio", command: "mira", args: ["mcp-bridge", <url>] }]` |
| Codex | `thread/start` / `thread/resume` `config.mcp_servers.mira.url` |

### Tool surface

Same tools for every agent, with the specs Mira's own model uses where they
exist.

**Browser** (split from today's single `browser` tool):

| Tool | Does |
| --- | --- |
| `browser_open` | open a URL (starts the browser if needed) |
| `browser_snapshot` | page title, URL, readable text and interactive elements with refs |
| `browser_screenshot` | PNG of the viewport |
| `browser_click` | click an element (ref or text) |
| `browser_type` | type into a field |
| `browser_press` | press a key |
| `browser_scroll` | scroll the page or an element |
| `browser_evaluate` | run JS, return a JSON value |
| `browser_wait_for` | wait for text / an element / navigation |
| `browser_back` | history back |

The single `browser` tool stays for Mira's own model (it's tuned for it);
the agent server exposes the split set.

**Processes**: `run_background`, `read_output`, `kill_background` (done).

**Later**: device preview (`preview_devices` screenshot at phone/tablet
sizes), the chat's task list, and orchestration (`delegate_task` to another
engine — "have Codex review what Claude wrote").

### The delegation view

`delegate_task` is a hand-off, not an ordinary tool call, so the UI gives it
its own card rather than a generic "Handed off …" row. The row carries the
two ends of the gesture (Mira's orb → the receiving engine's mark), who took
the task, a live stage, and an elapsed clock; opening it shows the brief,
what the helper may touch (read and search only), a three-stop progress track,
a live **activity log** of the steps the child is taking, and the answer as
prose. The card is in `frontend/src/components/DelegateCard.tsx` and is used
by both the main transcript and the subagent panel. It matches on
`delegate_task` only — an external agent's own `Task`/`Agent` call is a
subagent inside that agent and keeps folding into the ordinary tool rows.

The activity log is fed by `delegate_progress` frames: `delegate.rs` drains
the child's own `HarnessEvent` stream (tool starts and warnings on the Mira
engine; `AcpToolCall` frames on an agent engine) and re-broadcasts each as a
step on the parent's channel, tagged with the parent's call id. The snippets
come from the same verb/target vocabulary the tool rows use, so a delegated
step reads exactly like a step Mira ran itself (`Read src/lib.rs`,
`Searched for "bind_any"`, `Ran cargo test`).

### Approval

| Gate | Who asks | Used for |
| --- | --- | --- |
| `agent` | the agent's own permission gate, which is Mira's approval card | Claude Code |
| `mira` | Mira's approval card, before anything that runs or stops a command | ACP agents, Codex (they may call MCP tools without asking) |

Read-only tools (snapshot, screenshot, read_output) never prompt. Browser
actions follow the browser tool's own policy, as for Mira's model.

### Security

- Loopback only (the server binds 127.0.0.1).
- **Per-session tokens** in an `Authorization: Bearer` header (never the
  URL, so they don't end up in agent configs or logs). A token names one
  chat and its gate, is reused by every agent started in that chat, and
  ends when the chat is deleted or after 12 h unused. The stdio bridge reads it from
  `MIRA_MCP_TOKEN`, not argv. `/mcp` without a live token is a 401.

## Phases

- [x] **0 · Tool server for ACP agents** — `mcpServers` at `session/new`;
      per-chat URL; background-process tools; Mira-side approval gate.
      (#97)
- [x] **1 · Codex** — `config.mcp_servers.mira` on `thread/start` and
      `thread/resume`, gate `mira`.
      *Done when* Codex opens a page in Mira's browser and its background
      command shows in Processes after Mira's approval.
- [x] **2 · Granular browser tools** — the split set above on the agent
      server, sharing Mira's browser session.
      *Done when* OpenCode and Codex complete "open localhost:5173, click
      Sign in, screenshot" without retries.
- [x] **3 · stdio bridge** — `mira mcp-bridge <url>`; ACP agents without
      HTTP MCP get the tools through it.
      *Done when* an agent advertising `mcpCapabilities.http: false` lists
      Mira's tools.
- [x] **4 · Fewer prompts for Claude Code** — `--allowedTools` for the
      read-only Mira tools.
- [x] **5 · Per-session tokens** — header auth, idle expiry, revoke on stop.
- [x] **6 · Device preview + orchestration tools** — `browser_devices`; `delegate_task` to Mira's model or an installed agent, read-only, in a hidden child chat.

## Testing

- Unit: injection params per agent (`session/new`, `thread/start`, launch
  args), tool listing per session, gate behaviour (denied → error result,
  nothing runs), token checks.
- Live, against a test server (scripted over the WebSocket API):
  OpenCode and Codex each open a page, read its heading, start a
  background server that appears in Processes after approval.

## Open questions

- Should the agent server also expose Mira's `ask_user` / plan cards as
  tools for agents that have no question tool of their own?
- Per-agent tool allow-lists in Settings, or all-or-nothing?
- Name the server `mira` everywhere (tools show as `mcp__mira__…`), or
  something more descriptive?
