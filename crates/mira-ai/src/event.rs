use mira_core::{ToolCall, ToolCallId};
use serde::{Deserialize, Serialize};

/// Provider-agnostic stream event emitted during a single model turn.
///
/// The harness reassembles a full assistant `Message` from a stream of these.
/// Providers with richer protocols (extended thinking, redacted content) can
/// grow this enum non-breakingly with new variants.
#[derive(Clone, Debug)]
pub enum ChatEvent {
    /// A fragment of assistant free-form text.
    TextDelta(String),
    /// One or more tool calls have finished streaming and can be dispatched.
    ///
    /// Providers stream tool call arguments token-by-token; the client is
    /// responsible for buffering fragments and emitting fully-formed calls
    /// once the turn ends.
    ToolCalls(Vec<ToolCall>),
    /// Per-turn token accounting reported by the provider. For OpenAI-style
    /// streams this arrives in a trailer chunk (`stream_options.include_usage`).
    /// Providers that don't report usage simply never emit this event — the
    /// harness treats absence as zero.
    Usage(TokenUsage),
    /// The turn ended. Any partial state has been flushed via prior events.
    Done(FinishReason),
}

/// Token counts for a single model turn.
///
/// `cached_input_tokens` counts the prompt tokens that hit the provider's
/// prompt cache (they're a subset of `prompt_tokens`, not additional). Kept
/// as `u32` — OpenAI's per-request cap is well under `u32::MAX` and we
/// aggregate on the harness side into `u64` totals.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    #[serde(default)]
    pub prompt_tokens: u32,
    #[serde(default)]
    pub completion_tokens: u32,
    /// Portion of `prompt_tokens` served from the prompt cache. `0` when the
    /// provider doesn't report it.
    #[serde(default)]
    pub cached_input_tokens: u32,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    ToolCalls,
    Length,
    ContentFilter,
    Other,
}

impl FinishReason {
    pub fn from_wire(raw: Option<&str>) -> Self {
        match raw {
            Some("stop") => Self::Stop,
            Some("tool_calls") => Self::ToolCalls,
            Some("length") => Self::Length,
            Some("content_filter") => Self::ContentFilter,
            _ => Self::Other,
        }
    }
}

/// Buffer for reassembling streaming tool call fragments.
///
/// OpenAI-style deltas arrive as:
///   `{"index": 0, "id": "call_..", "function": {"name": "..", "arguments": ".."}}`
/// with `arguments` accumulating character-by-character across many chunks.
/// We index by the delta's `index` field, not `id`, because some providers
/// only send the id on the first fragment.
#[derive(Default)]
pub(crate) struct ToolCallBuffer {
    slots: Vec<PartialCall>,
}

#[derive(Default, Clone)]
struct PartialCall {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

impl ToolCallBuffer {
    pub fn push_delta(
        &mut self,
        index: usize,
        id: Option<&str>,
        name: Option<&str>,
        args_fragment: Option<&str>,
    ) {
        if self.slots.len() <= index {
            self.slots.resize(index + 1, PartialCall::default());
        }
        let slot = &mut self.slots[index];
        if let Some(id) = id {
            slot.id = Some(id.to_owned());
        }
        if let Some(name) = name {
            slot.name = Some(name.to_owned());
        }
        if let Some(frag) = args_fragment {
            slot.arguments.push_str(frag);
        }
    }

    pub fn take(&mut self) -> Vec<ToolCall> {
        std::mem::take(&mut self.slots)
            .into_iter()
            .enumerate()
            .filter_map(|(idx, p)| {
                let name = p.name?;
                let id = p.id.unwrap_or_else(|| format!("call_{idx}"));
                Some(ToolCall {
                    id: ToolCallId::from(id),
                    kind: mira_core::message::ToolCallKind::Function,
                    function: mira_core::message::ToolCallFunction {
                        name,
                        arguments: if p.arguments.is_empty() {
                            "{}".to_owned()
                        } else {
                            p.arguments
                        },
                    },
                })
            })
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
}
