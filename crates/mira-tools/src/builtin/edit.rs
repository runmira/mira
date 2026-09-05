use async_trait::async_trait;
use mira_ai::ToolSpec;
use mira_core::{ToolCall, ToolResult};
use serde::Deserialize;
use serde_json::json;
use tokio::fs;

use crate::context::ToolContext;
use crate::tool::{spec, Action, Tool, ToolError};

/// Edit a file by exact-string replacement.
///
/// The `old_string` must occur exactly once (unless `replace_all` is true).
/// The model must carry enough context in `old_string` to disambiguate.
pub struct EditFile;

#[derive(Deserialize)]
struct Args {
    path: String,
    old_string: String,
    new_string: String,
    #[serde(default)]
    replace_all: bool,
}

#[async_trait]
impl Tool for EditFile {
    fn spec(&self) -> ToolSpec {
        spec(
            "edit_file",
            "Replace an exact string in a file. `old_string` must appear \
             exactly once unless `replace_all` is true. Read the file first \
             so `old_string` matches byte-for-byte, including indentation.",
            json!({
                "type": "object",
                "properties": {
                    "path":        { "type": "string" },
                    "old_string":  { "type": "string" },
                    "new_string":  { "type": "string" },
                    "replace_all": { "type": "boolean", "default": false }
                },
                "required": ["path", "old_string", "new_string"],
                "additionalProperties": false
            }),
        )
    }

    fn action(&self) -> Action {
        Action::Edit
    }

    async fn invoke(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let args: Args = call.parse_arguments()?;
        let path = ctx
            .resolve(&args.path)
            .ok_or_else(|| ToolError::Failed(format!("path escapes cwd: {}", args.path)))?;

        if args.old_string == args.new_string {
            return Err(ToolError::InvalidArgs(
                "old_string and new_string are identical".into(),
            ));
        }

        let contents = fs::read_to_string(&path).await?;
        let count = contents.matches(&args.old_string).count();

        let updated = match (count, args.replace_all) {
            (0, _) => {
                return Err(ToolError::Failed(format!(
                    "old_string not found in {}",
                    path.display()
                )))
            }
            (n, false) if n > 1 => {
                return Err(ToolError::Failed(format!(
                    "old_string matches {n} times; pass replace_all=true or add context to disambiguate"
                )))
            }
            (_, true) => contents.replace(&args.old_string, &args.new_string),
            (_, false) => contents.replacen(&args.old_string, &args.new_string, 1),
        };

        fs::write(&path, updated.as_bytes()).await?;

        Ok(ToolResult::ok(
            call.id.clone(),
            format!(
                "edited {} ({count} replacement{})",
                path.display(),
                if count == 1 { "" } else { "s" }
            ),
        ))
    }
}
