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

    /// Images attached to this message — today only tool results carry
    /// them (screenshots from the `computer` / `browser` tools). Providers
    /// map them to their native image blocks; ones without vision support
    /// drop them. Empty for every text-only message, and skipped on the
    /// wire so older persisted sessions round-trip unchanged.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<ImageData>,

    /// The model's reasoning ("extended thinking") for an assistant turn,
    /// in the order the provider streamed it. Kept so providers that
    /// require it can replay it — Anthropic rejects a tool-use loop whose
    /// previous assistant turn drops its signed `thinking` block — and so
    /// a resumed transcript can show what the model was thinking. Empty
    /// for every other message, and skipped on the wire so older
    /// persisted sessions round-trip unchanged.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasoning: Vec<ReasoningBlock>,
}

/// One block of model reasoning attached to an assistant [`Message`].
///
/// `text` is the human-readable (possibly summarized) thinking. Anthropic
/// signs each block — `signature` must be sent back verbatim for the
/// block to be accepted on replay — and may return a `redacted` block
/// whose content is encrypted: `text` is empty and `redacted` carries the
/// opaque payload. OpenAI-compatible providers (DeepSeek, OpenRouter, …)
/// stream unsigned text only; those blocks are display-only and never
/// replayed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReasoningBlock {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redacted: Option<String>,
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
            images: Vec::new(),
            reasoning: Vec::new(),
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
            images: Vec::new(),
            reasoning: Vec::new(),
        }
    }

    /// Attach images (builder-style). Used for tool results that carry
    /// screenshots.
    pub fn with_images(mut self, images: Vec<ImageData>) -> Self {
        self.images = images;
        self
    }

    fn text(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: Some(content.into()),
            tool_calls: Vec::new(),
            tool_call_id: None,
            name: None,
            images: Vec::new(),
            reasoning: Vec::new(),
        }
    }
}

/// A base64-encoded image attached to a message.
///
/// `media_type` is a MIME type (`image/png`, `image/jpeg`, …); `data` is
/// the standard-alphabet base64 payload with no `data:` prefix — the shape
/// both Anthropic's `image` block and OpenAI's data-URL `image_url` want.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageData {
    pub media_type: String,
    pub data: String,
}

impl ImageData {
    pub fn png(base64: impl Into<String>) -> Self {
        Self {
            media_type: "image/png".to_owned(),
            data: base64.into(),
        }
    }

    /// `data:<mime>;base64,<payload>` — the OpenAI `image_url` form.
    pub fn data_url(&self) -> String {
        format!("data:{};base64,{}", self.media_type, self.data)
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
    /// Images the model should see alongside `content` (screenshots).
    /// The harness copies them onto the tool `Message` it records.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<ImageData>,
}

impl ToolResult {
    pub fn ok(call_id: ToolCallId, content: impl Into<String>) -> Self {
        Self {
            call_id,
            content: content.into(),
            is_error: false,
            data: None,
            images: Vec::new(),
        }
    }

    pub fn err(call_id: ToolCallId, content: impl Into<String>) -> Self {
        Self {
            call_id,
            content: content.into(),
            is_error: true,
            data: None,
            images: Vec::new(),
        }
    }

    /// Attach images (builder-style).
    pub fn with_images(mut self, images: Vec<ImageData>) -> Self {
        self.images = images;
        self
    }
}
