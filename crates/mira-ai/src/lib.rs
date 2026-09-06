//! Provider abstraction for the Mira harness.
//!
//! The harness never talks to a specific vendor SDK. It talks to a
//! [`ChatProvider`], which yields a stream of [`ChatEvent`]s. A single
//! OpenAI-compatible implementation ([`openai::OpenAiCompatible`]) covers
//! OpenRouter, Groq, Together, Anthropic's compat endpoint, and local runners
//! that speak the same schema (llama.cpp server, LM Studio, Ollama with the
//! `/v1` shim).
//!
//! Adding a native adapter (e.g. Anthropic Messages) is a new module that
//! implements [`ChatProvider`] — no changes needed in the harness.

pub mod event;
pub mod null;
pub mod openai;
pub mod pricing;
pub mod provider;
pub mod tool_spec;

pub use event::{ChatEvent, FinishReason, TokenUsage};
pub use null::NullProvider;
pub use pricing::{cost_usd, price_for};
pub use provider::{ChatProvider, ChatRequest, ModelInfo, ProviderError};
pub use tool_spec::ToolSpec;
