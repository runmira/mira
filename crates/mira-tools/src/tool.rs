use async_trait::async_trait;
use mira_ai::ToolSpec;
use mira_core::{ToolCall, ToolResult};
use serde_json::Value;
use thiserror::Error;

use crate::context::ToolContext;

/// Semantic action a tool performs, used by the policy engine to decide
/// whether to allow, prompt, or deny before dispatch.
///
/// This is intentionally coarse. The engine gets richer input (paths, argv)
/// from the tool's arguments; this enum just says which rule family applies.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    Read,
    Edit,
    Write,
    Bash,
    /// Anything that doesn't need gating — a pure computation.
    Pure,
}

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("invalid arguments: {0}")]
    InvalidArgs(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("tool failed: {0}")]
    Failed(String),
}

impl From<serde_json::Error> for ToolError {
    fn from(e: serde_json::Error) -> Self {
        ToolError::InvalidArgs(e.to_string())
    }
}

/// A single, named capability the model can invoke.
///
/// Design notes:
///
/// - `spec()` returns the wire description sent to the model. It's a method
///   rather than a constant so tools can generate schemas dynamically (e.g.
///   inject the allowed root path into a description).
/// - `action()` is what the policy engine reads. It's separate from `spec()`
///   so tools can't accidentally lie to the policy layer through free-form
///   description text.
/// - `invoke()` takes raw JSON args because we pass them straight through
///   from the model. Tools deserialize into their own strongly-typed struct
///   inside `invoke` — that keeps the trait object-safe.
#[async_trait]
pub trait Tool: Send + Sync {
    fn spec(&self) -> ToolSpec;
    fn action(&self) -> Action;

    /// The string the policy engine gates on: a file path for read/edit/write,
    /// the shell command for bash, or empty for pure tools.
    ///
    /// The default implementation extracts `path` or `command` from the call
    /// arguments — enough for the built-ins. Custom tools override when they
    /// use a different arg name.
    fn policy_target(&self, call: &ToolCall) -> String {
        let Ok(v) = serde_json::from_str::<Value>(&call.function.arguments) else {
            return String::new();
        };
        for key in ["path", "command", "target"] {
            if let Some(s) = v.get(key).and_then(Value::as_str) {
                return s.to_owned();
            }
        }
        String::new()
    }

    async fn invoke(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError>;
}

/// Helper: build a `ToolSpec` from name + description + a JSON Schema `Value`.
pub fn spec(name: &str, description: &str, parameters: Value) -> ToolSpec {
    ToolSpec {
        name: name.to_owned(),
        description: description.to_owned(),
        parameters,
    }
}
