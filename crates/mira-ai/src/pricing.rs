//! Per-model pricing table for approximating session cost from token counts.
//!
//! Prices are USD per million tokens. Numbers are rough public list prices
//! sampled early-2026 — not authoritative; a self-hosted deploy pays $0 for
//! the same model and OpenRouter often lists a surcharge. Users who need
//! exact accounting should treat cost as advisory and rely on their
//! provider's dashboard.
//!
//! Unknown models return `None` and the UI falls back to showing tokens
//! only. Contributions welcome — grep for `MODEL_PRICES` and add a row.

use crate::event::TokenUsage;

/// Dollars-per-million pricing for one model.
#[derive(Copy, Clone, Debug)]
pub struct ModelPrice {
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
    /// Cached prompt tokens are usually discounted (OpenAI charges 0.5x,
    /// Anthropic 0.1x). We charge them at this rate instead of `input`.
    pub cached_input_per_mtok: f64,
}

/// Look up pricing by model id. Matches on prefix so `gpt-4o-2024-11-20`
/// resolves to the `gpt-4o` entry.
pub fn price_for(model: &str) -> Option<ModelPrice> {
    let m = model.to_ascii_lowercase();
    MODEL_PRICES
        .iter()
        .find(|(prefix, _)| m.starts_with(prefix))
        .map(|(_, price)| *price)
}

/// Compute USD cost for a usage record, if the model is priced.
pub fn cost_usd(model: &str, usage: TokenUsage) -> Option<f64> {
    let p = price_for(model)?;
    let uncached_input = usage
        .prompt_tokens
        .saturating_sub(usage.cached_input_tokens);
    let dollars = (uncached_input as f64) * p.input_per_mtok / 1_000_000.0
        + (usage.cached_input_tokens as f64) * p.cached_input_per_mtok / 1_000_000.0
        + (usage.completion_tokens as f64) * p.output_per_mtok / 1_000_000.0;
    Some(dollars)
}

/// Match order matters: put more-specific prefixes first (e.g. `gpt-4o-mini`
/// before `gpt-4o`).
const MODEL_PRICES: &[(&str, ModelPrice)] = &[
    // -- OpenAI --
    (
        "gpt-4o-mini",
        ModelPrice {
            input_per_mtok: 0.15,
            output_per_mtok: 0.60,
            cached_input_per_mtok: 0.075,
        },
    ),
    (
        "gpt-4o",
        ModelPrice {
            input_per_mtok: 2.50,
            output_per_mtok: 10.00,
            cached_input_per_mtok: 1.25,
        },
    ),
    (
        "gpt-4.1-mini",
        ModelPrice {
            input_per_mtok: 0.40,
            output_per_mtok: 1.60,
            cached_input_per_mtok: 0.10,
        },
    ),
    (
        "gpt-4.1",
        ModelPrice {
            input_per_mtok: 2.00,
            output_per_mtok: 8.00,
            cached_input_per_mtok: 0.50,
        },
    ),
    (
        "o1-mini",
        ModelPrice {
            input_per_mtok: 3.00,
            output_per_mtok: 12.00,
            cached_input_per_mtok: 1.50,
        },
    ),
    (
        "o1",
        ModelPrice {
            input_per_mtok: 15.00,
            output_per_mtok: 60.00,
            cached_input_per_mtok: 7.50,
        },
    ),
    (
        "o3-mini",
        ModelPrice {
            input_per_mtok: 1.10,
            output_per_mtok: 4.40,
            cached_input_per_mtok: 0.55,
        },
    ),
    (
        "o3",
        ModelPrice {
            input_per_mtok: 2.00,
            output_per_mtok: 8.00,
            cached_input_per_mtok: 0.50,
        },
    ),
    // -- Anthropic (via native or compat endpoint) --
    (
        "claude-haiku-4",
        ModelPrice {
            input_per_mtok: 1.00,
            output_per_mtok: 5.00,
            cached_input_per_mtok: 0.10,
        },
    ),
    (
        "claude-sonnet-4",
        ModelPrice {
            input_per_mtok: 3.00,
            output_per_mtok: 15.00,
            cached_input_per_mtok: 0.30,
        },
    ),
    (
        "claude-opus-4",
        ModelPrice {
            input_per_mtok: 15.00,
            output_per_mtok: 75.00,
            cached_input_per_mtok: 1.50,
        },
    ),
    // -- Groq / Together / open-weight (very rough; often free tier) --
    (
        "llama-3.3-70b",
        ModelPrice {
            input_per_mtok: 0.59,
            output_per_mtok: 0.79,
            cached_input_per_mtok: 0.59,
        },
    ),
    (
        "llama-3.1-70b",
        ModelPrice {
            input_per_mtok: 0.59,
            output_per_mtok: 0.79,
            cached_input_per_mtok: 0.59,
        },
    ),
    (
        "llama-3.1-8b",
        ModelPrice {
            input_per_mtok: 0.05,
            output_per_mtok: 0.08,
            cached_input_per_mtok: 0.05,
        },
    ),
    (
        "qwen-2.5-72b",
        ModelPrice {
            input_per_mtok: 0.90,
            output_per_mtok: 0.90,
            cached_input_per_mtok: 0.90,
        },
    ),
    (
        "deepseek-v3",
        ModelPrice {
            input_per_mtok: 0.27,
            output_per_mtok: 1.10,
            cached_input_per_mtok: 0.07,
        },
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_match_picks_more_specific_first() {
        let mini = price_for("gpt-4o-mini-2024-07-18").unwrap();
        let full = price_for("gpt-4o-2024-11-20").unwrap();
        assert!(mini.input_per_mtok < full.input_per_mtok);
    }

    #[test]
    fn unknown_model_yields_none() {
        assert!(price_for("some-unknown-model").is_none());
        assert!(cost_usd(
            "some-unknown-model",
            TokenUsage {
                prompt_tokens: 1000,
                completion_tokens: 500,
                cached_input_tokens: 0
            }
        )
        .is_none());
    }

    #[test]
    fn cached_tokens_are_discounted() {
        let usage_no_cache = TokenUsage {
            prompt_tokens: 1_000_000,
            completion_tokens: 0,
            cached_input_tokens: 0,
        };
        let usage_all_cache = TokenUsage {
            prompt_tokens: 1_000_000,
            completion_tokens: 0,
            cached_input_tokens: 1_000_000,
        };
        let a = cost_usd("gpt-4o", usage_no_cache).unwrap();
        let b = cost_usd("gpt-4o", usage_all_cache).unwrap();
        assert!(b < a, "cached tokens should be cheaper");
    }
}
