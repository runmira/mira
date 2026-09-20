//! Shared helpers for the ripgrep-based code-intel tools.
//!
//! The five tools (grep, find_symbol, find_callers, find_references, plus
//! future variants) all shell out to `rg`. When ripgrep isn't installed on
//! PATH the raw shell call surfaces as `rg: command not found`, and the
//! model quietly re-routes through `bash` — which is exactly what the user
//! saw in the wild. This module centralises two things:
//!
//! 1. A cached [`rg_available`] probe so we only pay the `which` cost once.
//! 2. A [`translate_to_grep`] fallback that rewrites the most common rg
//!    invocations into POSIX `grep -REn` so the tool still returns useful
//!    results (with a leading note) on systems without ripgrep.
//!
//! The translation is best-effort — a handful of rg-specific flags
//! (`-e`, `-g`, `--max-columns`) map cleanly to grep equivalents; the rest
//! are dropped with a debug log. When translation isn't possible we return
//! `None` so the caller can raise a clear "install ripgrep" error rather
//! than silently truncating results.

use std::sync::OnceLock;

/// True when `rg` is on PATH (checked once per process).
pub fn rg_available() -> bool {
    static CACHED: OnceLock<bool> = OnceLock::new();
    *CACHED.get_or_init(|| which_binary("rg"))
}

fn which_binary(name: &str) -> bool {
    let Ok(path) = std::env::var("PATH") else {
        return false;
    };
    let sep = if cfg!(windows) { ';' } else { ':' };
    for dir in path.split(sep) {
        let candidate = std::path::Path::new(dir).join(name);
        if candidate.is_file() {
            return true;
        }
        if cfg!(windows) {
            let candidate = candidate.with_extension("exe");
            if candidate.is_file() {
                return true;
            }
        }
    }
    false
}
