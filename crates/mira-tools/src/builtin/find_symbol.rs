
//! Locate symbol definitions across the repo.
//!
//! Cheaper alternative to LSP: recognises the common definition shapes for
//! Rust, TS/JS, Python, Go, and Java, and shells out to ripgrep with
//! multiple patterns so a single scan covers every kind.
//!
//! Not a semantic index — a `fn` and a comment referencing that fn both
//! match; the model still needs to read to disambiguate. What this earns
//! over plain grep is *precision*: the patterns anchor to the syntactic
//! forms a definition actually takes, so you're not sifting through every
//! call site.

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

    /// Force a specific language mode. When omitted, search all patterns.
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
             const, type, etc.) across the repo. Faster than read_file on \
             whole files when you just need to jump to a definition. \
             Optionally scoped with path or language. Matches common \
             definition shapes for Rust, TypeScript / JavaScript, Python, \
             Go, and Java; falls back to a word-boundary search when the \
             language is unknown.",
            json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Identifier to locate."
                    },
                    "path": {
                        "type": "string",
                        "description": "Optional subdirectory or file."
                    },
                    "language": {
                        "type": "string",
                        "enum": ["rust", "ts", "js", "python", "go", "java"],
                        "description": "Force a language mode. Omit to search all."
                    },
                    "max_results": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 500,
                        "default": 100
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

    async fn invoke(
        &self,
        call: &ToolCall,
        ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let args: Args = call.parse_arguments()?;

        let name = args.name.trim();

        if name.is_empty() {
            return Err(ToolError::InvalidArgs(
                "name is empty".into(),
            ));
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

        let patterns = patterns_for(args.language.as_deref(), name);
        let max_results = args.max_results.min(500);

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

        // Every definition pattern becomes its own -e argument.
        for (pattern, globs) in &patterns {
            for glob in *globs {
                command_args.push("--glob".to_owned());
                command_args.push((*glob).to_owned());
            }

            command_args.push("-e".to_owned());
            command_args.push(pattern.clone());
        }

        if let Some(path) = &args.path {
            let resolved = ctx.resolve(path).ok_or_else(|| {
                ToolError::Failed(format!(
                    "path escapes repository: {path}"
                ))
            })?;

            command_args.push(
                resolved.to_string_lossy().into_owned()
            );
        } else {
            command_args.push(
                ctx.cwd.to_string_lossy().into_owned()
            );
        }

        let outcome = ctx
            .sandbox
            .run_with_timeout(
                "rg",
                &command_args,
                &ctx.cwd,
                30,
            )
            .await
            .map_err(|e| ToolError::Failed(e.to_string()))?;

        if outcome.timed_out {
            return Ok(ToolResult::ok(
                call.id.clone(),
                format!(
                    "find_symbol timed out after 30 seconds for `{name}`"
                ),
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

            Some(1) => Ok(ToolResult::ok(
                call.id.clone(),
                format!(
                    "no definition-shaped match for `{name}`"
                ),
            )),

            Some(code) => {
                let mut body = format!(
                    "ripgrep failed with exit code {code}"
                );

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

/// (pattern, file globs). Empty globs = no filter.
fn patterns_for(
    lang: Option<&str>,
    name: &str,
) -> Vec<(String, &'static [&'static str])> {
    let n = regex_escape(name);

    let all = |lang: &str| -> Vec<(String, &'static [&'static str])> {
        match lang {
            "rust" => vec![
                (
                    format!(
                        r"(^|\s)(pub(\(.*?\))?\s+)?(async\s+)?fn\s+{n}\b"
                    ),
                    &["*.rs"],
                ),
                (
                    format!(
                        r"(^|\s)(pub(\(.*?\))?\s+)?(struct|enum|trait|type|const|static|mod|union)\s+{n}\b"
                    ),
                    &["*.rs"],
                ),
                (
                    format!(r"impl(\s+<[^>]+>)?\s+{n}\b"),
                    &["*.rs"],
                ),
                (
                    format!(r"macro_rules!\s+{n}\b"),
                    &["*.rs"],
                ),
            ],

            "ts" | "js" => vec![
                (
                    format!(
                        r"(^|\s)(export\s+)?(async\s+)?function\s+{n}\b"
                    ),
                    &[
                        "*.ts",
                        "*.tsx",
                        "*.js",
                        "*.jsx",
                        "*.mts",
                        "*.mjs",
                    ],
                ),
                (
                    format!(
                        r"(^|\s)(export\s+)?(abstract\s+)?class\s+{n}\b"
                    ),
                    &["*.ts", "*.tsx", "*.js", "*.jsx"],
                ),
                (
                    format!(
                        r"(^|\s)(export\s+)?(interface|type|enum)\s+{n}\b"
                    ),
                    &["*.ts", "*.tsx"],
                ),
                (
                    format!(
                        r"(^|\s)(export\s+)?(const|let|var)\s+{n}\b"
                    ),
                    &["*.ts", "*.tsx", "*.js", "*.jsx"],
                ),
            ],

            "python" => vec![
                (
                    format!(r"^\s*(async\s+)?def\s+{n}\b"),
                    &["*.py"],
                ),
                (
                    format!(r"^\s*class\s+{n}\b"),
                    &["*.py"],
                ),
                (
                    format!(r"^\s*{n}\s*="),
                    &["*.py"],
                ),
            ],

            "go" => vec![
                (
                    format!(r"^func(\s+\([^)]*\))?\s+{n}\b"),
                    &["*.go"],
                ),
                (
                    format!(r"^type\s+{n}\b"),
                    &["*.go"],
                ),
                (
                    format!(r"^var\s+{n}\b|^const\s+{n}\b"),
                    &["*.go"],
                ),
            ],

            "java" => vec![
                (
                    format!(
                        r"(public|private|protected|static|final|\s)+\s+{n}\s*\("
                    ),
                    &["*.java"],
                ),
                (
                    format!(
                        r"(public|private|protected|abstract|static|final|\s)+\s+(class|interface|enum|record)\s+{n}\b"
                    ),
                    &["*.java"],
                ),
            ],

            _ => vec![
                (
                    format!(r"\b{n}\b"),
                    &[] as &[&str],
                ),
            ],
        }
    };

    match lang {
        Some(language) => all(language),

        None => {
            let mut out = Vec::new();

            for language in [
                "rust",
                "ts",
                "python",
                "go",
                "java",
            ] {
                out.extend(all(language));
            }

            out
        }
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
