//! Apply-verify loop.
//!
//! After the model would otherwise return control ("Stop" with no pending
//! tool calls), the harness inspects which files were written this turn.
//! When they include source files for a project we know how to check
//! (Rust, TypeScript, Python, Go), we run the natural safety check via the
//! sandbox and feed any failure back into the loop as a synthetic user
//! message so the model gets one shot to fix.
//!
//! Kept intentionally conservative:
//!
//! - **Auto-detect only.** We never invent a check — if nothing matches, we
//!   emit `Done` unchanged.
//! - **Cap on retries.** Three attempts per turn max; after that we emit a
//!   warning and stop.
//! - **Cheap timeouts.** Type-checkers are fast; expensive things
//!   (`cargo test`, `pytest`) are left to explicit tool calls.
//! - **No shell.** Checks are run as `binary + args` through the sandbox,
//!   which runs offline by default, so checks must not need the network.

use std::path::{Path, PathBuf};
use std::time::Duration;

use mira_sandbox::Sandbox;

/// Max characters of check output fed back to the model (tail is kept,
/// since compilers put the summary last).
const MAX_OUTPUT_CHARS: usize = 8_000;

/// One project-appropriate command we run to catch obvious regressions.
pub struct VerifyCheck {
    /// Short name shown in warning frames — e.g. `cargo check`.
    pub name: &'static str,
    /// Executable to run (resolved via PATH inside the sandbox).
    pub binary: &'static str,
    /// Arguments passed directly to the executable (no shell parsing).
    pub args: Vec<String>,
    /// Hard cap on runtime. Keeps a hung check from blocking Done forever.
    pub timeout: Duration,
}

impl VerifyCheck {
    fn new(name: &'static str, binary: &'static str, args: &[&str], timeout_secs: u64) -> Self {
        Self {
            name,
            binary,
            args: args.iter().map(|a| a.to_string()).collect(),
            timeout: Duration::from_secs(timeout_secs),
        }
    }
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
    let touched_ts = writes
        .iter()
        .any(|p| matches!(ext(p).as_deref(), Some("ts" | "tsx" | "mts" | "cts")));
    let touched_js = writes
        .iter()
        .any(|p| matches!(ext(p).as_deref(), Some("js" | "jsx" | "mjs" | "cjs")));
    let touched_py = writes.iter().any(|p| ext(p).as_deref() == Some("py"));
    let touched_go = writes.iter().any(|p| ext(p).as_deref() == Some("go"));

    if touched_rust && cwd.join("Cargo.toml").exists() {
        // The sandbox denies network by default, so fail fast and clearly
        // instead of hanging on a registry fetch.
        return Some(VerifyCheck::new(
            "cargo check",
            "cargo",
            &["check", "--quiet", "--offline", "--message-format=short"],
            300,
        ));
    }
    if (touched_ts || touched_js) && cwd.join("tsconfig.json").exists() {
        // `--no-install` prevents npx from downloading tsc mid-turn.
        return Some(VerifyCheck::new(
            "tsc",
            "npx",
            &["--no-install", "tsc", "--noEmit"],
            180,
        ));
    }
    if touched_py && (cwd.join("pyproject.toml").exists() || cwd.join("setup.py").exists()) {
        return Some(VerifyCheck::new("ruff check", "ruff", &["check", "."], 60));
    }
    if touched_go && cwd.join("go.mod").exists() {
        return Some(VerifyCheck::new("go build", "go", &["build", "./..."], 300));
    }
    None
}

pub async fn run(sandbox: &Sandbox, check: &VerifyCheck, cwd: &Path) -> VerifyOutcome {
    let result = sandbox
        .run_with_timeout(check.binary, &check.args, cwd, check.timeout.as_secs())
        .await;

    match result {
        Ok(o) => {
            let mut output = o.combined_output();

            if o.timed_out {
                output.push_str(&format!(
                    "\n(`{}` timed out after {}s)",
                    check.name,
                    check.timeout.as_secs()
                ));
            } else if o.exit_code.is_none() {
                output.push_str("\n(process terminated by signal)");
            }

            VerifyOutcome {
                ok: o.success(),
                output: tail_chars(&output, MAX_OUTPUT_CHARS),
            }
        }
        Err(e) => VerifyOutcome {
            ok: false,
            output: format!("(sandbox error: {e:#})"),
        },
    }
}

/// Keep the last `max` characters, on a char boundary.
fn tail_chars(s: &str, max: usize) -> String {
    let total = s.chars().count();
    if total <= max {
        return s.to_string();
    }
    let skip = total - max;
    let start = s.char_indices().nth(skip).map(|(i, _)| i).unwrap_or(0);
    format!("…(truncated)\n{}", &s[start..])
}