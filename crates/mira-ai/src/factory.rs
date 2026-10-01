//! Provider-name → concrete adapter dispatch.
//!
//! Every call site that used to instantiate [`openai::OpenAiCompatible`]
//! directly should route through [`build_chat_provider`] instead so the
//! `anthropic` provider name transparently gets the native Messages
//! adapter — same shape of inputs, different wire protocol under the
//! hood.
//!
//! ## How provider routing works
//!
//! This module is the single source of truth for mapping a user-
//! supplied provider name to a concrete [`ChatProvider`] implementation.
//! The dispatch logic lives in [`build_unretried`]:
//! - `"anthropic"` (case-insensitive) → the native Anthropic Messages
//!   API adapter (`Anthropic`), which speaks the Messages API directly.
//! - `"bedrock"` → Amazon Bedrock's Converse API (`Bedrock`), which
//!   handles SigV4 signing and model-id resolution internally.
//! - **anything else** → the OpenAI-compatible adapter
//!   (`OpenAiCompatible`). This covers OpenRouter, Groq, Together,
//!   local `/v1` shims, and Anthropic's own compat endpoint when the
//!   user opts in by naming their provider entry something else.
//!
//! The returned `Arc<dyn ChatProvider>` allows the harness to swap
//! providers mid-session without changing any call-site code.
//!
//! ## Retry policy
//!
//! [`build_chat_provider`] wraps the inner adapter in a [`Retrying`]
//! layer so transient failures (rate limits, overloads, dropped
//! connections) are retried rather than ending the turn. The retry
//! configuration is read from the environment via
//! [`RetryPolicy::from_env`].

use std::sync::Arc;

use crate::anthropic::{Anthropic, AnthropicConfig};
use crate::bedrock::{Bedrock, BedrockConfig};
use crate::openai::{OpenAiCompatible, OpenAiConfig};
use crate::provider::{ChatProvider, ProviderError};
use crate::retry::{RetryPolicy, Retrying};

/// Build the appropriate [`ChatProvider`] for the given provider name.
///
/// * `"anthropic"` (case-insensitive) → native Messages API adapter.
/// * `"bedrock"` → Amazon Bedrock's Converse API (SigV4 or API key).
/// * anything else → OpenAI-compatible adapter (OpenRouter, Groq,
///   Together, local `/v1` shims, Anthropic's own compat endpoint if
///   the user opts in by naming their provider entry something else).
pub fn build_chat_provider(
    name: &str,
    base_url: String,
    api_key: String,
    extra_headers: Vec<(String, String)>,
    prompt_caching: bool,
) -> Result<Arc<dyn ChatProvider>, ProviderError> {
    let inner = build_unretried(name, base_url, api_key, extra_headers, prompt_caching)?;
    // Rate limits, overloads and dropped connections get retried rather
    // than ending the turn.
    Ok(Arc::new(Retrying::new(inner, RetryPolicy::from_env())))
}

fn build_unretried(
    name: &str,
    base_url: String,
    api_key: String,
    extra_headers: Vec<(String, String)>,
    prompt_caching: bool,
) -> Result<Arc<dyn ChatProvider>, ProviderError> {
    if name.eq_ignore_ascii_case("anthropic") {
        let p = Anthropic::new(AnthropicConfig {
            base_url,
            api_key,
            extra_headers,
            prompt_caching,
        })?;
        return Ok(Arc::new(p));
    }
    if name.eq_ignore_ascii_case("bedrock") {
        return Ok(Arc::new(Bedrock::new(BedrockConfig { base_url, api_key })?));
    }
    let p = OpenAiCompatible::new(OpenAiConfig {
        base_url,
        api_key,
        extra_headers,
        prompt_caching,
    })?;
    Ok(Arc::new(p))
}
