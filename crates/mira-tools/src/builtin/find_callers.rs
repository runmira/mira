//! Find call sites of a function, method, or constructor.
//!
//! Narrower than `find_references`: instead of every occurrence of the
//! identifier, this returns only lines where the identifier is
//! immediately followed by `(` — i.e. a call — and drops the
//! definition itself (`fn NAME(`, `def NAME(`, `func NAME(`,
//! `function NAME(`). What's left is, roughly, "who calls this?".
//!
//! Still not semantic — same-named methods on different receivers all
//! match, macros with `!` in the name (Rust) won't, and the
//! definition-line filter is best-effort per-language. Use
//! `find_references` when you need the un-filtered view.

use std::path::Path;
use std::time::Duration;

use async_trait::async_trait;
use mira_ai::ToolSpec;
use mira_core::{ToolCall, ToolResult};
use regex::Regex;
use serde::Deserialize;
use serde_json::json;

use crate::context::ToolContext;
use crate::tool::{spec, Action, Tool, ToolError};

pub struct FindCallers;

#[derive(Deserialize)]
struct Args {
    /// Identifier whose call sites we want.
    name: String,
    /// Restrict to a subdirectory or file path.
    #[serde(default)]
    path: Option<String>,
    /// Restrict to a specific language's file extensions.
    #[serde(default)]
    language: Option<String>,
    /// Cap on returned lines.
    #[serde(default = "default_max")]
    max_results: usize,
}

fn default_max() -> usize {
    200
}

#[async_trait]
impl Tool for FindCallers {
    fn spec(&self) -> ToolSpec {
        spec(
            "find_callers",
            "Find call sites of a function, method, or constructor \
             across the repo. Matches the identifier followed by `(`, \
             then filters out the definition itself (fn / def / func / \
             function NAME). Optional `language` (rust / ts / js / \
             python / go / java) and `path` scope. Not semantic — \
             same-named methods on different receivers all match. Use \
             `find_references` for every mention (imports, types, \
             comments).",
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Identifier to find calls of." },
                    "path": { "type": "string", "description": "Optional subdirectory or file." },
                    "language": {
                        "type": "string",
                        "enum": ["rust", "ts", "js", "python", "go", "java"],
                        "description": "Restrict to this language's file extensions."
                    },
                    "max_results": { "type": "integer", "minimum": 1, "maximum": 2000, "default": 200 }
                },
                "required": ["name"],
                "additionalProperties": false
            }),
        )
    }

    fn action(&self) -> Action {
        Action::Read
    }

    async fn invoke(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let args: Args = call.parse_arguments()?;
        let name = args.name.trim();
        if name.is_empty() {
            return Err(ToolError::InvalidArgs("name is empty".into()));
        }
        if !name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        {
            return Err(ToolError::InvalidArgs(
                "name must be an identifier (alphanumeric / _ / -)".into(),
            ));
        }

        // \b NAME \s* \( — the leading \b handles the preceding-char case
        // (`.NAME(`, `::NAME(`, `NAME(` all match; `xNAME(` doesn't).
        let pattern = format!(r"\b{}\s*\(", regex_escape(name));
        let mut cmd = String::from(
            "rg --line-number --no-heading --color never --max-columns 300",
        );
        // Over-fetch: post-filtering will drop some hits, so give rg
        // headroom before we cap in-process.
        let raw_cap = args.max_results.saturating_mul(2).clamp(50, 4000);
        cmd.push_str(&format!(" --max-count {}", raw_cap));
        for g in globs_for(args.language.as_deref()) {
            cmd.push_str(&format!(" -g {}", shell_quote(g)));
        }
        cmd.push_str(" -e ");
        cmd.push_str(&shell_quote(&pattern));
        if let Some(p) = &args.path {
            let resolved = ctx
                .resolve(p)
                .ok_or_else(|| ToolError::Failed(format!("path escapes cwd: {p}")))?;
            cmd.push(' ');
            cmd.push_str(&shell_quote(&resolved.to_string_lossy()));
        }

        let outcome = ctx
            .sandbox
            .run(&cmd, &ctx.cwd, Duration::from_secs(30))
            .await
            .map_err(|e| ToolError::Failed(e.to_string()))?;

        let filtered = drop_definition_lines(&outcome.output, name, args.max_results.min(2000));
        let body = if filtered.trim().is_empty() {
            format!("no callers of `{name}`")
        } else {
            filtered
        };
        Ok(ToolResult::ok(call.id.clone(), body))
    }
}

/// Language file globs. `None` / unknown → no filter. TS and JS share
/// globs to match the rest of the code-intelligence tools.
fn globs_for(lang: Option<&str>) -> &'static [&'static str] {
    match lang {
        Some("rust") => &["*.rs"],
        Some("ts") | Some("js") => &["*.ts", "*.tsx", "*.js", "*.jsx", "*.mts", "*.mjs"],
        Some("python") => &["*.py"],
        Some("go") => &["*.go"],
        Some("java") => &["*.java"],
        _ => &[],
    }
}

/// Walk ripgrep's `path:lineno:content` output, drop lines that look
/// like the definition of `name`, and truncate at `max`.
fn drop_definition_lines(rg_out: &str, name: &str, max: usize) -> String {
    let defs = definition_regexes(name);
    let mut out = String::new();
    let mut kept = 0usize;
    for line in rg_out.lines() {
        let Some((path, rest)) = line.split_once(':') else { continue };
        let Some((_lineno, content)) = rest.split_once(':') else { continue };
        let lang = lang_of(path);
        if let Some(re) = defs.get(lang) {
            if re.is_match(content) {
                continue;
            }
        }
        if kept >= max {
            break;
        }
        out.push_str(line);
        out.push('\n');
        kept += 1;
    }
    out
}

/// One compiled def-shape regex per language. Best-effort: catches the
/// common `<keyword> NAME (` / `<keyword> NAME <` shapes. Misses Go
/// methods with receivers (`func (r *R) NAME(`) and Java (which has
/// no leading keyword) — those lines will pass through unfiltered.
fn definition_regexes(name: &str) -> std::collections::HashMap<&'static str, Regex> {
    let n = regex_escape(name);
    let mk = |p: String| Regex::new(&p).expect("valid regex");
    let mut m = std::collections::HashMap::new();
    m.insert("rust", mk(format!(r"\bfn\s+{n}\s*[(<]")));
    m.insert("python", mk(format!(r"\bdef\s+{n}\s*\(")));
    // Go: also catch `func <name>(` (receiver-less funcs).
    m.insert("go", mk(format!(r"\bfunc\s+{n}\s*\(")));
    m.insert("ts", mk(format!(r"\bfunction\s+{n}\s*[(<]")));
    m.insert("js", mk(format!(r"\bfunction\s+{n}\s*\(")));
    m
}

fn lang_of(path: &str) -> &'static str {
    match Path::new(path).extension().and_then(|s| s.to_str()) {
        Some("rs") => "rust",
        Some("py") => "python",
        Some("go") => "go",
        Some("ts") | Some("tsx") | Some("mts") => "ts",
        Some("js") | Some("jsx") | Some("mjs") | Some("cjs") => "js",
        _ => "",
    }
}

fn regex_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if "\\.^$|?*+()[]{}".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn shell_quote(s: &str) -> String {
    let escaped = s.replace('\'', "'\\''");
    format!("'{escaped}'")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(path: &str, line: u32, content: &str) -> String {
        format!("{path}:{line}:{content}\n")
    }

    #[test]
    fn drops_rust_definition_keeps_calls() {
        let name = "foo";
        let input = [
            call("src/lib.rs", 10, "fn foo() -> u32 {"),
            call("src/lib.rs", 20, "    foo();"),
            call("src/other.rs", 5, "    thing.foo(1, 2);"),
            call("src/other.rs", 8, "    Module::foo();"),
        ]
        .concat();
        let out = drop_definition_lines(&input, name, 100);
        assert!(!out.contains("fn foo()"), "definition should be dropped");
        assert!(out.contains("foo();"));
        assert!(out.contains("thing.foo("));
        assert!(out.contains("Module::foo();"));
    }

    #[test]
    fn drops_generic_rust_definition() {
        let input = call("src/lib.rs", 3, "pub fn foo<T: Clone>(x: T) -> T {");
        let out = drop_definition_lines(&input, "foo", 100);
        assert!(out.trim().is_empty());
    }

    #[test]
    fn keeps_similar_named_fn_definition() {
        // `fn foobar(` should NOT be filtered when looking for `foo`.
        let input = call("src/lib.rs", 1, "fn foobar() {");
        // But rg wouldn't emit this line for the `\bfoo\s*\(` pattern
        // in the first place, so this test just guards the filter
        // itself from being over-eager.
        let out = drop_definition_lines(&input, "foo", 100);
        assert!(out.contains("foobar"));
    }

    #[test]
    fn python_def_dropped() {
        let input = [
            call("mod.py", 10, "def foo(x):"),
            call("mod.py", 20, "    foo(1)"),
        ]
        .concat();
        let out = drop_definition_lines(&input, "foo", 100);
        assert!(!out.contains("def foo"));
        assert!(out.contains("foo(1)"));
    }

    #[test]
    fn respects_cap() {
        let input = (0..10)
            .map(|i| call("f.rs", i, "    foo();"))
            .collect::<String>();
        let out = drop_definition_lines(&input, "foo", 3);
        assert_eq!(out.lines().count(), 3);
    }

    #[test]
    fn unknown_language_passes_through() {
        // A .java definition line survives — Java has no leading
        // keyword we can cheaply match on, so we don't filter.
        let input = call("A.java", 1, "public void foo() {");
        let out = drop_definition_lines(&input, "foo", 100);
        assert!(out.contains("public void foo"));
    }
}
