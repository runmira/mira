use async_trait::async_trait;
use mira_ai::ToolSpec;
use mira_core::{ToolCall, ToolResult};
use serde::Deserialize;
use serde_json::json;
use tokio::fs;

use crate::context::ToolContext;
use crate::tool::{spec, Action, Tool, ToolError};

/// Read a file from the working tree.
///
/// Line-numbered output so the model can produce line-anchored edits from
/// what it just read.
pub struct ReadFile;

#[derive(Deserialize)]
struct Args {
    path: String,
    #[serde(default)]
    start_line: Option<usize>,
    #[serde(default)]
    end_line: Option<usize>,
}

const MAX_BYTES: usize = 512 * 1024;

#[async_trait]
impl Tool for ReadFile {
    fn spec(&self) -> ToolSpec {
        spec(
            "read_file",
            "Read a text file, returning line-numbered contents. Paths are \
             resolved relative to the repo root. Optionally restrict to a line \
             range with `start_line` and `end_line` (1-indexed, inclusive).",
            json!({
                "type": "object",
                "properties": {
                    "path":       { "type": "string", "description": "File path relative to the repo root." },
                    "start_line": { "type": "integer", "minimum": 1 },
                    "end_line":   { "type": "integer", "minimum": 1 }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        )
    }

    fn action(&self) -> Action {
        Action::Read
    }

    async fn invoke(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let args: Args = call.parse_arguments()?;
        let path = ctx
            .resolve(&args.path)
            .ok_or_else(|| ToolError::Failed(format!("path escapes cwd: {}", args.path)))?;

        let meta = fs::metadata(&path).await?;
        if meta.len() as usize > MAX_BYTES {
            return Err(ToolError::Failed(format!(
                "file too large ({} bytes; limit {MAX_BYTES}). Read a range with start_line/end_line.",
                meta.len()
            )));
        }
        let raw = fs::read_to_string(&path).await?;

        // Stamp a watermark so a later edit_file / write_file can detect
        // out-of-band modifications. Best-effort; no-op when no guard.
        if let Some(g) = &ctx.guard {
            g.record_read(&path).await;
        }

        let start = args.start_line.unwrap_or(1).saturating_sub(1);
        let end = args.end_line.unwrap_or(usize::MAX);

        let mut out = String::with_capacity(raw.len() + raw.lines().count() * 6);
        for (i, line) in raw.lines().enumerate() {
            let n = i + 1;
            if n <= start {
                continue;
            }
            if n > end {
                break;
            }
            out.push_str(&format!("{n:>6}\t{line}\n"));
        }

        Ok(ToolResult::ok(call.id.clone(), out))
    }
}
