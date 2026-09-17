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

/// Translate a fully-formed `rg …` command line into a `grep -REn …`
/// equivalent. Returns `None` when the invocation uses rg-only features
/// (multiple `-e` patterns) that don't map cleanly.
///
/// The translation targets the two output invariants callers rely on:
/// - `file:line:content` prefix on each match line
/// - a zero exit-code contract via `|| true` (POSIX grep exits 1 on no
///   matches; the caller already treats empty output as "no matches").
pub fn translate_to_grep(rg_cmd: &str) -> Option<String> {
    let tokens = shell_split(rg_cmd)?;
    if tokens.first().map(String::as_str) != Some("rg") {
        return None;
    }

    let mut patterns: Vec<String> = Vec::new();
    let mut includes: Vec<String> = Vec::new();
    let mut case_insensitive = false;
    let mut max_count: Option<String> = None;
    let mut positional: Vec<String> = Vec::new();

    let mut it = tokens.iter().skip(1).peekable();
    while let Some(tok) = it.next() {
        match tok.as_str() {
            "--line-number" | "--no-heading" => {}
            "--color" => {
                let _ = it.next();
            }
            s if s.starts_with("--color=") => {}
            "-i" | "--ignore-case" => case_insensitive = true,
            "--max-count" => {
                if let Some(v) = it.next() {
                    max_count = Some(v.clone());
                }
            }
            s if s.starts_with("--max-count=") => {
                max_count = Some(s.trim_start_matches("--max-count=").to_owned());
            }
            "-m" => {
                if let Some(v) = it.next() {
                    max_count = Some(v.clone());
                }
            }
            "--max-columns" => {
                let _ = it.next();
            }
            s if s.starts_with("--max-columns=") => {}
            "-g" | "--glob" => {
                if let Some(v) = it.next() {
                    includes.push(v.clone());
                }
            }
            "-e" | "--regexp" => {
                if let Some(v) = it.next() {
                    patterns.push(v.clone());
                }
            }
            s if s.starts_with("--") || (s.starts_with('-') && s.len() > 1) => {
                // Unknown flag — bail rather than silently mis-translate.
                return None;
            }
            _ => positional.push(tok.clone()),
        }
    }

    // First non-flag positional is the pattern when no explicit -e was given.
    let mut roots: Vec<String> = Vec::new();
    if patterns.is_empty() {
        if positional.is_empty() {
            return None;
        }
        patterns.push(positional.remove(0));
    }
    roots.extend(positional);
    if roots.is_empty() {
        roots.push(".".to_string());
    }

    // grep only supports one -e per invocation cleanly for our shape;
    // multiple patterns → union via -e … -e …, which POSIX grep also
    // accepts.
    let mut cmd = String::from("grep -REn");
    if case_insensitive {
        cmd.push_str(" -i");
    }
    if let Some(n) = max_count {
        cmd.push_str(&format!(" -m {n}"));
    }
    for inc in &includes {
        cmd.push_str(&format!(" --include={}", shell_quote(inc)));
    }
    for pat in &patterns {
        cmd.push_str(" -e ");
        cmd.push_str(&shell_quote(pat));
    }
    for r in &roots {
        cmd.push(' ');
        cmd.push_str(&shell_quote(r));
    }
    // POSIX grep exits 1 on no matches; keep the shell happy so downstream
    // sandbox-error handling doesn't misread "no results" as a hard failure.
    cmd.push_str(" || true");
    Some(cmd)
}

/// Prefix appended to fallback output so the model (and human reading the
/// transcript) can see we're on the degraded path — not silently wrong.
pub const FALLBACK_NOTICE: &str =
    "note: ripgrep (`rg`) isn't on PATH; using `grep -REn` as a fallback. \
     Install ripgrep for faster searches (`brew install ripgrep` / \
     `cargo install ripgrep`).\n\n";

/// Split a `bash -lc` style command line on whitespace, honoring single
/// quotes. We only produce these strings ourselves (see the callers below),
/// so this is a *reverse* of [`shell_quote`] rather than a full sh parser.
fn shell_split(s: &str) -> Option<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut chars = s.chars().peekable();
    let mut in_quote = false;
    while let Some(c) = chars.next() {
        if in_quote {
            if c == '\'' {
                // In our quoting scheme, an escaped single-quote is emitted
                // as `'\''` — a closing quote, a backslash-quote, and a
                // reopening quote. Handle that four-char run so identifiers
                // containing `'` survive the round-trip.
                if chars.peek() == Some(&'\\') {
                    let mut lookahead = chars.clone();
                    lookahead.next(); // '\\'
                    if lookahead.next() == Some('\'') && lookahead.next() == Some('\'') {
                        cur.push('\'');
                        chars.next(); // '\\'
                        chars.next(); // '\''
                        chars.next(); // '\''
                        continue;
                    }
                }
                in_quote = false;
            } else {
                cur.push(c);
            }
        } else if c == '\'' {
            in_quote = true;
        } else if c.is_whitespace() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(c);
        }
    }
    if in_quote {
        return None;
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    Some(out)
}

fn shell_quote(s: &str) -> String {
    let escaped = s.replace('\'', "'\\''");
    format!("'{escaped}'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translates_simple_grep() {
        let rg = "rg --line-number --no-heading --color never -e 'foo' 'src'";
        let got = translate_to_grep(rg).unwrap();
        assert!(got.starts_with("grep -REn"));
        assert!(got.contains(" -e 'foo'"));
        assert!(got.contains(" 'src'"));
        assert!(got.ends_with("|| true"));
    }

    #[test]
    fn preserves_ignore_case_and_max_count() {
        let rg = "rg --line-number --no-heading --color never -i --max-count 200 'api_key'";
        let got = translate_to_grep(rg).unwrap();
        assert!(got.contains(" -i"));
        assert!(got.contains(" -m 200"));
    }

    #[test]
    fn maps_globs_to_includes() {
        let rg = "rg --line-number --no-heading --color never -g '*.rs' -e 'fn foo'";
        let got = translate_to_grep(rg).unwrap();
        assert!(got.contains("--include='*.rs'"));
    }

    #[test]
    fn defaults_root_to_dot_when_missing() {
        let rg = "rg --line-number --no-heading --color never 'foo'";
        let got = translate_to_grep(rg).unwrap();
        assert!(got.ends_with(" '.' || true"), "got: {got}");
    }

    #[test]
    fn bails_on_unknown_flag() {
        let rg = "rg --wibble 'foo'";
        assert!(translate_to_grep(rg).is_none());
    }
}
