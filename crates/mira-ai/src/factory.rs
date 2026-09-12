//! Provider-name → concrete adapter dispatch.
//!
//! Every call site that used to instantiate [`openai::OpenAiCompatible`]
//! directly should route through [`build_chat_provider`] instead so the
//! `anthropic` provider name transparently gets the native Messages
//! adapter — same shape of inputs, different wire protocol under the
//! hood.

use std::sync::Arc;

use crate::anthropic::{Anthropic, AnthropicConfig};
use crate::openai::{OpenAiCompatible, OpenAiConfig};
use crate::provider::{ChatProvider, ProviderError};

/// Build the appropriate [`ChatProvider`] for the given provider name.
///
/// * `"anthropic"` (case-insensitive) → native Messages API adapter.
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
    if name.eq_ignore_ascii_case("anthropic") {
        let p = Anthropic::new(AnthropicConfig {
            base_url,
            api_key,
            extra_headers,
            prompt_caching,
        })?;
        return Ok(Arc::new(p));
    }
    let p = OpenAiCompatible::new(OpenAiConfig {
        base_url,
        api_key,
        extra_headers,
        prompt_caching,
    })?;
    Ok(Arc::new(p))
}
