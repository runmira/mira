//! Find call sites and other usages of a symbol across the repo.
//!
//! Companion to `find_symbol`. Where `find_symbol` targets definitions,
//! this targets every occurrence — a word-boundary search for the
//! identifier, scoped by language file globs so the model doesn't have
//! to know that Rust lives in `*.rs` and TS/JS in `*.ts,*.tsx,…`.
//!
//! Not semantic: shadowed locals, same-named methods on different
//! types, and matches inside comments or strings all appear. Cheap and
//! good enough for "roughly, where is this used?" — pair with
//! `read_file` (or `find_symbol` for the definition) to disambiguate.

use std::time::Duration;

use async_trait::async_trait;
use mira_ai::ToolSpec;
use mira_core::{ToolCall, ToolResult};
use serde::Deserialize;
use serde_json::json;

use crate::context::ToolContext;
use crate::tool::{spec, Action, Tool, ToolError};

pub struct FindReferences;

#[derive(Deserialize)]
struct Args {
    /// Identifier to search for.
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
impl Tool for FindReferences {
    fn spec(&self) -> ToolSpec {
        spec(
            "find_references",
            "Find call sites and other usages of a symbol across the \
             repo. Word-boundary match on the identifier, optionally \
             scoped to a `language` (rust / ts / js / python / go / \
             java) or a `path`. Not semantic — same-named methods on \
             different types and mentions inside comments both match. \
             Pair with `find_symbol` to jump to the definition.",
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Identifier to find." },
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
        // Same guard as find_symbol: refuse anything that would need
        // regex escaping so a malformed input can't silently become a
        // wildcard search.
        if !name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        {
            return Err(ToolError::InvalidArgs(
                "name must be an identifier (alphanumeric / _ / -)".into(),
            ));
        }

        let pattern = format!(r"\b{}\b", regex_escape(name));
        let mut cmd = String::from(
            "rg --line-number --no-heading --color never --max-columns 300",
        );
        cmd.push_str(&format!(" --max-count {}", args.max_results.min(2000)));
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

        let body = if outcome.output.trim().is_empty() {
            format!("no references to `{name}`")
        } else {
            outcome.output
        };
        Ok(ToolResult::ok(call.id.clone(), body))
    }
}

/// File globs for a given language. `None` (or unknown) means no
/// filter — let ripgrep scan everything. TS and JS share globs to
/// match `find_symbol`'s "same family" treatment.
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
