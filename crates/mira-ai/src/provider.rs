use async_trait::async_trait;
use futures::stream::BoxStream;
use mira_core::Message;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{event::ChatEvent, tool_spec::ToolSpec};

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("provider returned {status}: {body}")]
    Status {
        status: u16,
        body: String,
        /// Seconds from a `Retry-After` header, when the provider sent one.
        retry_after: Option<u64>,
    },

    #[error("stream decode error: {0}")]
    Decode(String),

    #[error("bad configuration: {0}")]
    Config(String),
}

/// Seconds to wait from a `Retry-After` header (the delay form; the
/// HTTP-date form is rare for APIs and ignored).
pub fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()
}

impl From<ProviderError> for mira_core::Error {
    fn from(e: ProviderError) -> Self {
        mira_core::Error::Provider(e.to_string())
    }
}

/// A single completion request.
#[derive(Clone, Debug)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSpec>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    /// Reasoning effort (`"minimal" | "low" | "medium" | "high"`) — passed
    /// through to providers that expose it. `None` skips the field.
    pub reasoning_effort: Option<String>,
    /// Service tier, the other half of "make this fast": OpenAI's
    /// `service_tier` (`"auto" | "default" | "flex" | "priority"`).
    /// Separate from `reasoning_effort` because they trade against
    /// different things — effort trades latency for quality, tier trades
    /// cost for latency — and a model can support either without the other.
    pub service_tier: Option<String>,
    /// Constrain the response shape. When set, the provider is told to
    /// emit JSON matching the schema (OpenAI's `response_format:
    /// json_schema` mode). Callers that want typed results out of a
    /// subagent set this; leave `None` for freeform prose.
    pub response_format: Option<ResponseFormat>,
}

/// How the model's text output should be shaped.
///
/// Providers that don't support structured output silently ignore this
/// (or error at the wire level — the harness surfaces those). Currently
/// only the OpenAI-compatible provider wires it.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResponseFormat {
    /// Freeform JSON — any valid JSON object. Cheaper to enforce than a
    /// full schema; useful when the caller only needs "must be JSON".
    JsonObject,
    /// Strict JSON matching a JSON Schema. `name` is a short identifier
    /// the provider surfaces in errors. `strict: true` asks the provider
    /// to reject drift (unknown fields, missing required keys); providers
    /// that don't support strict fall back to best-effort.
    JsonSchema {
        name: String,
        schema: serde_json::Value,
        #[serde(default)]
        strict: bool,
    },
}

/// One value a model offers for a `select`-shaped descriptor.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OptionChoice {
    /// Wire value, sent back verbatim in a selection.
    pub value: String,
    /// Human label. "Fast" for `service_tier: priority`, etc.
    pub label: String,
    /// Optional note shown under the label, e.g. what it costs you.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// A single adjustable knob on a model.
///
/// The point of this type is that "fast" is not one thing. OpenAI exposes
/// a service tier, Anthropic has no equivalent on the API, and several
/// providers expose nothing at all. A hardcoded "Fast" button would be a
/// lie for most of the catalog, so instead each model advertises the knobs
/// it genuinely supports and the composer renders exactly those. Nothing
/// appears for a model that can't do it.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OptionDescriptor {
    /// A dropdown. Effort levels, service tier, and similar enums.
    Select {
        /// Stable id, e.g. `reasoning_effort`, `service_tier`.
        id: String,
        label: String,
        options: Vec<OptionChoice>,
    },
    /// An on/off switch, e.g. Claude's fast mode.
    Boolean {
        id: String,
        label: String,
        /// Value used when the switch is on.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        on_value: Option<String>,
    },
}

impl OptionDescriptor {
    pub fn id(&self) -> &str {
        match self {
            OptionDescriptor::Select { id, .. } | OptionDescriptor::Boolean { id, .. } => id,
        }
    }
}

/// What a model can be asked to change about a request.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ModelCapabilities {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub option_descriptors: Vec<OptionDescriptor>,
}

/// One entry in a provider's model catalog. `id` is the value you pass as
/// `ChatRequest::model`; the extras are metadata the UI can surface.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Provider-specific bucket (OpenRouter groups by "openai", "anthropic", …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owned_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_length: Option<u32>,
    /// Knobs this model actually supports. Absent for providers that
    /// advertise nothing, in which case the composer hides the section
    /// rather than showing a control that would do nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<ModelCapabilities>,
}

/// Anything that can turn a `ChatRequest` into a stream of `ChatEvent`s.
///
/// Implementations own their HTTP client and auth. The harness holds this
/// behind `Arc<dyn ChatProvider>` so it can be swapped mid-session.
#[async_trait]
pub trait ChatProvider: Send + Sync {
    async fn stream(
        &self,
        request: ChatRequest,
    ) -> Result<BoxStream<'static, Result<ChatEvent, ProviderError>>, ProviderError>;

    /// Fetch the provider's model catalog. Default returns an empty list —
    /// providers that don't expose one (or don't want to) fall back to a
    /// text input on the UI side.
    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        Ok(Vec::new())
    }
}
