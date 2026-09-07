use async_trait::async_trait;
use mira_ai::ToolSpec;
use mira_core::{ToolCall, ToolResult};
use serde::Deserialize;
use serde_json::json;
use tokio::fs;

use crate::context::ToolContext;
use crate::tool::{spec, Action, Tool, ToolError};

/// Write a file, creating parents as needed. Overwrites existing content.
pub struct WriteFile;

#[derive(Deserialize)]
struct Args {
    path: String,
    content: String,
}

#[async_trait]
impl Tool for WriteFile {
    fn spec(&self) -> ToolSpec {
        spec(
            "write_file",
            "Create or overwrite a file with the given content. Parent \
             directories are created if missing. Prefer `edit_file` when \
             modifying an existing file.",
            json!({
                "type": "object",
                "properties": {
                    "path":    { "type": "string" },
                    "content": { "type": "string" }
                },
                "required": ["path", "content"],
                "additionalProperties": false
            }),
        )
    }

    fn action(&self) -> Action {
        Action::Write
    }

    async fn invoke(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let args: Args = call.parse_arguments()?;
        let path = ctx
            .resolve(&args.path)
            .ok_or_else(|| ToolError::Failed(format!("path escapes cwd: {}", args.path)))?;

        // Refuse to overwrite a file that was modified externally since we
        // last saw it. Skipped when writing a brand-new path (no watermark
        // recorded there).
        if let Some(g) = &ctx.guard {
            g.check_conflict(&path)
                .await
                .map_err(|e| ToolError::Failed(e.to_string()))?;
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        // Snapshot the pre-image (or record a `create` entry) before writing.
        if let Some(g) = &ctx.guard {
            g.snapshot_before(&path)
                .await
                .map_err(|e| ToolError::Failed(e.to_string()))?;
        }
        fs::write(&path, args.content.as_bytes()).await?;

        Ok(ToolResult::ok(
            call.id.clone(),
            format!("wrote {} bytes to {}", args.content.len(), path.display()),
        ))
    }
}
