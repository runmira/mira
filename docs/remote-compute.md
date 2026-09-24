# Mira Remote Compute: Architecture Guide

> How to build `mira-computer` — the execution backend that routes Mira's
> tool calls to an isolated remote environment instead of the user's local
> machine.

> **Status (implemented: steps 1–3).** `mira --sandbox local|e2b` works in
> the terminal. See [Implementation notes](#implementation-notes) at the
> end for what shipped and where it differs from this design.

---

## The Problem

Mira today runs every tool call (bash, read_file, write_file, edit_file)
directly on the user's local machine. That is the right default for a
developer who owns their environment. It is the wrong choice for:

- **Safety** — untrusted scripts run as the user's own process.
- **Reproducibility** — "works on my machine" is the environment.
- **Shareability** — another person (or a second Mira instance) cannot
  join the same live workspace without SSH.
- **Parallelism** — running two agents on the same codebase simultaneously
  is destructive.

The `mira-computer` crate is a `ComputeBackend` trait that lets Mira swap
the execution substrate — keep using the same TUI, the same event loop, the
same tool-call wire protocol — but route execution somewhere else.

---

## The Two Canonical Architectures

### Option A — Local-canonical, remote mirror

```
 User's machine
 ├── mira TUI          (all rendering, input)
 ├── tool execution    (bash, file I/O — happens HERE)
 └── ← sync daemon →  remote copy (for sharing/backup)
```

**Who uses this:** Claude Code (default), VS Code Remote.  
**Pros:** Zero network latency per tool call, works offline, no sandboxing infra.  
**Cons:** No isolation from host OS, not parallelisable, no reproducible environment.  
**Right when:** The developer owns the machine and isolation is not a requirement.

### Option B — Remote-canonical, local render

```
 User's machine
 └── mira TUI          (rendering, input, event loop)
          │
          │  tool calls  →  RemoteBackend::exec()
          ↓
 Cloud sandbox (Firecracker microVM)
 ├── /workspace        (the project tree)
 ├── bash / cargo / npm / git  (real processes)
 └── ← stdout/stderr stream ←
```

**Who uses this:** Devin, OpenHands, Replit Agent, all managed sandbox platforms.  
**Pros:** OS-level isolation, reproducible, parallelisable (fork N sandboxes), shareable.  
**Cons:** Network RTT added to every tool call (~50–200 ms). Acceptable because LLM
calls dominate the loop at 1–10 seconds.  
**Right when:** Isolation, reproducibility, or parallel agent runs matter.

**Production systems choose Option B when a security boundary is needed.**
Mira should support both — local stays the default, remote is opt-in via
config or a `--sandbox` flag.

---

## Technology Choice: What Runs the Sandbox?

### The Foundation: Firecracker

Almost every serious sandbox platform runs on **Firecracker**, an
open-source VMM written in Rust by AWS. Each sandbox is a full Linux VM
(its own kernel, memory space, network namespace). VM escape requires
exploiting the hypervisor itself — valued at $250K–$500K bug bounties.

The key performance technique is **snapshot-restore**:

1. Boot the VM once, install all dependencies, call `PauseVM`.
2. Firecracker writes the full memory + rootfs diff to a snapshot file.
3. On the next request, restore from snapshot: copy-on-write memory mapping,
   CPU register restore. Time: **5–28 ms**.
4. Every sandbox shares the base snapshot's memory pages and diverges only
   on writes — no physical copy until a page is touched.

This is how the platforms below get sub-second sandbox boot despite full
Linux kernel isolation.

---

## Provider Comparison

| Provider | VMM | Cold start | Resume | Billing | Best for |
|---|---|---|---|---|---|
| **E2B** | Firecracker | ~717 ms | ~662 ms | Wall-clock/sec | Agent-optimised SDK, pause/resume |
| **Vercel Sandbox** | Firecracker | ~1.8 s | ~3.3 s | Active CPU + provisioned mem | Active-CPU model; Drives for shared deps |
| **Fly Sprites** | Firecracker | 1–12 s cold, <1 s restore | <1 s | Active use only, free idle | Long-lived persistent workspaces |
| **Fly Machines** | Firecracker | ~300 ms | ~300 ms | Per-second | Low-level control, cheapest raw rate |
| **Modal** | gVisor | ~2.4 s | ~2.3 s | Wall-clock/sec | GPU workloads, bursty short sessions |
| **AWS Lambda + EFS** | Firecracker | 2–10 s | N/A | Per 100 ms | Async batch only — NOT interactive |
| **Cloudflare Workers** | V8 isolate | 2–5 ms | N/A | Per request | JS-only, no filesystem — NOT viable |
| **GitHub Codespaces** | Docker | 30–60 s | 10–30 s | Per core-hour | Human-IDE sessions — NOT interactive |

### E2B — Recommended for interactive Mira use

E2B is purpose-built for LLM agent execution. It has the fastest
pause/resume in the benchmark (662 ms), a mature SDK with direct Claude
integrations, and the right primitives for the Mira use case.

```typescript
import { Sandbox } from '@e2b/code-interpreter';

// Create or resume a persistent sandbox
const sandbox = await Sandbox.create();

// Stream bash output — maps directly to Mira's tool_tail events
await sandbox.commands.run('cargo build 2>&1', {
  onStdout: (data) => emit(ToolTailEvent(data)),
  onStderr: (data) => emit(ToolTailEvent(data)),
});

// Persist full state (memory + filesystem) — call on session pause
const pausedId = await sandbox.pause();

// Restore later — deps already installed, in-memory caches warm
const resumed = await Sandbox.resume(pausedId);
```

**Pricing:** ~$0.05/vCPU-hour, wall-clock billing. A 30-min coding session
with a 2 vCPU / 4 GB sandbox costs ~$0.04. Free tier: 100 sandbox-hours/month.

**Limitations:** Wall-clock billing means you pay during LLM inference time.
Pause/resume has a known bug (GitHub #884) with file loss after repeated
cycles — monitor and test.

### Vercel Sandbox + Drives — Recommended if active-CPU billing matters

Vercel bills only for CPU cycles when the process is actually burning CPU.
Time spent waiting for LLM responses costs nothing. For Mira, where the
agent spends 80%+ of session time waiting for Claude, this billing model
is ideal for long sessions.

The **Drives** feature is the most interesting for Mira: a persistent NVMe
volume that can be shared across sandboxes. One sandbox installs
`node_modules` or builds `target/` into a Drive, then all subsequent
sandboxes mount a read-only snapshot of it — eliminating install time from
the hot path entirely.

```typescript
import { Sandbox, Drive } from '@vercel/sandbox';

// Shared dependency cache — populate once, mount everywhere
const depCache = await Drive.getOrCreate({ name: 'cargo-registry' });

// Sandbox with the pre-built registry mounted
const sandbox = await Sandbox.create({
  mounts: { '/usr/local/cargo/registry': depCache.snapshot() }
});

// Subsequent `cargo build` calls skip re-downloading crates
const cmd = await sandbox.runCommand({ cmd: 'cargo', args: ['build'], detached: true });
for await (const log of cmd.logs()) { emit(ToolTailEvent(log.data)); }

await sandbox.stop(); // auto-snapshots filesystem
```

**Pricing:** $0.128/active CPU-hour + $0.021/GB-hour provisioned memory.
Expensive per active CPU-minute but the active-CPU model saves cost over
E2B for LLM-heavy sessions. Drive storage: $0.05/GB-month.

**Limitations:** Slow resume (~3.3 s measured). Region-locked Drives.
Concurrent sandboxes cannot write the same Drive simultaneously.

### Fly Sprites — Recommended for persistent long-running workspaces

A Sprite is a persistent Firecracker microVM with a 100 GB NVMe volume,
**zero idle cost** (you pay only active compute), and checkpoint-restore
under 1 second. Perfect for a Mira session that spans days — the workspace
is always exactly where it was left.

**Pricing:** $0.07/CPU-hour, $0.04/GB-hour memory. 100 GB NVMe storage
included. Cold storage while idle: $0.02/GB-month. No per-Sprite fee.

**Limitation:** Raw Machines API — no Mira-specific SDK. You build the
agent execution layer yourself on top of the Fly REST API.

---

## Dependency Caching Patterns

The slowest part of cold-starting a sandbox is package installation.
These patterns eliminate it from the hot path.

### Pattern 1: Warm snapshot (any Firecracker provider)
```
1. Boot sandbox, run: cargo build / npm install / pip install
2. Snapshot: sandbox.snapshot() → "snap_deps_abc123"
3. All future sandboxes: Sandbox.create({ source: { snapshotId: "snap_deps_abc123" } })
4. Dependencies present from first command. Zero install penalty.
```

### Pattern 2: Shared Drive (Vercel)
```
1. Writer sandbox mounts Drive, runs: pnpm install --store-dir /drive/cache
2. Writer stops. Drive holds the content-addressable package store.
3. Reader sandboxes mount drive.snapshot() at /drive/cache (read-only).
4. pnpm install runs in seconds — store already populated.
```

### Pattern 3: Pause/Resume (E2B)
```
1. First session: sandbox boots cold, installs everything.
2. sandbox.pause() — saves full memory + filesystem state.
3. Next session: Sandbox.resume(id) → 1 second. npm, cargo, pip warm.
```

### Pattern 4: CoW fork for parallel agents (E2B / Replit)
```
1. Snapshot a "ready" sandbox with all deps installed.
2. Fork N sandboxes from it simultaneously for parallel runs.
3. Each fork diverges independently. Base pages are shared (CoW).
4. E2B supports up to 100 concurrent forks from one snapshot.
```

---

## What Other Agents Do

| Agent | Architecture | Notes |
|---|---|---|
| **Claude Code** | Local-canonical | Tools run in user's local shell. E2B sandbox IS the "local" when used inside E2B. |
| **Devin** | Remote-canonical VM | Every session = fresh Devbox VM in the cloud. Parallel instances = parallel VMs. Supports Vercel Sandbox via Outposts. |
| **OpenHands** | Docker-canonical + optional remote | Python controller + Docker sandbox. REST API between controller and sandbox. `remote_runtime.py` for cloud delegation. |
| **Replit Agent** | Remote-canonical + CoW block storage | "Bottomless Storage": virtual block device over GCS, O(1) filesystem forks via manifest copy. Checkpoint = manifest operation. |
| **Cursor Agent** | Local-canonical | Runs in user's environment. E2B integration available for isolation. |

**Takeaway:** All agents doing meaningful isolation use remote-canonical.
The local model is only dominant when isolation is explicitly not needed.

---

## What to Avoid

| Option | Why |
|---|---|
| **AWS Lambda + EFS** | SnapStart and EFS cannot be used together (AWS limitation). 15-min max execution. Cold starts 2–10 seconds. Viable only for async batch tasks, not interactive sessions. |
| **Cloudflare Workers** | JavaScript execution only, no filesystem, 30-second CPU limit. Not viable for bash, cargo, npm. |
| **GitHub Codespaces** | 30–60 second boot time. Designed for human developer sessions, not programmatic per-request use. |
| **Docker without VM isolation** | No kernel boundary. Eight container escape CVEs in 2024–2025. Not suitable for any adversarial code execution. |

---

## Mira `mira-computer` Design

### Trait shape

```rust
/// The execution substrate. Local runs tools in the user's shell.
/// Remote routes them to an isolated sandbox.
#[async_trait]
pub trait ComputeBackend: Send + Sync {
    /// Run a command, streaming stdout/stderr to the provided sink.
    async fn exec(
        &self,
        cmd: &str,
        args: &[&str],
        opts: ExecOpts,
        sink: mpsc::Sender<ComputeEvent>,
    ) -> Result<ExitStatus>;

    /// Read a file's contents.
    async fn read_file(&self, path: &Path) -> Result<String>;

    /// Write a file, creating parent directories if needed.
    async fn write_file(&self, path: &Path, content: &str) -> Result<()>;

    /// Apply a unified diff patch.
    async fn patch_file(&self, path: &Path, patch: &str) -> Result<DiffSummary>;

    /// Persist current state for later resume. Returns an opaque ID.
    async fn checkpoint(&self) -> Result<String>;

    /// Suspend the sandbox (free compute, keep storage).
    async fn pause(&self) -> Result<()>;
}

pub enum ComputeEvent {
    Stdout(String),        // → tool_tail in TUI
    Stderr(String),        // → tool_tail in TUI
    Exit(ExitStatus),
}
```

### Implementations

```
mira-computer/src/
├── lib.rs                 — ComputeBackend trait + types
├── local.rs               — current default: runs in user's shell
├── e2b.rs                 — E2B Firecracker sandbox
├── vercel.rs              — Vercel Sandbox + Drives
└── fly.rs                 — Fly Sprites (low-level, self-managed)
```

### Configuration (mira.toml)

```toml
[compute]
backend = "local"          # "local" | "e2b" | "vercel" | "fly"

[compute.e2b]
api_key = "e2b_..."
template = "ubuntu-24-04"
cpus = 2
memory_mb = 4096
# Snapshot to restore from (optional — cold boot if absent)
snapshot_id = "snap_deps_abc123"

[compute.vercel]
token = "..."
region = "iad1"
# Drive to mount at /workspace for dep caching (optional)
dep_drive = "cargo-registry"

[compute.fly]
api_token = "..."
app = "mira-sprites"
region = "iad"
```

### How the TUI event loop wires it

The event loop today calls `session.send(text)` and handles `HarnessEvent`s.
With `mira-computer`, `HarnessEvent::ToolStart` routes to
`backend.exec()` instead of a local subprocess. `ComputeEvent::Stdout` maps
to the existing `tool_tail` state field — no TUI changes required.

---

## Recommended Starting Point

1. **Implement `local.rs`** — wrap the existing subprocess spawning in the
   trait. Zero behaviour change; just formalises the interface.

2. **Implement `e2b.rs`** — E2B has the best agent SDK, the fastest
   pause/resume, and a free tier for development. Wire `ComputeEvent::Stdout`
   → `tool_tail` events so the live tool tail already in the TUI "just works"
   over the remote stream.

3. **Add `--sandbox` flag** — `mira --sandbox e2b` forces the E2B backend.
   Local stays the default.

4. **Add snapshot caching** — on first use, let the sandbox run
   `cargo build` / `npm install`, checkpoint it, and store the snapshot ID in
   `~/.config/mira/snapshots.toml` keyed to a hash of `Cargo.toml` /
   `package.json`. Subsequent sessions restore from snapshot — zero install
   penalty.

5. **Vercel Drives later** — once snapshot caching is working, add the Drives
   integration for shared dependency caches across multiple Mira instances
   working on the same project.

---

## Cost Reference

| Scenario | E2B | Vercel | Fly Sprites |
|---|---|---|---|
| Quick test (2 min, 2 vCPU / 4 GB) | ~$0.004 | ~$0.01 | ~$0.005 |
| Full session (30 min) | ~$0.04 | ~$0.34* | ~$0.035 |
| Long task (2 hours) | ~$0.16 | ~$2.73* | ~$0.14 |
| 50 agents retained 24h (2h active/day) | ~$533/mo | ~$3,800/mo | ~$90/mo |

\* Vercel active-CPU model is cheaper than it looks for LLM-heavy sessions where 80%+ of time is waiting on the model — the provisioned memory charge is the dominant cost.

**Rule of thumb:** E2B for interactive sessions under 30 minutes. Fly Sprites
for long-lived persistent workspaces. Vercel if you want Drives for dep
caching and are already on Vercel infra.

---

## Implementation notes

What shipped for steps 1–3, and where it differs from the design above.

### Usage

```sh
mira --sandbox local     # scratch copy of the project on this machine
mira --sandbox e2b       # E2B microVM; needs E2B_API_KEY
mira doctor --sandbox e2b
```

Default backend, in `~/.mira/mira.yaml` (the per-repo config can't set
it, because a cloned repo must not decide that its code gets shipped to a
third party):

```yaml
compute:
  backend: e2b            # optional; --sandbox overrides
  e2b:
    api_key_env: E2B_API_KEY   # default; the key can also live under `keys:`
    template: base
    timeout_secs: 3600
```

### Differences from the design

| Design above | What shipped | Why |
|---|---|---|
| Trait in `mira-computer` | New `mira-compute` crate | `mira-computer` became the desktop-control (`computer` tool) crate in #9. |
| `mira.toml`, `[compute]` | `mira.yaml`, `compute:` | Mira's config is YAML. |
| `ToolStart` routes to `backend.exec()` in the event loop | Tools route through `ToolContext::compute` | The event loop only renders; tools do the I/O. Routing there covers subagents and the REPL, and needs no TUI changes. |
| `patch_file` on the trait | Edits are read → modify → write | Keeps every backend to exec + read + write, and edit semantics stay identical to local. |
| `local.rs` = today's default path | Default path is untouched; `LocalBackend` powers `--sandbox local` (a scratch copy) | "Zero behavior change" for the default comes from not routing at all. `--sandbox local` makes the whole remote path testable without a cloud account. |
| Snapshot cache in `~/.config/mira/snapshots.toml` | Not yet (step 4) | `ComputeBackend::checkpoint()` / `E2bBackend::resume()` are in place for it. |

### Workspace sync

- **Start.** Pack git-tracked plus untracked-but-not-ignored files; build
  output and `.gitignore`d secrets never leave the machine. Symlinks are
  kept as links, not followed. Upload and extract the archive into a
  fresh git repo in the sandbox, tagged `mira-baseline`.
- **During.** The sandbox is the source of truth. `bash`, `read_file`,
  `write_file`, `edit_file`, `grep` and `glob` run there. Local absolute
  paths are mapped onto the workspace, and anything outside it is
  refused.
- **Tools that would touch this machine are withheld:** `apply_patch`,
  git and code-intel tools, `rustfmt`, MCP servers, and
  `computer`/`browser`. The model uses `bash` in the sandbox instead.
  Subagents inherit the sandbox, and skip local git-worktree isolation.
- **End.** `git diff --binary mira-baseline` is saved to
  `~/.mira/sandbox/<project>-<ts>.patch`, with a stat and the
  `git apply` command. The checkout is never written to by the session.

### E2B specifics

E2B has no Rust SDK, so `E2bBackend` talks to the control plane (`POST
/sandboxes`, `/pause`, `/resume`, `/timeout`, `DELETE`). It talks to
envd for files (`/files`) and commands. Commands use the Connect-RPC
`process.Process/Start` stream, whose stdout/stderr chunks feed the tool
tail live. A command that times out is killed with `SendSignal`. The
sandbox's timeout is refreshed every 5 minutes while in use, and it's
killed at session end (or on drop).

Tests run the full upload → edit → exec → diff → `git apply` round trip
against a local fake of both E2B surfaces. The code has **not yet been
run against the live E2B service**.

### Not yet

- Step 4 (snapshot caching keyed on lockfile hashes) and step 5 (Vercel
  Drives).
- `mira serve --sandbox` (refused explicitly for now).
- Resuming a session into the same sandbox (the backend supports
  `checkpoint`/`resume`; the CLI doesn't wire it yet).
- A sandbox image with `rg`. `grep` falls back to `grep -rn` in images
  without it.

## Sources

- [Vercel Sandbox Drives public beta](https://vercel.com/changelog/drives-for-vercel-sandbox-are-now-in-public-beta)
- [Vercel Sandbox concepts + pricing](https://vercel.com/docs/sandbox)
- [E2B docs: sandbox persistence, streaming, Claude Code integration](https://docs.e2b.dev)
- [Modal sandbox docs + pricing](https://modal.com/docs/guide/sandboxes)
- [Fly Sprites launch](https://fly.io/blog/fly-machines/)
- [Fly AI sandbox pricing comparison](https://fly.io/learn/ai-sandbox-pricing/)
- [LogRocket: comparing AI agent sandbox platforms](https://blog.logrocket.com/comparing-ai-agent-sandbox-platforms)
- [Northflank: AI sandbox pricing comparison](https://northflank.com/blog/ai-sandbox-pricing)
- [OpenHands runtime architecture](https://docs.openhands.dev/openhands/usage/architecture/runtime)
- [Inside Replit's snapshot engine](https://replit.com/blog/inside-replits-snapshot-engine)
- [How I built sandboxes that boot in 28ms (Firecracker snapshots)](https://dev.to/adwitiya/how-i-built-sandboxes-that-boot-in-28ms-using-firecracker-snapshots-i0k)
- [DeltaBox: millisecond-level sandbox checkpoint/rollback](https://arxiv.org/html/2605.22781v1)
