# Mira

**An open-source coding agent that runs on your machine, with the model you choose.**

[![CI](https://github.com/runmira/mira/actions/workflows/ci.yml/badge.svg)](https://github.com/runmira/mira/actions/workflows/ci.yml)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](./LICENSE)

Mira reads your code, edits files, runs commands, reviews diffs, and
delegates subtasks — from a terminal, a browser, or your editor. It
talks to Anthropic and Amazon Bedrock natively, and to any
OpenAI-compatible provider (OpenRouter, OpenAI, Groq, DeepSeek, or a
local runtime). Sessions and configuration live as plain
files on disk. No hosted control plane, no vector database, no telemetry.

Three ideas shape it:

- **You own the whole thing.** Your key, your model, your machine.
  Everything is a file you can read.
- **A harness, not a prompt.** Tools, permissions, history, and subagents
  are shared infrastructure. New capabilities are additions on top of the
  loop — not new agents that re-scaffold everything.
- **Safety is a policy layer.** A small DSL (`Bash(cargo test:*)`,
  `Edit(src/**)`) decides what the model may do without asking, what needs
  approval, and what's off-limits.

> **Status: alpha, building in the open.** Chat, tool use, permissions,
> subagents, code review, memory, plan/undo, MCP, plugins and hooks, a
> web UI, a desktop app, and editor support (VS Code, Zed) are working.
> Expect the surface to keep moving.

---

## Install

**Homebrew** (macOS and Linux):

```bash
brew install runmira/tap/mira
```

**Install script** (macOS and Linux, no Homebrew required):

```bash
curl -fsSL https://raw.githubusercontent.com/runmira/mira/main/install.sh | bash
```

**From source** — requires Rust 1.88+ (see [`rust-toolchain.toml`](./rust-toolchain.toml)):

```bash
git clone https://github.com/runmira/mira && cd mira
cargo install --path crates/mira-cli
```

## Quickstart

Two ways to talk to Mira. Both share the same harness, the same session
store (`~/.mira/sessions/`), and the same config (`~/.mira/mira.yaml`) —
swap between them freely.

**Sign in (recommended):**

```bash
mira login openrouter          # or: mira login openai
```

Opens your browser, PKCE OAuth round-trip, writes the resulting key to
`~/.mira/mira.yaml`. `openai` is "Sign in with ChatGPT" — Mira mints and
refreshes API keys against your ChatGPT account and stashes the token
bundle in `~/.mira/auth/openai.json` (mode 0600). Pass `--no-browser`
(or set `MIRA_NO_BROWSER=1`) for headless / SSH sessions to just print
the URL. `mira auth status` shows what's signed in; `mira logout <p>`
forgets it.

**Terminal (TUI):**

```bash
# After `mira login`, or manually:
export MIRA_API_KEY=sk-or-v1-...                 # e.g. an OpenRouter key
export MIRA_BASE_URL=https://openrouter.ai/api/v1
export MIRA_MODEL=google/gemini-2.5-flash

cd your-repo
mira --mode manual
```

**Browser:**

```bash
mira serve --open
```

**Desktop app:** the same UI in a native window, built from
[`apps/desktop`](./apps/desktop). See [docs/desktop.md](./docs/desktop.md).

**Editors:** the [VS Code extension](./extensions/vscode) adds `@mira`
to the Chat sidebar. Zed (and other editors that speak the
[Agent Client Protocol](https://agentclientprotocol.com)) run
`mira acp`; add this to Zed's `settings.json`:

```json
{ "agent_servers": { "Mira": { "command": "mira", "args": ["acp"] } } }
```

Then talk to it:

```text
> what does the sandbox crate do?
> add a test for the truncate function
> run cargo test
> review my current branch
> delegate: read every file that references WsApprover and summarize
```

## What Mira can do

### Providers

Any OpenAI-compatible endpoint works; `anthropic` and `bedrock` get
native adapters (prompt caching, extended thinking, Bedrock's Converse
API). For Bedrock, Mira signs requests with your usual AWS credentials
(`AWS_ACCESS_KEY_ID` / `AWS_PROFILE`, `~/.aws/credentials`) or uses a
Bedrock API key (`AWS_BEARER_TOKEN_BEDROCK`):

```yaml
default_provider: bedrock
default_model: us.anthropic.claude-sonnet-4-5-20250929-v1:0
providers:
  bedrock:
    # Region in the URL; without one, AWS_REGION or ~/.aws/config decides.
    base_url: https://bedrock-runtime.us-west-2.amazonaws.com
```

`mira models` lists the models and inference profiles your account can use.

### Tools

Built-in: `read_file`, `write_file`, `edit_file`, `bash`, `grep`, `glob`,
`find_symbol`, `find_references`, `find_callers`, `git_status` /
`git_diff` / `git_log` / `git_commit`, `rustfmt`, `web_fetch`,
`web_search`, and a set of `memory_*` tools.
Every call goes through the permission layer.

Extensible: any MCP server (`stdio`, `http` or `sse`, with OAuth
sign-in) adds its own tools, and Claude Code plugins install as they
are: commands, agents, skills, hooks and MCP servers. Manage both from
the Plugins page in the web UI or with `mira mcp` / `mira plugin`. See
[docs/mcp-and-plugins.md](./docs/mcp-and-plugins.md).

### Subagents

Cold-context child sessions delegated by name — same shape as Claude
Code's Agent tool, driven by markdown files with YAML frontmatter
(bundled built-ins, plus user overrides in `~/.mira/agents/*.md` and
per-repo overrides in `<cwd>/.mira/agents/*.md`).

Built-in roster:

| Type | Role |
| --- | --- |
| `explore` | Read-only investigation. Returns a JSON summary with cited findings. |
| `cartographer` | Read-only architecture mapper. |
| `reviewer` | Adversarial correctness reviewer (read-only). |
| `sentinel` | Mission-creep auditor for a delegated diff. |
| `coder` | Implements a small, well-scoped change. Worktree-isolated. |
| `documenter` | Updates docs to match reality. Worktree-isolated. |

Round-tripped features:

- **Parallel dispatch** — multiple `agent` calls in one round run
  concurrently, so a fan-out of research questions finishes in one
  wall-clock trip.
- **Approval routing** — a write-capable subagent's `bash rm` pops the
  same modal the parent's own commands would; there's no auto-approval
  cliff hidden behind delegation.
- **Structured output** — types can pin a JSON schema for their final
  message; the parent gets typed data back in `ToolResult.data`.
- **Worktree isolation** — write-capable types run in an ephemeral
  `git worktree` off HEAD, their changes get merged back after
  completion, and the temporary directory is torn down. Parallel
  writers no longer stomp on each other.
- **Stop cascades** — pressing Stop on the parent halts every in-flight
  subagent (and grand-subagent) in one action.
- **Live progress** — a subagent calls the `progress` tool during long
  investigations to yield a one-line status; the parent's UI renders it
  as a chip in the SubagentPanel so a multi-minute delegation stops
  looking opaque.
- **Auto-routing** — pass `type: "auto"` and a tiny classifier picks
  the best specialist from the roster for the task. Useful when the
  parent isn't sure which subagent applies.
- **Shared scratchpad** — peer subagents running in parallel on the
  same session share a pad through `scratchpad_note` / `scratchpad_read`,
  so a fan-out of researchers can coordinate mid-flight instead of
  duplicating each other's work.

### Code review

Two-stage `mira review`: stage 1 generates findings with high recall;
stage 2 re-verifies each one against the actual source with a hostile
prompt, dropping any it can't defend. Runs on a working-tree diff, a
committed range, a raw diff, or a GitHub PR (with `GITHUB_TOKEN`
configured). Same engine drives the web UI's Review panel.

### Pull requests

Browse open PRs across every repo Mira knows about, read the diff, leave
comments, submit a review, or merge — through the web UI's Pull Requests
tab. Powered by the GitHub REST API; token is stored alongside your
other keys in `mira.yaml`.

### Plan / Undo / Verify

- `/plan` — model proposes a step list; you edit / reorder / approve /
  cancel before execution.
- `/undo [N]` — reverts the last N file writes tracked by `FileGuard`.
  Non-destructive; snapshots live under `.mira/.undo/<session>/`.
- Apply-verify loop — after each turn's writes, Mira runs the relevant
  build check (`cargo check`, `tsc`, `ruff`, `go build`) and feeds
  failures back into the next turn until it fixes them or hits the retry
  cap.

### Evals

`mira eval` batch-runs regression tasks against the current provider /
model and prints a pass/fail summary. Tasks live in `evals/tasks/*.yaml`
and take one of two verifier shapes: `expect_grep` (regex against the
final assistant message) or `verify` (shell command that must exit 0 in
the task's isolated tempdir). Optional `fixture:` copies a seed
directory in before the run. Uses `--json` for CI, `--task <substr>` to
scope to a single task. Costs real API tokens — not run automatically.

### Memory

`~/.mira/MIRA.md` (user-wide) and `<cwd>/.mira/MIRA.md` (per-repo) are
appended to the system prompt at session start. `/remember [--user]
<note>` writes to the right file without leaving the chat.

### Cost + caching

Per-turn token accounting and USD cost surface live in both the TUI
status bar and the web UI's composer. Prompt caching is opt-in per
provider (auto-enabled for Anthropic-compatible endpoints).

## Permission modes

One flag decides how freely the agent edits. Change it with `--mode`.

| Mode | What it does |
| --- | --- |
| `plan` | Reads and searches only. Never edits, never runs a command. |
| `manual` | Asks before every edit and command. (default) |
| `auto` | Auto-approves file changes; asks before commands. |
| `edit` | Auto-approves everything unless a rule blocks it. |
| `yolo` | No gating whatsoever. Use with care. |

Rules override the mode:

```yaml
permissions:
  allow: ["Bash(cargo test:*)", "Edit(src/**)"]
  ask:   ["Edit(migrations/**)"]
  deny:  ["Bash(rm:*)"]
```

### Command sandbox

Commands the agent runs are confined to the repository: they can write
only inside it (plus temp and build caches), can't read credentials
like `~/.ssh` or `~/.aws`, and have no network unless allowed.

| Platform | Sandbox |
| --- | --- |
| macOS | `sandbox-exec` (Seatbelt). Keychain access is blocked too. |
| Linux | bubblewrap when it can run; otherwise Landlock (kernel 5.13+, network blocking from 6.7). |

`mira doctor` shows which one is active. `MIRA_SANDBOX_BACKEND=bwrap|landlock|seatbelt|none`
overrides the choice.

### Computer use and browser

`mira --computer` lets the agent see your screen and drive the mouse and
keyboard. `mira --browser` lets it drive Chrome in a separate Mira
profile. Both are off by default, and desktop input asks for approval
even in `yolo`. See [docs/COMPUTER_USE.md](./docs/COMPUTER_USE.md).

### Cloud tasks

`mira cloud run "<task>"` runs the whole session in an E2B sandbox: clone,
work, verify, push. You come back to a pull request, and can close your
laptop once the command returns. See [docs/cloud-tasks.md](./docs/cloud-tasks.md).

### Remote environments

`/remote-env <name>` in the TUI, or the environment chip in the web UI,
moves the session's file and shell tools into a remote environment: an
E2B microVM, or a scratch copy on this machine. The environment gets a
copy of your worktree, uncommitted edits included. `/remote-env local`
merges the changes back into the same worktree, three-way, so your own
edits survive. Named environments, with their own template, env vars and
setup script, live under `compute:` in `~/.mira/mira.yaml`. See
[docs/remote-compute.md](./docs/remote-compute.md#implementation-notes).

## What's inside

Mira is a Cargo workspace. Each crate has one job.

| Crate | Job |
| --- | --- |
| [`mira-core`](./crates/mira-core) | Shared vocabulary — messages, tool calls, IDs, errors. |
| [`mira-ai`](./crates/mira-ai) | `ChatProvider` trait, OpenAI-compatible client, native Anthropic and Bedrock adapters. |
| [`mira-tools`](./crates/mira-tools) | `Tool` trait, registry, and built-ins (files, bash, grep, git, memory, web). |
| [`mira-agents`](./crates/mira-agents) | Subagent type registry (markdown + YAML frontmatter loader). |
| [`mira-policy`](./crates/mira-policy) | Permission modes + rule DSL. |
| [`mira-sandbox`](./crates/mira-sandbox) | Runs commands in an OS sandbox: Seatbelt (macOS), bubblewrap or Landlock (Linux). |
| [`mira-harness`](./crates/mira-harness) | The agent loop — turn state, tool dispatch, streaming events, session persistence. |
| [`mira-memory`](./crates/mira-memory) | `MIRA.md` loader + episodic memory store. |
| [`mira-review`](./crates/mira-review) | Two-stage code review (generate + hostile re-verify). |
| [`mira-config`](./crates/mira-config) | `mira.yaml` loader — provider, MCP servers, permissions. |
| [`mira-mcp`](./crates/mira-mcp) | MCP client: live server connections, OAuth sign-in, tools, prompts. |
| [`mira-plugins`](./crates/mira-plugins) | Claude Code-compatible plugins and marketplaces, plus hooks. |
| [`mira-skills`](./crates/mira-skills) | Skills: named instruction bundles loaded from markdown. |
| [`mira-auth`](./crates/mira-auth) | OAuth sign-in for providers (OpenRouter, ChatGPT). |
| [`mira-computer`](./crates/mira-computer) | Desktop control for the `computer` tool — screenshots, mouse, keyboard (macOS, X11). |
| [`mira-browser`](./crates/mira-browser) | Chrome DevTools driver for the `browser` tool. |
| [`mira-cloud`](./crates/mira-cloud) | Cloud tasks: headless worker in a sandbox that delivers a pull request. |
| [`mira-compute`](./crates/mira-compute) | Where tools execute: the local worktree, or a named remote environment (scratch copy, E2B microVM) you can switch to mid-session. |
| [`mira-cli`](./crates/mira-cli) | Terminal entrypoint (TUI + `mira review`, `mira serve`, `mira acp`, …). |
| [`mira-server`](./crates/mira-server) | Axum backend + embedded React frontend for the web UI. |

Adding a new tool is a `Tool` impl and one `registry.register()` line —
see [`crates/mira-tools/src/builtin/read.rs`](./crates/mira-tools/src/builtin/read.rs)
for the smallest example. Adding a new provider is a `ChatProvider` impl.
Adding a new subagent type is dropping a markdown file into
`~/.mira/agents/`.

## Roadmap

- [x] Streaming chat + tool use
- [x] Permission modes + rule DSL
- [x] OpenAI-compatible provider
- [x] Native Anthropic and Amazon Bedrock providers
- [x] TUI (ratatui) — approvals, diffs, mode picker
- [x] Session persistence + `--resume`
- [x] Web UI (sidebar, chat, subagent panel, review panel, PR panel, plugins)
- [x] Desktop app
- [x] `MIRA.md` auto-loaded memory + `/remember`
- [x] Interactive plan tool + undo + apply-verify loop
- [x] MCP client (stdio, http, sse) with OAuth, per-tool switches and on-demand tool loading
- [x] Claude Code-compatible plugins, marketplaces and hooks
- [x] Subagents: named types, parallel dispatch, approval routing, worktree isolation
- [x] Subagent streaming intermediate summaries + `type: "auto"` router + shared scratchpad
- [x] Two-stage code review (`mira review` + Review panel)
- [x] Pull-request panel (browse / review / merge GitHub PRs)
- [x] Token usage + cost tracking + prompt caching
- [x] Remote environments (scratch copy, E2B) and cloud tasks
- [x] Editors: VS Code extension, Zed via ACP (`mira acp`)
- [x] Sandboxing: `sandbox-exec` (macOS), bubblewrap and Landlock (Linux)
- [ ] VS Code extension on the Marketplace and Open VSX
- [ ] Scheduled prompts (the web UI's Scheduled view)
- [ ] Command sandbox on Windows

## Contributing

Read [`CONTRIBUTING.md`](./CONTRIBUTING.md). Short version: `cargo fmt`,
`cargo clippy -- -D warnings`, `cargo test --workspace`, then open a PR.

## License

Apache-2.0. See [`LICENSE`](./LICENSE).
