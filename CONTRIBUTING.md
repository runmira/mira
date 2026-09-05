# Contributing to Mira

Thanks for considering it. Mira is early — the surface is small enough
that most contributions land as focused PRs rather than sweeping refactors.

## Build

```bash
git clone https://github.com/runmira/mira && cd mira
cargo build --workspace
```

Requires Rust 1.88+ (see `rust-toolchain.toml`).

## Running locally

Set an OpenAI-compatible endpoint and run the CLI:

```bash
export MIRA_API_KEY=...          # e.g. an OpenRouter key
export MIRA_BASE_URL=https://openrouter.ai/api/v1
export MIRA_MODEL=google/gemini-2.5-flash
cargo run -p mira-cli -- --mode manual --max-tokens 4000
```

## Before you open a PR

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

CI runs the same checks. Green on your machine → green on CI.

## Where things live

| Crate | Job |
| --- | --- |
| `mira-core` | Shared types (messages, tool calls, IDs). Cheap to depend on. |
| `mira-ai` | `ChatProvider` trait + OpenAI-compatible streaming client. |
| `mira-tools` | `Tool` trait, registry, built-ins (read, write, edit, bash, grep, rustfmt). |
| `mira-policy` | Permission modes + rule DSL. |
| `mira-sandbox` | Command exec wrapper. |
| `mira-harness` | The agent loop. |
| `mira-cli` | Terminal entrypoint. |

## Adding a tool

Implement `mira_tools::Tool`, then register it. See
`crates/mira-tools/src/builtin/read.rs` for the smallest canonical example.
Prefer reusing `Action::Bash` for shell-based tools — don't add a new
`Action` variant unless the policy engine genuinely needs it.

## Adding a provider

Implement `mira_ai::ChatProvider` in a new module under `crates/mira-ai/`.
The OpenAI-compatible adapter covers most endpoints — add a native one only
if you need features (tools, thinking, caching) that the compat layer
can't express.

## Style

- No unwraps outside tests.
- Errors: `thiserror` in libs, `anyhow` in bins.
- Doc-comment the *why* on non-obvious code. Skip the *what* — good names
  handle that.
- One responsibility per crate. If you find yourself adding a dep to
  `mira-core`, stop and think.

## Reporting a bug

Open an issue with the failing input, the model + provider, and the trace
from `RUST_LOG=mira=debug cargo run ...`.

## Security

Report vulnerabilities privately — see [`SECURITY.md`](./SECURITY.md).
