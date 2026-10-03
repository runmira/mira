# Mira's tools for external agents

Status: **in progress** · Tracking PR: `agent-tools` → `windows`

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
| Gemini / Grok / other ACP | ACP | same, **only if** the agent takes HTTP MCP servers | agents with stdio-only MCP get nothing |
| Codex | `codex app-server` (JSON-RPC) | **none** | no MCP server passed at `thread/start` |

## How T3 Code does it (reference)

From [pingdotgg/t3code](https://github.com/pingdotgg/t3code):

- **One app-owned MCP endpoint** (`127.0.0.1:<port>/mcp`, server key
  `t3-code`), authenticated per provider session with a short-lived bearer
  token that expires when idle and is revoked when the session ends.
- **Injected natively per agent**
  - Codex: per thread, in `thread/start`'s `config.mcp_servers.t3-code`
    (`url` + `http_headers.Authorization`).
  - Claude: an `http` entry in `mcpServers`, plus `allowedTools:
    ["mcp__t3-code__*"]` so its own tools don't prompt.
  - ACP: `mcpServers` on `session/new` / `load` / `fork`.
  - Agents without HTTP MCP: `t3 acp-mcp-bridge`, a stdio MCP server that
    relays each JSON-RPC line to the HTTP endpoint.
- **Granular browser tools** ("preview" toolkit): `preview_open`,
  `preview_navigate`, `preview_click`, `preview_type`, `preview_press`,
  `preview_scroll`, `preview_snapshot`, `preview_evaluate`,
  `preview_wait_for`, `preview_resize`, recording… — one verb per tool.
- **Orchestration tools**: delegate a task to a sub-agent on any provider,
  manage threads.

## Target design

### One tool server, per chat

`/mcp/<server-token>?session=<chat>&gate=<agent|mira>` today. Every tool
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

### Approval

| Gate | Who asks | Used for |
| --- | --- | --- |
| `agent` | the agent's own permission gate, which is Mira's approval card | Claude Code |
| `mira` | Mira's approval card, before anything that runs or stops a command | ACP agents, Codex (they may call MCP tools without asking) |

Read-only tools (snapshot, screenshot, read_output) never prompt. Browser
actions follow the browser tool's own policy, as for Mira's model.

### Security

- Loopback only (the server binds 127.0.0.1).
- Today: one process-wide unguessable token in the URL.
- Target: **per-session tokens** in an `Authorization` header (not the URL,
  so they don't end up in agent logs), expiring when idle and revoked when
  the agent stops. Required before remote control exposes the server.

## Phases

- [x] **0 · Tool server for ACP agents** — `mcpServers` at `session/new`;
      per-chat URL; background-process tools; Mira-side approval gate.
      (#97)
- [ ] **1 · Codex** — `config.mcp_servers.mira` on `thread/start` and
      `thread/resume`, gate `mira`.
      *Done when* Codex opens a page in Mira's browser and its background
      command shows in Processes after Mira's approval.
- [ ] **2 · Granular browser tools** — the split set above on the agent
      server, sharing Mira's browser session.
      *Done when* OpenCode and Codex complete "open localhost:5173, click
      Sign in, screenshot" without retries.
- [ ] **3 · stdio bridge** — `mira mcp-bridge <url>`; ACP agents without
      HTTP MCP get the tools through it.
      *Done when* an agent advertising `mcpCapabilities.http: false` lists
      Mira's tools.
- [ ] **4 · Fewer prompts for Claude Code** — `--allowedTools` for the
      read-only Mira tools.
- [ ] **5 · Per-session tokens** — header auth, idle expiry, revoke on stop.
- [ ] **6 · Device preview + orchestration tools** (separate PRs).

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
