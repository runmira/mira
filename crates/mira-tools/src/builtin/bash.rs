
use async_trait::async_trait;
use mira_ai::ToolSpec;
use mira_core::{ToolCall, ToolResult};
use serde::Deserialize;
use serde_json::json;

use crate::context::ToolContext;
use crate::tool::{spec, Action, Tool, ToolError};

/// Run a shell command through the sandbox.
///
/// The command is executed as `bash -lc <command>` inside the session
/// sandbox. The sandbox remains responsible for filesystem, network,
/// credential, timeout, and process isolation.
pub struct Bash;

#[derive(Deserialize)]
struct Args {
    command: String,

    #[serde(default = "default_timeout_ms")]
    timeout_ms: u64,
}

fn default_timeout_ms() -> u64 {
    300_000
}

#[async_trait]
impl Tool for Bash {
    fn spec(&self) -> ToolSpec {
        spec(
            "bash",
            "Run a shell command via `bash -lc` inside the sandbox. \
             Working directory is the current session directory. \
             Combined stdout/stderr is returned. Default timeout is \
             300 seconds; override with `timeout_ms` (maximum 600000).",
            json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string"
                    },
                    "timeout_ms": {
                        "type": "integer",
                        "minimum": 100,
                        "maximum": 600000,
                        "default": 300000
                    }
                },
                "required": ["command"],
                "additionalProperties": false
            }),
        )
    }

    fn action(&self) -> Action {
        Action::Bash
    }

    async fn invoke(
        &self,
        call: &ToolCall,
        ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError> {
        let args: Args = call.parse_arguments()?;

        let timeout_ms = args.timeout_ms.min(600_000);
        let timeout = std::time::Duration::from_millis(timeout_ms);

        // Track filesystem changes caused by the command when the
        // FileGuard is available.
        let pre_bash = ctx
            .guard
            .as_ref()
            .and_then(|guard| guard.pre_bash());

        // Bash is deliberately invoked as a binary with argv.
        //
        // The command string is interpreted only by this explicit shell.
        // It is never concatenated into a larger shell command by Mira.
        let command_args = vec![
            "-lc".to_owned(),
            args.command.clone(),
        ];

        let outcome = ctx
            .sandbox
            .run_with_timeout(
                "bash",
                &command_args,
                &ctx.cwd,
                timeout.as_secs().max(1),
            )
            .await
            .map_err(|e| ToolError::Failed(e.to_string()))?;

        if let (Some(guard), Some(pre)) = (&ctx.guard, &pre_bash) {
            if let Err(e) = guard.record_bash_changes(pre).await {
                tracing::warn!(
                    %e,
                    "post-bash change tracking failed"
                );
            }
        }

        let mut output = String::new();

        output.push_str(&outcome.stdout);

        if !outcome.stderr.is_empty() {
            if !output.is_empty() {
                output.push('\n');
            }

            output.push_str(&outcome.stderr);
        }

        let mut body = String::new();

        match outcome.exit_code {
            Some(code) => {
                body.push_str(&format!("exit={code}\n"));
            }

            None => {
                body.push_str("exit=unknown\n");
            }
        }

        if outcome.timed_out {
            body.push_str("(command timed out)\n");
        }

        body.push_str("--- output ---\n");
        body.push_str(&truncate(&output, 32_000));

        Ok(ToolResult::ok(call.id.clone(), body))
    }
}

fn truncate(s: &str, limit: usize) -> String {
    if s.len() <= limit {
        return s.to_owned();
    }

    let head_end = floor_boundary(s, limit / 2);
    let tail_start =
        ceil_boundary(s, s.len().saturating_sub(limit / 2));

    format!(
        "{}\n... [truncated {} bytes] ...\n{}",
        &s[..head_end],
        s.len() - limit,
        &s[tail_start..]
    )
}

fn floor_boundary(s: &str, at: usize) -> usize {
    let mut i = at.min(s.len());

    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }

    i
}

fn ceil_boundary(s: &str, at: usize) -> usize {
    let mut i = at.min(s.len());

    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }

    i
}

