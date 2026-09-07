//! Locate symbol definitions across the repo.
//!
//! Cheaper alternative to LSP: recognises the common definition shapes for
//! Rust, TS/JS, Python, Go, and Java, and shells out to ripgrep with a
//! merged regex so a single scan covers every kind. Falls back to a
//! word-boundary match when the language is unknown.
//!
//! Not a semantic index — a `fn` and a comment referencing that fn both
//! match; the model still needs to read to disambiguate. What this earns
//! over plain grep is *precision*: the patterns anchor to the syntactic
//! forms a definition actually takes, so you're not sifting through every
//! call site.

use std::time::Duration;

use async_trait::async_trait;
use mira_ai::ToolSpec;
use mira_core::{ToolCall, ToolResult};
use serde::Deserialize;
use serde_json::json;

use crate::context::ToolContext;
use crate::tool::{spec, Action, Tool, ToolError};

pub struct FindSymbol;

#[derive(Deserialize)]
struct Args {
    /// Symbol identifier to find (function / class / struct / etc.).
    name: String,
    /// Restrict to a subdirectory or file path.
    #[serde(default)]
    path: Option<String>,
    /// Force a specific language mode. When omitted, we run all patterns
    /// so multi-language repos are covered.
    #[serde(default)]
    language: Option<String>,
    /// Cap on returned lines.
    #[serde(default = "default_max")]
    max_results: usize,
}

fn default_max() -> usize {
    100
}

#[async_trait]
impl Tool for FindSymbol {
    fn spec(&self) -> ToolSpec {
        spec(
            "find_symbol",
            "Locate the definition of a symbol (function, class, struct, \
             const, type, …) across the repo. Faster than `read_file` on \
             whole files when you just need to jump to a definition. \
             Optionally scoped with `path` or `language`. Matches against \
             common definition shapes for Rust, TypeScript / JavaScript, \
             Python, Go, and Java; falls back to a word-boundary search \
             when the language is unknown.",
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Identifier to locate." },
                    "path": { "type": "string", "description": "Optional subdirectory or file." },
                    "language": {
                        "type": "string",
                        "enum": ["rust", "ts", "js", "python", "go", "java"],
                        "description": "Force a language mode. Omit to search all."
                    },
                    "max_results": { "type": "integer", "minimum": 1, "maximum": 500, "default": 100 }
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
        // Bail on identifiers that would need regex-escaping in the pattern.
        // We're not sanitising — just refusing so a malformed input can't
        // silently become a wildcard search.
        if !name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        {
            return Err(ToolError::InvalidArgs(
                "name must be an identifier (alphanumeric / _ / -)".into(),
            ));
        }

        let patterns = patterns_for(args.language.as_deref(), name);
        // ripgrep alternation via `-e` per pattern — cleaner than one giant
        // regex and lets each pattern carry its own file-type filter via `-g`.
        let mut cmd = String::from(
            "rg --line-number --no-heading --color never --max-count 5",
        );
        cmd.push_str(&format!(" --max-columns 300 --max-count {}", args.max_results.min(500)));
        for (pat, globs) in &patterns {
            for g in globs.iter() {
                cmd.push_str(&format!(" -g {}", shell_quote(g)));
            }
            cmd.push_str(" -e ");
            cmd.push_str(&shell_quote(pat));
        }
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

        // ripgrep exits 1 on "no matches" — treat as ok with a friendly note.
        let body = if outcome.output.trim().is_empty() {
            format!("no definition-shaped match for `{name}`")
        } else {
            outcome.output
        };
        Ok(ToolResult::ok(call.id.clone(), body))
    }
}

/// (pattern, file globs). Empty globs = no filter (used for the fallback).
fn patterns_for(lang: Option<&str>, name: &str) -> Vec<(String, &'static [&'static str])> {
    // Anchor each pattern with `\b` so `foo` doesn't also match `foobar`.
    let n = regex_escape(name);
    let all = |lang: &str| -> Vec<(String, &'static [&'static str])> {
        match lang {
            "rust" => vec![
                (format!(r"(^|\s)(pub(\(.*?\))?\s+)?(async\s+)?fn\s+{n}\b"), &["*.rs"]),
                (format!(r"(^|\s)(pub(\(.*?\))?\s+)?(struct|enum|trait|type|const|static|mod|union)\s+{n}\b"), &["*.rs"]),
                (format!(r"impl(\s+<[^>]+>)?\s+{n}\b"), &["*.rs"]),
                (format!(r"macro_rules!\s+{n}\b"), &["*.rs"]),
            ],
            "ts" | "js" => vec![
                (format!(r"(^|\s)(export\s+)?(async\s+)?function\s+{n}\b"), &["*.ts", "*.tsx", "*.js", "*.jsx", "*.mts", "*.mjs"]),
                (format!(r"(^|\s)(export\s+)?(abstract\s+)?class\s+{n}\b"), &["*.ts", "*.tsx", "*.js", "*.jsx"]),
                (format!(r"(^|\s)(export\s+)?(interface|type|enum)\s+{n}\b"), &["*.ts", "*.tsx"]),
                (format!(r"(^|\s)(export\s+)?(const|let|var)\s+{n}\b"), &["*.ts", "*.tsx", "*.js", "*.jsx"]),
            ],
            "python" => vec![
                (format!(r"^\s*(async\s+)?def\s+{n}\b"), &["*.py"]),
                (format!(r"^\s*class\s+{n}\b"), &["*.py"]),
                (format!(r"^\s*{n}\s*="), &["*.py"]),
            ],
            "go" => vec![
                (format!(r"^func(\s+\([^)]*\))?\s+{n}\b"), &["*.go"]),
                (format!(r"^type\s+{n}\b"), &["*.go"]),
                (format!(r"^var\s+{n}\b|^const\s+{n}\b"), &["*.go"]),
            ],
            "java" => vec![
                (format!(r"(public|private|protected|static|final|\s)+\s+{n}\s*\("), &["*.java"]),
                (format!(r"(public|private|protected|abstract|static|final|\s)+\s+(class|interface|enum|record)\s+{n}\b"), &["*.java"]),
            ],
            _ => vec![(format!(r"\b{n}\b"), &[] as &[&str])],
        }
    };
    match lang {
        Some(l) => all(l),
        None => {
            // Merge all language patterns. ripgrep de-dupes on file path
            // implicitly (same file may match multiple patterns; we accept
            // that — extra matches are cheap).
            let mut out = Vec::new();
            for l in ["rust", "ts", "python", "go", "java"] {
                out.extend(all(l));
            }
            out
        }
    }
}

fn regex_escape(s: &str) -> String {
    // Names are already validated to be identifier-shaped, so nothing here
    // is a regex metachar — but escape defensively anyway.
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
