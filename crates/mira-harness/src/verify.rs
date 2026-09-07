//! Apply-verify loop.
//!
//! After the model would otherwise return control ("Stop" with no pending
//! tool calls), the harness inspects which files were written this turn.
//! When they include source files for a project we know how to check
//! (Rust, TypeScript, Python), we run the natural safety check via the
//! sandbox and feed any failure back into the loop as a synthetic user
//! message so the model gets one shot to fix.
//!
//! Kept intentionally conservative:
//!
//! - **Auto-detect only.** We never invent a check — if nothing matches, we
//!   emit `Done` unchanged. Doesn't touch python-typechecking, complicated
//!   build scripts, custom test runners, etc.
//! - **Cap on retries.** Model can't loop forever — three attempts per
//!   turn max; after that we emit a warning and stop.
//! - **Cheap timeouts.** Type-checkers (`cargo check`, `tsc --noEmit`,
//!   `ruff check`) are fast; expensive things (`cargo test`, `pytest`) are
//!   left to explicit tool calls so verify never balloons a turn.

use std::path::{Path, PathBuf};
use std::time::Duration;

use mira_sandbox::Sandbox;

/// One project-appropriate command we run to catch obvious regressions.
pub struct VerifyCheck {
    /// Short name shown in warning frames — e.g. `cargo check`.
    pub name: &'static str,
    /// Full shell command line handed to the sandbox.
    pub command: String,
    /// Hard cap on runtime. Keeps a hung check from blocking Done forever.
    pub timeout: Duration,
}

pub struct VerifyOutcome {
    pub ok: bool,
    pub output: String,
}

/// Pick a check based on the cwd and which files the current turn wrote.
/// Returns `None` when we don't have a rule that applies — e.g. the writes
/// were only to markdown, or the project isn't one we recognise.
pub fn detect(cwd: &Path, writes: &[PathBuf]) -> Option<VerifyCheck> {
    let ext = |p: &PathBuf| p.extension().map(|e| e.to_string_lossy().into_owned());
    let touched_rust = writes.iter().any(|p| ext(p).as_deref() == Some("rs"));
    let touched_ts = writes.iter().any(|p| {
        matches!(ext(p).as_deref(), Some("ts" | "tsx" | "mts" | "cts"))
    });
    let touched_js = writes.iter().any(|p| {
        matches!(ext(p).as_deref(), Some("js" | "jsx" | "mjs" | "cjs"))
    });
    let touched_py = writes.iter().any(|p| ext(p).as_deref() == Some("py"));
    let touched_go = writes.iter().any(|p| ext(p).as_deref() == Some("go"));

    if touched_rust && cwd.join("Cargo.toml").exists() {
        return Some(VerifyCheck {
            name: "cargo check",
            command: "cargo check --quiet --message-format=short".into(),
            timeout: Duration::from_secs(300),
        });
    }
    if (touched_ts || touched_js) && cwd.join("tsconfig.json").exists() {
        // `--no-install` prevents npx from downloading tsc mid-turn if it's
        // not present; failing that we just skip and the model runs its
        // own build later.
        return Some(VerifyCheck {
            name: "tsc",
            command: "npx --no-install tsc --noEmit".into(),
            timeout: Duration::from_secs(180),
        });
    }
    if touched_py
        && (cwd.join("pyproject.toml").exists() || cwd.join("setup.py").exists())
    {
        return Some(VerifyCheck {
            name: "ruff check",
            command: "ruff check .".into(),
            timeout: Duration::from_secs(60),
        });
    }
    if touched_go && cwd.join("go.mod").exists() {
        return Some(VerifyCheck {
            name: "go build",
            command: "go build ./...".into(),
            timeout: Duration::from_secs(300),
        });
    }
    None
}

pub async fn run(sandbox: &Sandbox, check: &VerifyCheck, cwd: &Path) -> VerifyOutcome {
    match sandbox.run(&check.command, cwd, check.timeout).await {
        Ok(o) => VerifyOutcome {
            ok: o.exit_code == 0 && !o.timed_out,
            output: o.output,
        },
        Err(e) => VerifyOutcome {
            ok: false,
            output: format!("(sandbox error: {e})"),
        },
    }
}
