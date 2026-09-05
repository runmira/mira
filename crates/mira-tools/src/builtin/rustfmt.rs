use async_trait::async_trait;
use mira_ai::ToolSpec;
use mira_core::{ToolCall, ToolResult};
use serde::Deserialize;
use serde_json::json;

use crate::context::ToolContext;
use crate::tool::{spec, Action, Tool, ToolError};

/// Format Rust source with `rustfmt` / `cargo fmt`.
///
/// - No `path` → `cargo fmt --all` at the workspace root (handles every
///   crate correctly, respects `rustfmt.toml`).
/// - With `path` → `rustfmt <path>` on that single file.
/// - `check = true` → `--check` flag: report drift without modifying files.
///
/// Reports as [`Action::Bash`] rather than adding a new variant so existing
/// bash rules (e.g. `Bash(cargo fmt:*)`) cover it.
pub struct RustFmt;

#[derive(Deserialize)]
struct Args {
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    check: bool,
}

#[async_trait]
impl Tool for RustFmt {
    fn spec(&self) -> ToolSpec {
        spec(
            "rustfmt",
            "Format Rust source. With no path, runs `cargo fmt --all` at the \
             workspace root. With a path, runs `rustfmt <path>`. Pass \
             `check: true` to report formatting drift without writing.",
            json!({
                "type": "object",
                "properties": {
                    "path":  { "type": "string", "description": "Optional single file to format." },
                    "check": { "type": "boolean", "default": false }
                },
                "additionalProperties": false
            }),
        )
    }

    fn action(&self) -> Action {
        Action::Bash
    }

    /// Expose the actual shell command so bash rules can gate it.
    fn policy_target(&self, call: &ToolCall) -> String {
        let args: Args = call.parse_arguments().unwrap_or(Args {
            path: None,
            check: false,
        });
        build_command(&args)
    }

    async fn invoke(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let args: Args = call.parse_arguments()?;
        let command = build_command(&args);

        let outcome = ctx
            .sandbox
            .run(&command, &ctx.cwd, std::time::Duration::from_secs(120))
            .await
            .map_err(|e| ToolError::Failed(e.to_string()))?;

        let mut body = String::new();
        body.push_str(&format!("$ {command}\n"));
        body.push_str(&format!("exit={}\n", outcome.exit_code));
        if outcome.timed_out {
            body.push_str("(command timed out)\n");
        }
        body.push_str("--- output ---\n");
        body.push_str(&outcome.output);
        Ok(ToolResult::ok(call.id.clone(), body))
    }
}

fn build_command(args: &Args) -> String {
    match (&args.path, args.check) {
        (Some(p), true) => format!("rustfmt --check {}", shell_quote(p)),
        (Some(p), false) => format!("rustfmt {}", shell_quote(p)),
        (None, true) => "cargo fmt --all -- --check".to_owned(),
        (None, false) => "cargo fmt --all".to_owned(),
    }
}

fn shell_quote(s: &str) -> String {
    let escaped = s.replace('\'', "'\\''");
    format!("'{escaped}'")
}
