# Mira

**An open-source coding agent that runs on your machine, with the model you choose.**

[![CI](https://github.com/runmira/mira/actions/workflows/ci.yml/badge.svg)](https://github.com/runmira/mira/actions/workflows/ci.yml)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](./LICENSE)

Mira reads your code, edits files, runs commands, reviews diffs, and
delegates subtasks — from a terminal or a browser. It talks to any
OpenAI-compatible provider (OpenRouter, OpenAI, Anthropic-compat, Groq,
DeepSeek, or a local runtime). Sessions and configuration live as plain
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
> subagents, code review, memory, plan/undo, MCP, and a web UI are
> working. A pull-request panel and worktree-isolated write agents are
> the latest additions. Expect the surface to keep moving.

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

**Terminal (TUI):**

```bash
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

Then talk to it:

```text
> what does the sandbox crate do?
> add a test for the truncate function
> run cargo test
> review my current branch
> delegate: read every file that references WsApprover and summarize
```

## What Mira can do

### Tools

Built-in: `read_file`, `write_file`, `edit_file`, `bash`, `grep`, `glob`,
`find_symbol`, `find_references`, `find_callers`, `git_status` /
`git_diff` / `git_log` / `git_commit`, `rustfmt`, `web_fetch`,
`web_search`, and a set of `memory_*` tools.
Every call goes through the permission layer.

Extensible: any MCP server (`stdio` or `http`) registers its own tools —
manage them from the Plugins panel in the web UI or `mcp_servers:` in
`mira.yaml`.

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

## What's inside

Mira is a Cargo workspace. Each crate has one job.

| Crate | Job |
| --- | --- |
| [`mira-core`](./crates/mira-core) | Shared vocabulary — messages, tool calls, IDs, errors. |
| [`mira-ai`](./crates/mira-ai) | `ChatProvider` trait + OpenAI-compatible streaming client. |
| [`mira-tools`](./crates/mira-tools) | `Tool` trait, registry, and built-ins (files, bash, grep, git, memory, web). |
| [`mira-agents`](./crates/mira-agents) | Subagent type registry (markdown + YAML frontmatter loader). |
| [`mira-policy`](./crates/mira-policy) | Permission modes + rule DSL. |
| [`mira-sandbox`](./crates/mira-sandbox) | Command execution wrapper. Landlock/seatbelt slot in here later. |
| [`mira-harness`](./crates/mira-harness) | The agent loop — turn state, tool dispatch, streaming events, session persistence. |
| [`mira-memory`](./crates/mira-memory) | `MIRA.md` loader + episodic memory store. |
| [`mira-review`](./crates/mira-review) | Two-stage code review (generate + hostile re-verify). |
| [`mira-config`](./crates/mira-config) | `mira.yaml` loader — provider, MCP servers, permissions. |
| [`mira-cli`](./crates/mira-cli) | Terminal entrypoint (TUI + `mira review`, `mira serve`, …). |
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
- [x] TUI (ratatui) — approvals, diffs, mode picker
- [x] Session persistence + `--resume`
- [x] Web UI (sidebar, chat, subagent panel, review panel, PR panel, plugins)
- [x] `MIRA.md` auto-loaded memory + `/remember`
- [x] Interactive plan tool + undo + apply-verify loop
- [x] MCP client (stdio + http)
- [x] Subagents: named types, parallel dispatch, approval routing, worktree isolation
- [x] Two-stage code review (`mira review` + Review panel)
- [x] Pull-request panel (browse / review / merge GitHub PRs)
- [x] Token usage + cost tracking + prompt caching
- [ ] Subagent streaming intermediate summaries + `type: "auto"` router
- [ ] Native Anthropic + Bedrock adapters
- [ ] Editor extension (VS Code, then Zed via ACP)
- [ ] Sandboxing: `landlock` (Linux), `sandbox-exec` (macOS)

## Contributing

Read [`CONTRIBUTING.md`](./CONTRIBUTING.md). Short version: `cargo fmt`,
`cargo clippy -- -D warnings`, `cargo test --workspace`, then open a PR.

## License

Apache-2.0. See [`LICENSE`](./LICENSE).
