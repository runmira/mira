use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::id::ToolCallId;

/// Who authored a message.
///
/// Matches the OpenAI Chat Completions vocabulary so wire serialization is
/// direct; providers that differ (e.g. Anthropic) can map at their adapter.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// A single turn in the conversation.
///
/// An assistant message may carry either free-form `content`, one or more
/// `tool_calls`, or both (some providers emit text before requesting a tool).
/// A tool message MUST carry `tool_call_id` referencing the assistant call it
/// answers.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<ToolCallId>,

    /// Optional human-readable label — the assistant's `name` field in the
    /// OpenAI schema. Unused by the harness today but kept for provider parity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self::text(Role::System, content)
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::text(Role::User, content)
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self::text(Role::Assistant, content)
    }

    /// Assistant turn that only requests tool calls (no free-form content).
    pub fn assistant_calls(calls: Vec<ToolCall>) -> Self {
        Self {
            role: Role::Assistant,
            content: None,
            tool_calls: calls,
            tool_call_id: None,
            name: None,
        }
    }

    /// A tool result answering a specific `ToolCall`.
    pub fn tool(call_id: ToolCallId, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: Some(content.into()),
            tool_calls: Vec::new(),
            tool_call_id: Some(call_id),
            name: None,
        }
    }

    fn text(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: Some(content.into()),
            tool_calls: Vec::new(),
            tool_call_id: None,
            name: None,
        }
    }
}

/// A single tool invocation requested by the model.
///
/// `arguments` is a JSON string on the wire (OpenAI convention). We keep the
/// same shape here so serialization is transparent; tool impls parse it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: ToolCallId,

    #[serde(rename = "type")]
    pub kind: ToolCallKind,

    pub function: ToolCallFunction,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallKind {
    Function,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCallFunction {
    pub name: String,
    /// JSON-encoded argument object. Kept as a string to match the wire and to
    /// preserve the exact bytes the model produced (useful for debugging).
    pub arguments: String,
}

impl ToolCall {
    /// Parse `arguments` as strongly-typed JSON.
    pub fn parse_arguments<T: for<'de> Deserialize<'de>>(&self) -> serde_json::Result<T> {
        serde_json::from_str(&self.function.arguments)
    }
}

/// Structured result of a tool invocation, produced by tool implementations.
///
/// The harness serializes this into a tool `Message` before feeding it back
/// to the model. Keeping a struct (rather than plain string) leaves room for
/// richer surfaces later: attachments, artifacts, structured errors.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolResult {
    pub call_id: ToolCallId,
    pub content: String,
    #[serde(default)]
    pub is_error: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl ToolResult {
    pub fn ok(call_id: ToolCallId, content: impl Into<String>) -> Self {
        Self {
            call_id,
            content: content.into(),
            is_error: false,
            data: None,
        }
    }

    pub fn err(call_id: ToolCallId, content: impl Into<String>) -> Self {
        Self {
            call_id,
            content: content.into(),
            is_error: true,
            data: None,
        }
    }
}
