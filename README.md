# Mira

**An open-source coding agent you run yourself, with the model you choose.**

[![CI](https://github.com/runmira/mira/actions/workflows/ci.yml/badge.svg)](https://github.com/runmira/mira/actions/workflows/ci.yml)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](./LICENSE)

Mira is a terminal agent that reads your code, answers questions, edits
files, and runs commands. It works with any OpenAI-compatible provider —
OpenRouter, OpenAI, Anthropic (compat), Groq, or a model on your own
machine.

Three ideas shape it:

- **You own the whole thing.** Your key, your model, your machine.
  Sessions and configuration live as plain files on disk. No hosted
  control plane, no vector database, no telemetry.
- **A harness, not a prompt.** Tools, permissions, and history are shared
  infrastructure. New capabilities are additions on top of the loop — not
  new agents that re-scaffold everything.
- **Safety is a policy layer.** A small DSL (`Bash(cargo test:*)`,
  `Edit(src/**)`) decides what the model may do without asking, what needs
  approval, and what's off-limits.

> **Status: early, building in the open.** Chat, tool use, streaming,
> permissions, and a terminal REPL are working. Editor extensions, memory,
> and a TUI are next. Expect rough edges.

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

Point Mira at any OpenAI-compatible endpoint:

```bash
export MIRA_API_KEY=sk-or-v1-...                 # e.g. an OpenRouter key
export MIRA_BASE_URL=https://openrouter.ai/api/v1
export MIRA_MODEL=google/gemini-2.5-flash        # cheap, fast, good at code

cd your-repo
mira --mode manual --max-tokens 4000
```

Then talk to it:

```text
> what does the sandbox crate do?
> add a test for the truncate function
> run cargo test
```

Mira reads files, searches the repo (via `ripgrep`), runs commands, and
edits code when you let it. Outside a repo it still works — just less to
look at.

## Permission modes

One flag decides how freely the agent edits. Change it with `--mode`.

| Mode | What it does |
| --- | --- |
| `plan` | Reads and searches only. Never edits, never runs a command. |
| `manual` | Asks before every edit and command. (default) |
| `auto` | Auto-approves file changes; asks before commands. |
| `edit` | Auto-approves everything unless a rule blocks it. |
| `yolo` | No gating whatsoever. Use with care. |

Rules can override the mode:

```yaml
permissions:
  allow: ["Bash(cargo test:*)", "Edit(src/**)"]
  ask:   ["Edit(migrations/**)"]
  deny:  ["Bash(rm:*)"]
```

(Config file wiring lands in the next release — the DSL and engine already
work, see [`crates/mira-policy`](./crates/mira-policy).)

## What's inside

Mira is a Cargo workspace. Each crate has one job.

| Crate | Job |
| --- | --- |
| [`mira-core`](./crates/mira-core) | Shared vocabulary — messages, tool calls, IDs, errors. |
| [`mira-ai`](./crates/mira-ai) | `ChatProvider` trait + OpenAI-compatible streaming client. |
| [`mira-tools`](./crates/mira-tools) | `Tool` trait, registry, and built-ins (read/write/edit/bash/grep/rustfmt). |
| [`mira-policy`](./crates/mira-policy) | Permission modes + rule DSL. |
| [`mira-sandbox`](./crates/mira-sandbox) | Command execution wrapper. Landlock/seatbelt slot in here later. |
| [`mira-harness`](./crates/mira-harness) | The agent loop — turn state, tool dispatch, streaming events. |
| [`mira-cli`](./crates/mira-cli) | Terminal entrypoint. |

Adding a new tool is a `Tool` impl and one `registry.register()` line —
see [`crates/mira-tools/src/builtin/read.rs`](./crates/mira-tools/src/builtin/read.rs)
for the smallest example. Adding a new provider is a `ChatProvider` impl.

## Roadmap

- [x] Streaming chat + tool use
- [x] Permission modes + rule DSL
- [x] OpenAI-compatible provider
- [ ] TUI (ratatui) — approvals, diffs, mode picker
- [ ] Session persistence + `--resume`
- [ ] `MIRA.md` auto-loaded memory
- [ ] Native Anthropic + Bedrock adapters
- [ ] MCP client
- [ ] Editor extension (VS Code, then Zed via ACP)
- [ ] Sandboxing: `landlock` (Linux), `sandbox-exec` (macOS)

## Contributing

Read [`CONTRIBUTING.md`](./CONTRIBUTING.md). Short version: `cargo fmt`,
`cargo clippy -- -D warnings`, `cargo test --workspace`, then open a PR.

## License

Apache-2.0. See [`LICENSE`](./LICENSE).
