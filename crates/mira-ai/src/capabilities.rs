//! Per-model capability descriptors.
//!
//! A model advertises the request knobs it genuinely accepts, and the
//! composer renders exactly those. The alternative — a hardcoded "Fast"
//! button — is a lie for most of the catalog: OpenAI has a service tier,
//! Anthropic's API has no equivalent, and plenty of models expose neither.
//!
//! Rules here are deliberately conservative. A missing descriptor means the
//! control is hidden, which is a small loss; a wrong one means the control
//! appears and the request is either ignored or rejected upstream, which is
//! worse. Where a provider silently drops a field we don't advertise it.
//!
//! Note that most OpenAI-compatible providers (OpenRouter, Groq, Together,
//! and Google's compat endpoint) all reach [`openai_compatible`], so the
//! provider identity — not just the model id — decides what we claim.

use crate::provider::{ModelCapabilities, OptionChoice, OptionDescriptor};

/// Reasoning effort levels, matching OpenAI's `reasoning_effort` values.
/// `off` is Mira's sentinel for "omit the field entirely", which is what
/// non-reasoning models and non-thinking clients need.
pub const EFFORT_CHOICES: &[(&str, &str)] = &[
    ("off", "Off"),
    ("minimal", "Minimal"),
    ("low", "Low"),
    ("medium", "Medium"),
    ("high", "High"),
];

/// Service tiers. `priority` is the one people mean by "fast" — lower
/// latency in exchange for a higher per-token price.
pub const TIER_CHOICES: &[(&str, &str, &str)] = &[
    ("auto", "Auto", "Let the provider route"),
    ("default", "Default", ""),
    ("flex", "Flex", "Cheaper, occasionally slower"),
    ("priority", "Fast", "Lower latency, costs more"),
];

fn effort_descriptor() -> OptionDescriptor {
    OptionDescriptor::Select {
        id: "reasoning_effort".into(),
        label: "Reasoning".into(),
        options: EFFORT_CHOICES
            .iter()
            .map(|(value, label)| OptionChoice {
                value: (*value).to_string(),
                label: (*label).to_string(),
                hint: None,
            })
            .collect(),
    }
}

fn tier_descriptor() -> OptionDescriptor {
    OptionDescriptor::Select {
        id: "service_tier".into(),
        label: "Service tier".into(),
        options: TIER_CHOICES
            .iter()
            .map(|(value, label, hint)| OptionChoice {
                value: (*value).to_string(),
                label: (*label).to_string(),
                hint: (!hint.is_empty()).then(|| (*hint).to_string()),
            })
            .collect(),
    }
}

/// OpenAI's own reasoning families. `o1`/`o3`/`o4` and `gpt-5` take
/// `reasoning_effort`; everything older does not, and sending it is an error
/// rather than a no-op.
fn is_openai_reasoning_model(id: &str) -> bool {
    let id = id.to_ascii_lowercase();
    let bare = id.rsplit('/').next().unwrap_or(&id);
    bare.starts_with("gpt-5")
        || bare.starts_with("o1")
        || bare.starts_with("o3")
        || bare.starts_with("o4")
        || bare.starts_with("codex")
}

/// Google's thinking models, reached through the OpenAI-compat endpoint.
fn is_google_thinking_model(id: &str) -> bool {
    let id = id.to_ascii_lowercase();
    id.contains("gemini-2.5") || id.contains("gemini-3")
}

/// Reasoning families that show up behind OpenRouter. OpenRouter normalizes
/// a single `reasoning.effort` object across them, so one descriptor is
/// honest for all of these.
fn is_router_reasoning_model(id: &str) -> bool {
    let id = id.to_ascii_lowercase();
    [
        "gpt-5", "o1-", "o3-", "o4-", "claude-", "gpt-oss", "qwen3", "deepseek-r1", "kimi",
    ]
    .iter()
    .any(|needle| id.contains(needle))
}

/// OpenAI's tier-aware families. `service_tier` is a first-class field on
/// these; it is not on Gemini, Bedrock, or most compat providers, so we
/// only offer it when we know the endpoint is OpenAI's own.
fn is_openai_tier_model(id: &str) -> bool {
    let id = id.to_ascii_lowercase();
    let bare = id.rsplit('/').next().unwrap_or(&id);
    bare.starts_with("gpt-5")
        || bare.starts_with("gpt-4.1")
        || bare.starts_with("o3")
        || bare.starts_with("o4")
}

/// Which OpenAI-compatible provider is actually on the other end.
///
/// This has to be an input rather than inferred from the model id. OpenRouter
/// serves `openai/gpt-5`, an id indistinguishable from OpenAI's own — but
/// OpenRouter has no `service_tier` field, so offering the "Fast" tier there
/// would render a control that silently does nothing. Same model id,
/// different capability, and only the endpoint tells them apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompatFlavor {
    /// `api.openai.com` — the only endpoint with a real service tier.
    OpenAi,
    /// OpenRouter and other aggregators. Reasoning effort yes, tier no.
    Router,
    /// Google's `generativelanguage` compat endpoint.
    Google,
    /// Some other compatible endpoint we know nothing about. Conservative.
    Other,
}

impl CompatFlavor {
    /// Classify from the configured base URL.
    pub fn from_base_url(base_url: &str) -> Self {
        let url = base_url.to_ascii_lowercase();
        if url.contains("api.openai.com") {
            CompatFlavor::OpenAi
        } else if url.contains("generativelanguage.googleapis.com") {
            CompatFlavor::Google
        } else if url.contains("openrouter.ai") {
            CompatFlavor::Router
        } else {
            // Groq, Together, Fireworks, vLLM, Ollama… all speak the wire
            // format but none of them forward `service_tier`.
            CompatFlavor::Other
        }
    }
}

/// Descriptors for anything spoken to over the OpenAI-compatible wire.
///
/// The flavour decides whether the OpenAI-only service tier is offered; the
/// model id decides whether reasoning effort is.
pub fn openai_compatible(model_id: &str, flavor: CompatFlavor) -> ModelCapabilities {
    let mut option_descriptors = Vec::new();

    let reasoning = match flavor {
        CompatFlavor::Google => is_google_thinking_model(model_id),
        CompatFlavor::OpenAi | CompatFlavor::Router | CompatFlavor::Other => {
            is_openai_reasoning_model(model_id) || is_router_reasoning_model(model_id)
        }
    };
    if reasoning {
        option_descriptors.push(effort_descriptor());
    }

    // Only OpenAI's own endpoint forwards `service_tier`.
    if flavor == CompatFlavor::OpenAi && is_openai_tier_model(model_id) {
        option_descriptors.push(tier_descriptor());
    }

    ModelCapabilities { option_descriptors }
}

/// Anthropic's native API. Extended thinking is budget-based, and
/// [`crate::anthropic`] maps an effort level onto a token budget, so the
/// control the user sees is the same "Reasoning" one. There is no service
/// tier on this API, so nothing else is offered.
pub fn anthropic(model_id: &str) -> ModelCapabilities {
    let id = model_id.to_ascii_lowercase();
    // Extended thinking arrived with the 3-series and is on every model
    // since; the 2.x family does not accept a thinking budget at all.
    let thinking = id.contains("claude-3")
        || id.contains("claude-4")
        || id.contains("claude-sonnet-4")
        || id.contains("claude-opus-4")
        || id.contains("claude-haiku-4")
        || id.contains("sonnet-4")
        || id.contains("opus-4")
        || id.contains("haiku-4");
    ModelCapabilities {
        option_descriptors: if thinking {
            vec![effort_descriptor()]
        } else {
            Vec::new()
        },
    }
}

/// Bedrock hosts other people's models behind its own request shape, and
/// neither `reasoning_effort` nor `service_tier` is a field it forwards
/// generically. We advertise nothing rather than guess.
pub fn bedrock() -> ModelCapabilities {
    ModelCapabilities::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(caps: &ModelCapabilities) -> Vec<&str> {
        caps.option_descriptors.iter().map(|d| d.id()).collect()
    }

    #[test]
    fn openai_reasoning_gets_effort_and_tier() {
        let caps = openai_compatible("gpt-5", CompatFlavor::OpenAi);
        assert_eq!(ids(&caps), vec!["reasoning_effort", "service_tier"]);
    }

    #[test]
    fn openai_tier_only_model_gets_tier_but_no_effort() {
        // gpt-4.1 is tier-aware but not a reasoning model.
        let caps = openai_compatible("gpt-4.1", CompatFlavor::OpenAi);
        assert_eq!(ids(&caps), vec!["service_tier"]);
    }

    #[test]
    fn older_openai_models_get_nothing() {
        assert!(ids(&openai_compatible("gpt-4o", CompatFlavor::OpenAi)).is_empty());
        assert!(ids(&openai_compatible("gpt-3.5-turbo", CompatFlavor::OpenAi)).is_empty());
    }

    #[test]
    fn google_gets_effort_but_never_the_openai_tier() {
        let caps = openai_compatible("models/gemini-2.5-pro", CompatFlavor::Google);
        assert_eq!(ids(&caps), vec!["reasoning_effort"]);
        // Even when the id would otherwise look tier-aware.
        let spoofed = openai_compatible("gpt-5-pro-preview", CompatFlavor::Google);
        assert!(!ids(&spoofed).contains(&"service_tier"));
    }

    /// The case that motivated making the flavour an input: OpenRouter
    /// serves `openai/gpt-5`, the same id OpenAI serves, but has no
    /// `service_tier` field to forward.
    #[test]
    fn openrouter_gets_effort_never_the_tier_even_for_a_gpt5_id() {
        let caps = openai_compatible("openai/gpt-5", CompatFlavor::Router);
        assert_eq!(ids(&caps), vec!["reasoning_effort"]);
    }

    #[test]
    fn openrouter_reasoning_families_get_effort_only() {
        assert_eq!(
            ids(&openai_compatible(
                "anthropic/claude-sonnet-4",
                CompatFlavor::Router
            )),
            vec!["reasoning_effort"]
        );
        assert_eq!(
            ids(&openai_compatible("deepseek/deepseek-r1", CompatFlavor::Router)),
            vec!["reasoning_effort"]
        );
    }

    #[test]
    fn unknown_compat_endpoints_are_treated_conservatively() {
        // Groq and friends: wire-compatible, but no service tier.
        let caps = openai_compatible("llama-3.3-70b-versatile", CompatFlavor::Other);
        assert!(ids(&caps).is_empty());
    }

    #[test]
    fn flavor_classification() {
        assert_eq!(
            CompatFlavor::from_base_url("https://api.openai.com/v1"),
            CompatFlavor::OpenAi
        );
        assert_eq!(
            CompatFlavor::from_base_url("https://openrouter.ai/api/v1"),
            CompatFlavor::Router
        );
        assert_eq!(
            CompatFlavor::from_base_url(
                "https://generativelanguage.googleapis.com/v1beta/openai"
            ),
            CompatFlavor::Google
        );
        assert_eq!(
            CompatFlavor::from_base_url("https://api.groq.com/openai/v1"),
            CompatFlavor::Other
        );
    }

    #[test]
    fn anthropic_gets_effort_and_nothing_else() {
        assert_eq!(ids(&anthropic("claude-sonnet-4-5")), vec!["reasoning_effort"]);
        assert!(ids(&anthropic("claude-2.1")).is_empty());
    }

    #[test]
    fn bedrock_offers_nothing() {
        assert!(ids(&bedrock()).is_empty());
    }

    #[test]
    fn effort_off_is_the_first_choice() {
        let caps = openai_compatible("gpt-5", CompatFlavor::OpenAi);
        let OptionDescriptor::Select { options, .. } = &caps.option_descriptors[0] else {
            panic!("expected a select descriptor");
        };
        assert_eq!(options[0].value, "off");
        assert_eq!(options.last().unwrap().value, "high");
    }
}
