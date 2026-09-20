
use async_trait::async_trait;
use mira_ai::ToolSpec;
use mira_core::{ToolCall, ToolResult};
use serde::Deserialize;
use serde_json::json;

use crate::context::ToolContext;
use crate::tool::{spec, Action, Tool, ToolError};

/// Search the repo with ripgrep.
///
/// Arguments are passed directly to `rg` rather than interpolated into a
/// shell command. Filesystem and process access are controlled by the
/// session sandbox.
pub struct Grep;

#[derive(Deserialize)]
struct Args {
    pattern: String,

    #[serde(default)]
    path: Option<String>,

    #[serde(default)]
    glob: Option<String>,

    #[serde(default)]
    case_insensitive: bool,

    #[serde(default = "default_max")]
    max_results: usize,
}

fn default_max() -> usize {
    200
}

#[async_trait]
impl Tool for Grep {
    fn spec(&self) -> ToolSpec {
        spec(
            "grep",
            "Search for a regex pattern across the repo using ripgrep. \
             Returns matching lines with file:line prefixes.",
            json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string"
                    },
                    "path": {
                        "type": "string",
                        "description": "Subdirectory to restrict to."
                    },
                    "glob": {
                        "type": "string",
                        "description": "Ripgrep glob filter, e.g. `*.rs`."
                    },
                    "case_insensitive": {
                        "type": "boolean",
                        "default": false
                    },
                    "max_results": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 2000,
                        "default": 200
                    }
                },
                "required": ["pattern"],
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

        if args.max_results == 0 {
            return Err(ToolError::Failed(
                "max_results must be greater than zero".to_owned(),
            ));
        }

        let max_results = args.max_results.min(2000);

      

        let mut command_args = vec![
            "--line-number".to_owned(),
            "--no-heading".to_owned(),
            "--color".to_owned(),
            "never".to_owned(),
            "--max-count".to_owned(),
            max_results.to_string(),
        ];

        if args.case_insensitive {
            command_args.push("--ignore-case".to_owned());
        }

        if let Some(glob) = &args.glob {
            command_args.push("--glob".to_owned());
            command_args.push(glob.clone());
        }

        // Pattern is a separate argv element.
        // No shell quoting or interpolation is required.
        command_args.push(args.pattern.clone());

        // Resolve paths through ToolContext so the search cannot escape
        // Mira's repository boundary.
        if let Some(path) = &args.path {
            let resolved = ctx
                .resolve(path)
                .ok_or_else(|| {
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
            let mut body = format!(
                "search timed out after 30 seconds for /{}/",
                args.pattern
            );

            if !outcome.stdout.is_empty() {
                body.push('\n');
                body.push_str(&outcome.stdout);
            }

            if !outcome.stderr.is_empty() {
                body.push('\n');
                body.push_str(&outcome.stderr);
            }

            return Ok(ToolResult::ok(call.id.clone(), body));
        }

        // ripgrep exit codes:
        //   0 = matches found
        //   1 = no matches
        //   2 = error
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
                format!("no matches for /{}/", args.pattern),
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
                "ripgrep terminated without an exit code".to_owned(),
            )),
        }
    }
}
