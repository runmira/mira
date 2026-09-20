
use async_trait::async_trait;
use mira_ai::ToolSpec;
use mira_core::{ToolCall, ToolResult};
use serde::Deserialize;
use serde_json::json;

use crate::context::ToolContext;
use crate::tool::{spec, Action, Tool, ToolError};

/// Format Rust source with `rustfmt` / `cargo fmt`.
///
/// - No `path` → `cargo fmt --all` at the current workspace directory.
/// - With `path` → `rustfmt <path>` on that single file.
/// - `check = true` → report formatting drift without modifying files.
///
/// Command execution is delegated entirely to the session sandbox.
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
             current workspace directory. With a path, runs `rustfmt <path>`. \
             Pass `check: true` to report formatting drift without writing.",
            json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Optional single file to format."
                    },
                    "check": {
                        "type": "boolean",
                        "default": false
                    }
                },
                "additionalProperties": false
            }),
        )
    }

    fn action(&self) -> Action {
        Action::Bash
    }

    fn policy_target(&self, call: &ToolCall) -> String {
        let args: Args = call.parse_arguments().unwrap_or(Args {
            path: None,
            check: false,
        });

        build_display_command(&args)
    }

    async fn invoke(
        &self,
        call: &ToolCall,
        ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let args: Args = call.parse_arguments()?;

        let (binary, command_args) = build_command(&args, ctx)?;

        let outcome = ctx
            .sandbox
            .run_with_timeout(
                binary,
                &command_args,
                &ctx.cwd,
                120,
            )
            .await
            .map_err(|e| ToolError::Failed(e.to_string()))?;

        let command = build_display_command(&args);

        let mut body = String::new();

        body.push_str(&format!("$ {command}\n"));

        match outcome.exit_code {
            Some(code) => body.push_str(&format!("exit={code}\n")),
            None => body.push_str("exit=unknown\n"),
        }

        if outcome.timed_out {
            body.push_str("(command timed out)\n");
        }

        body.push_str("--- output ---\n");
        body.push_str(&outcome.stdout);

        if !outcome.stderr.is_empty() {
            if !outcome.stdout.is_empty() {
                body.push('\n');
            }

            body.push_str(&outcome.stderr);
        }

        Ok(ToolResult::ok(call.id.clone(), body))
    }
}

fn build_command(
    args: &Args,
    ctx: &ToolContext,
) -> Result<(&'static str, Vec<String>), ToolError> {
    match &args.path {
        Some(path) => {
            let resolved = ctx.resolve(path).ok_or_else(|| {
                ToolError::Failed(format!(
                    "path escapes repository: {path}"
                ))
            })?;

            let mut command_args = Vec::new();

            if args.check {
                command_args.push("--check".to_owned());
            }

            command_args.push(resolved.to_string_lossy().into_owned());

            Ok(("rustfmt", command_args))
        }

        None => {
            let mut command_args = vec![
                "fmt".to_owned(),
                "--all".to_owned(),
            ];

            if args.check {
                command_args.push("--".to_owned());
                command_args.push("--check".to_owned());
            }

            Ok(("cargo", command_args))
        }
    }
}

fn build_display_command(args: &Args) -> String {
    match (&args.path, args.check) {
        (Some(path), true) => {
            format!("rustfmt --check {}", display_quote(path))
        }

        (Some(path), false) => {
            format!("rustfmt {}", display_quote(path))
        }

        (None, true) => {
            "cargo fmt --all -- --check".to_owned()
        }

        (None, false) => {
            "cargo fmt --all".to_owned()
        }
    }
}

fn display_quote(s: &str) -> String {
    let escaped = s.replace('\'', "'\\''");
    format!("'{escaped}'")
}
