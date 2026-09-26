//! Provider abstraction for the Mira harness.
//!
//! The harness never talks to a specific vendor SDK. It talks to a
//! [`ChatProvider`], which yields a stream of [`ChatEvent`]s. Two concrete
//! implementations ship in-tree:
//!
//! NOTE(mira): this module is intentionally vendor-agnostic so new providers
//! can be added as isolated crates without touching the harness core.
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
//!
//! Entry point for the provider abstraction; keep this module free of
//! vendor-specific logic so adapters stay isolated.

pub mod anthropic;
pub mod bedrock;
pub mod event;
pub mod factory;
pub mod null;
pub mod openai;
pub mod pricing;
pub mod provider;
pub mod ratelimit;
pub mod retry;
pub mod tool_spec;
pub mod typesafe;

pub use event::{ChatEvent, FinishReason, TokenUsage};
pub use factory::build_chat_provider;
pub use null::NullProvider;
pub use pricing::{cost_usd, price_for};
pub use provider::{ChatProvider, ChatRequest, ModelInfo, ProviderError, ResponseFormat};
pub use ratelimit::{Bucket, RateLimit};
pub use retry::{RetryPolicy, Retrying};
pub use tool_spec::ToolSpec;
