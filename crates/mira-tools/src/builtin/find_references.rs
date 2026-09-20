//! Find call sites and other usages of a symbol across the repo.
//!
//! Companion to `find_symbol`. Where `find_symbol` targets definitions,
//! this targets every occurrence — a word-boundary search for the
//! identifier, scoped by language file globs so the model doesn't
//! have to know that Rust lives in `*.rs` and TS/JS in `*.ts,*.tsx,…`.
//!
//! Not semantic: shadowed locals, same-named methods on different
//! types, and matches inside comments or strings all appear. Cheap and
//! good enough for "roughly, where is this used?" — pair with
//! `read_file` (or `find_symbol` for the definition) to disambiguate.

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
             scoped to a language (rust / ts / js / python / go / java) \
             or a path. Not semantic — same-named methods on different \
             types and mentions inside comments both match. Pair with \
             find_symbol to jump to the definition.",
            json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Identifier to find."
                    },
                    "path": {
                        "type": "string",
                        "description": "Optional subdirectory or file."
                    },
                    "language": {
                        "type": "string",
                        "enum": [
                            "rust",
                            "ts",
                            "js",
                            "python",
                            "go",
                            "java"
                        ],
                        "description": "Restrict to this language's file extensions."
                    },
                    "max_results": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 2000,
                        "default": 200
                    }
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

        if args.max_results == 0 {
            return Err(ToolError::InvalidArgs(
                "max_results must be greater than zero".into(),
            ));
        }

        if !crate::builtin::rg::rg_available() {
            return Err(ToolError::Failed(
                "ripgrep (`rg`) is not installed. Install ripgrep to \
                 continue (`brew install ripgrep` or \
                 `cargo install ripgrep`)."
                    .to_owned(),
            ));
        }

        let pattern = format!(r"\b{}\b", regex_escape(name));
        let max_results = args.max_results.min(2000);

        let mut command_args = vec![
            "--line-number".to_owned(),
            "--no-heading".to_owned(),
            "--color".to_owned(),
            "never".to_owned(),
            "--max-columns".to_owned(),
            "300".to_owned(),
            "--max-count".to_owned(),
            max_results.to_string(),
        ];

        for glob in globs_for(args.language.as_deref()) {
            command_args.push("--glob".to_owned());
            command_args.push((*glob).to_owned());
        }

        command_args.push("-e".to_owned());
        command_args.push(pattern);

        if let Some(path) = &args.path {
            let resolved = ctx
                .resolve(path)
                .ok_or_else(|| ToolError::Failed(format!("path escapes repository: {path}")))?;

            command_args.push(resolved.to_string_lossy().into_owned());
        } else {
            command_args.push(ctx.cwd.to_string_lossy().into_owned());
        }

        let outcome = ctx
            .sandbox
            .run_with_timeout("rg", &command_args, &ctx.cwd, 30)
            .await
            .map_err(|e| ToolError::Failed(e.to_string()))?;

        if outcome.timed_out {
            return Ok(ToolResult::ok(
                call.id.clone(),
                format!("find_references timed out after 30 seconds for `{name}`"),
            ));
        }

        match outcome.exit_code {
            Some(0) => {
                let mut body = outcome.stdout;

                if !outcome.stderr.is_empty() {
                    if !body.is_empty() {
                        body.push('\n');
                    }

                    body.push_str(&outcome.stderr);
                }

                Ok(ToolResult::ok(call.id.clone(), body))
            }

            // rg returns 1 when the search completed successfully but
            // found no matches.
            Some(1) => Ok(ToolResult::ok(
                call.id.clone(),
                format!("no references to `{name}`"),
            )),

            Some(code) => {
                let mut body = format!("ripgrep failed with exit code {code}");

                if !outcome.stderr.is_empty() {
                    body.push('\n');
                    body.push_str(&outcome.stderr);
                }

                if !outcome.stdout.is_empty() {
                    body.push('\n');
                    body.push_str(&outcome.stdout);
                }

                Err(ToolError::Failed(body))
            }

            None => Err(ToolError::Failed(
                "ripgrep terminated without an exit code".into(),
            )),
        }
    }
}

/// File globs for a given language. `None` (or unknown) means no
/// filter — let ripgrep scan everything.
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
