//! `repo_map`: the repository map (see [`crate::repo_map`]) for any
//! directory, in more detail than the summary in the system prompt.

use async_trait::async_trait;
use mira_ai::ToolSpec;
use mira_core::{ToolCall, ToolResult};
use serde::Deserialize;
use serde_json::json;

use crate::context::ToolContext;
use crate::repo_map::RepoMap;
use crate::tool::{spec, Action, Tool, ToolError};

pub struct RepoMapTool;

#[derive(Deserialize)]
struct Args {
    #[serde(default)]
    path: String,
    #[serde(default = "default_tokens")]
    max_tokens: usize,
}

fn default_tokens() -> usize {
    4_000
}

#[async_trait]
impl Tool for RepoMapTool {
    fn spec(&self) -> ToolSpec {
        spec(
            "repo_map",
            "Map of the repository's source files: what each defines, the \
             definitions the rest of the code uses most listed first. Pass \
             `path` to see one directory in detail. Use it to find where \
             something lives before grepping or reading files one by one.",
            json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Directory (or file) to map, relative to the working directory. Empty for the whole repository."
                    },
                    "max_tokens": {
                        "type": "integer",
                        "description": "Roughly how long the map may be. Default 4000."
                    }
                },
                "additionalProperties": false
            }),
        )
    }

    fn action(&self) -> Action {
        Action::Read
    }

    async fn invoke(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let args: Args = call.parse_arguments()?;
        if ctx.resolve(&args.path).is_none() {
            return Err(ToolError::Failed(format!(
                "path escapes cwd: {}",
                args.path
            )));
        }
        let root = ctx.cwd.clone();
        let map = tokio::task::spawn_blocking(move || RepoMap::build(&root))
            .await
            .map_err(|e| ToolError::Failed(format!("repo map: {e}")))?
            .map_err(|e| ToolError::Failed(format!("repo map: {e}")))?;
        // Members (methods) too when looking at one directory.
        let per_file = if args.path.is_empty() { 8 } else { 20 };
        let body = map.render(&args.path, args.max_tokens.clamp(500, 20_000), per_file);
        Ok(ToolResult::ok(call.id.clone(), body))
    }
}
