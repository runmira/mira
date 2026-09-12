//! Provider abstraction for the Mira harness.
//!
//! The harness never talks to a specific vendor SDK. It talks to a
//! [`ChatProvider`], which yields a stream of [`ChatEvent`]s. Two concrete
//! implementations ship in-tree:
//!
//! * [`openai::OpenAiCompatible`] — covers OpenAI, OpenRouter, Groq, Together,
//!   local runners (llama.cpp, LM Studio, Ollama `/v1`), and Anthropic's
//!   OpenAI-compat endpoint.
//! * [`anthropic::Anthropic`] — native Messages API. Used when the user
//!   selects the `anthropic` provider so we get real `cache_control`,
//!   extended thinking, and split cached-token metrics.
//!
//! Adding a new native adapter is a new module that implements
//! [`ChatProvider`] — no changes needed in the harness.

pub mod anthropic;
pub mod event;
pub mod factory;
pub mod null;
pub mod openai;
pub mod pricing;
pub mod provider;
pub mod tool_spec;

pub use event::{ChatEvent, FinishReason, TokenUsage};
pub use factory::build_chat_provider;
pub use null::NullProvider;
pub use pricing::{cost_usd, price_for};
pub use provider::{ChatProvider, ChatRequest, ModelInfo, ProviderError, ResponseFormat};
pub use tool_spec::ToolSpec;
