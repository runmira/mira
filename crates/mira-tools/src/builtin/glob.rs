use std::path::{Path, PathBuf};

use async_trait::async_trait;
use glob::{glob_with, MatchOptions, Pattern};
use mira_ai::ToolSpec;
use mira_core::{ToolCall, ToolResult};
use serde::Deserialize;
use serde_json::json;

use crate::context::ToolContext;
use crate::tool::{spec, Action, Tool, ToolError};

/// List paths that match a shell glob pattern.
///
/// Complements `grep`: grep answers "where does this text occur?",
/// glob answers "which files exist that match this pattern?". Patterns are
/// rooted at the repo cwd unless absolute; hidden files and common build
/// directories are skipped by default so `**/*.rs` doesn't drown the model
/// in `target/` output.
pub struct Glob;

#[derive(Deserialize)]
struct Args {
    pattern: String,
    #[serde(default = "default_true")]
    case_sensitive: bool,
    #[serde(default)]
    include_hidden: bool,
    #[serde(default)]
    include_ignored: bool,
    #[serde(default = "default_max")]
    max_results: usize,
}

fn default_true() -> bool {
    true
}
fn default_max() -> usize {
    500
}

/// Directories that are almost always noise for a coding agent. Filtered by
/// path component match, so nested `node_modules` is caught too.
const NOISY_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    "__pycache__",
    ".venv",
    "venv",
    ".mypy_cache",
    ".pytest_cache",
    ".tox",
    ".gradle",
    ".idea",
];

#[async_trait]
impl Tool for Glob {
    fn spec(&self) -> ToolSpec {
        spec(
            "glob",
            "List files matching a shell glob pattern (e.g. `**/*.rs`, \
             `src/**/mod.rs`, `**/package.json`). Rooted at the repo. \
             Hidden files and common build directories (`target`, \
             `node_modules`, `dist`, ...) are skipped by default — pass \
             `include_ignored: true` to include them.",
            json!({
                "type": "object",
                "properties": {
                    "pattern":         { "type": "string" },
                    "case_sensitive":  { "type": "boolean", "default": true },
                    "include_hidden":  { "type": "boolean", "default": false },
                    "include_ignored": { "type": "boolean", "default": false },
                    "max_results":     { "type": "integer", "minimum": 1, "maximum": 5000, "default": 500 }
                },
                "required": ["pattern"],
                "additionalProperties": false
            }),
        )
    }

    fn action(&self) -> Action {
        Action::Read
    }

    async fn invoke(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let args: Args = call.parse_arguments()?;

        // Root the pattern at cwd if it's relative. We escape the cwd portion
        // so a literal `[` or `*` in the repo path doesn't become a wildcard.
        let expanded = shellexpand::tilde(&args.pattern).into_owned();
        let full_pattern = if Path::new(&expanded).is_absolute() {
            expanded
        } else {
            let escaped_root = Pattern::escape(&ctx.cwd.to_string_lossy());
            format!("{escaped_root}/{expanded}")
        };

        let options = MatchOptions {
            case_sensitive: args.case_sensitive,
            require_literal_separator: false,
            require_literal_leading_dot: !args.include_hidden,
        };

        let cwd = ctx.cwd.clone();
        let include_ignored = args.include_ignored;
        let limit = args.max_results;

        let (matches, hit_limit) = tokio::task::spawn_blocking(move || {
            let paths = glob_with(&full_pattern, options).map_err(|e| e.to_string())?;
            let mut out: Vec<PathBuf> = Vec::new();
            let mut hit = false;
            for entry in paths {
                let Ok(path) = entry else { continue };
                // Confine to cwd. If the user passed an absolute pattern
                // that escapes, we silently drop it — same posture as read.
                let rel = match path.strip_prefix(&cwd) {
                    Ok(r) => r.to_path_buf(),
                    Err(_) => continue,
                };
                if !include_ignored && is_noisy(&rel) {
                    continue;
                }
                out.push(rel);
                if out.len() >= limit {
                    hit = true;
                    break;
                }
            }
            Ok::<_, String>((out, hit))
        })
        .await
        .map_err(|e| ToolError::Failed(e.to_string()))?
        .map_err(ToolError::Failed)?;

        let body = if matches.is_empty() {
            format!("no matches for `{}`", args.pattern)
        } else {
            let mut s = String::with_capacity(matches.len() * 40);
            for m in &matches {
                s.push_str(&m.to_string_lossy());
                s.push('\n');
            }
            if hit_limit {
                s.push_str(&format!("(truncated at {limit} results)\n"));
            }
            s
        };
        Ok(ToolResult::ok(call.id.clone(), body))
    }
}

fn is_noisy(rel: &Path) -> bool {
    rel.components().any(|c| {
        matches!(
            c,
            std::path::Component::Normal(name)
                if NOISY_DIRS.iter().any(|d| name == std::ffi::OsStr::new(d))
        )
    })
}
