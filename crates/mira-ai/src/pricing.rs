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
/// resolves to the `gpt-4o` entry,
/// and so `openai/gpt-4o` (a gateway id)
/// resolves the same way.
pub fn price_for(model: &str) -> Option<ModelPrice> {
    // Prefer the dynamic table (LiteLLM) when one is installed — it is fresher
    // than the embedded snapshot. Fall back to the builtin table on a miss so
    // offline-only setups keep working with exactly one pricing crate.
    if let Ok(g) = dynamic_lock().read() {
        if let Some(t) = g.as_ref() {
            if let Some(p) = dynamic_lookup(&t.rows, model) {
                return Some(p);
            }
        }
    }
    // Gateways like OpenRouter prefix the vendor (`anthropic/claude-…`);
    // price on the model part. Handle nested gateway IDs like 'provider/model/submodel'
    // by taking the last segment after the final slash.
    let m = model.to_ascii_lowercase();
    let m = m.rsplit('/').next().unwrap_or(&m);
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

// ---------------------------------------------------------------------------
// Dynamic pricing: fetched from LiteLLM with an on-disk TTL cache.
//
// The hardcoded `MODEL_PRICES` above stays as the offline default; everything
// else (new models, renames, new cache ratios) comes from the upstream JSON.
// Cost math is still the same formula — only the table is dynamic — so the
// UI keeps rendering `$0.024` without a round-trip once the fetch lands.
// ---------------------------------------------------------------------------

/// Where the active price table came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PricingSource {
    Builtin,
    Cached,
    Fetched,
}

/// One priced model row, ready for prefix-match lookups.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PricingRow {
    /// The model key exactly as published upstream (already lower-cased).
    pub key: String,
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
    pub cached_input_per_mtok: f64,
}

/// What the client is handed for quoting.
#[derive(Clone, Debug, serde::Serialize)]
pub struct PricingSnapshot {
    pub rows: Vec<PricingRow>,
    pub source: PricingSource,
    /// Unix seconds when we last fetched or deserialised the cache; `None`
    /// for the built-in fallback.
    pub fetched_at: Option<u64>,
}

static DYNAMIC: std::sync::OnceLock<std::sync::RwLock<Option<DynamicTable>>> =
    std::sync::OnceLock::new();

struct DynamicTable {
    rows: Vec<PricingRow>,
    fetched_at: Option<u64>,
    source: PricingSource,
}

const LITELLM_PRICES_URL: &str =
    "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";
const PRICING_TTL_SECS: u64 = 24 * 60 * 60;

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn dynamic_lock() -> &'static std::sync::RwLock<Option<DynamicTable>> {
    DYNAMIC.get_or_init(|| std::sync::RwLock::new(None))
}

/// The LiteLLM caches as a map of model-id → price fields, all costs in
/// USD per *token*; multiply by 1e6 for the units we serve.
fn parse_litellm_rows(body: &serde_json::Value) -> Vec<PricingRow> {
    let mut rows = Vec::new();
    let Some(obj) = body.as_object() else {
        return rows;
    };
    for (name, info) in obj {
        let pick = |k: &str| {
            info.get(k)
                .and_then(|v| v.as_f64())
                .map(|v| v * 1_000_000.0)
        };
        let (Some(input), Some(output)) =
            (pick("input_cost_per_token"), pick("output_cost_per_token"))
        else {
            continue;
        };
        let cached = pick("cache_read_input_token_cost").unwrap_or(input);
        rows.push(PricingRow {
            key: name.to_ascii_lowercase(),
            input_per_mtok: input,
            output_per_mtok: output,
            cached_input_per_mtok: cached,
        });
    }
    rows
}

/// Cache file payload: parsed rows, not the raw dump, so a partial parse
/// never sticks around and bootstrapping without the network stays cheap.
#[derive(serde::Serialize, serde::Deserialize)]
struct PricingCache {
    fetched_at: u64,
    rows: Vec<PricingRow>,
}

/// Look up dynamic table entries matching `model`. Gateway prefixes
/// (`openai/gpt-4o`) price on the last segment, and a longer prefix wins,
/// matching `price_for`'s builtin behaviour.
fn dynamic_lookup(rows: &[PricingRow], model: &str) -> Option<ModelPrice> {
    let lower = model.to_ascii_lowercase();
    let m = lower.rsplit('/').next().unwrap_or(&lower);
    rows.iter()
        .filter(|r| {
            let r_model = r.key.rsplit('/').next().unwrap_or(&r.key);
            m.starts_with(r_model)
        })
        .max_by_key(|r| r.key.len())
        .map(|r| ModelPrice {
            input_per_mtok: r.input_per_mtok,
            output_per_mtok: r.output_per_mtok,
            cached_input_per_mtok: r.cached_input_per_mtok,
        })
}

impl DynamicTable {
    fn from_cache(cache: PricingCache) -> Self {
        DynamicTable {
            rows: cache.rows,
            fetched_at: Some(cache.fetched_at),
            source: PricingSource::Cached,
        }
    }
}

/// Snapshot what the client should render right now: the dynamic table
/// when available, else the embedded one marked `Source::Builtin`.
pub fn pricing_snapshot() -> PricingSnapshot {
    match dynamic_lock()
        .read()
        .ok()
        .and_then(|g| g.as_ref().map(|t| (t.rows.clone(), t.source, t.fetched_at)))
    {
        Some((rows, source, fetched_at)) => PricingSnapshot {
            rows: rows
                .into_iter()
                .map(|r| PricingRow {
                    key: r.key,
                    input_per_mtok: r.input_per_mtok,
                    output_per_mtok: r.output_per_mtok,
                    cached_input_per_mtok: r.cached_input_per_mtok,
                })
                .collect(),
            source,
            fetched_at,
        },
        None => PricingSnapshot {
            rows: MODEL_PRICES
                .iter()
                .map(|(prefix, p)| PricingRow {
                    key: (*prefix).to_string(),
                    input_per_mtok: p.input_per_mtok,
                    output_per_mtok: p.output_per_mtok,
                    cached_input_per_mtok: p.cached_input_per_mtok,
                })
                .collect(),
            source: PricingSource::Builtin,
            fetched_at: None,
        },
    }
}

/// Ensure a fresh-enough dynamic table is installed. Falls back to an
/// untouched `DynamicTable::None` — the caller keeps using the builtin
/// table — when the network and the disk cache are both unavailable.
pub async fn ensure_pricing() {
    // Fast path: a cached snapshot younger than the TTL.
    if let Ok(g) = dynamic_lock().read() {
        if let Some(t) = g.as_ref() {
            if let Some(ft) = t.fetched_at {
                if now_secs().saturating_sub(ft) < PRICING_TTL_SECS {
                    return;
                }
            }
        }
    }

    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let cache_path = home.as_ref().map(|h| h.join(".mira/model_prices.json"));

    // Try the on-disk cache first; a network miss shouldn't disturb a
    // session running the right numbers offline.
    if let Some(path) = &cache_path {
        if let Ok(text) = std::fs::read_to_string(path) {
            if let Ok(cache) = serde_json::from_str::<PricingCache>(&text) {
                let age = now_secs().saturating_sub(cache.fetched_at);
                if age < PRICING_TTL_SECS {
                    if let Ok(mut g) = dynamic_lock().write() {
                        *g = Some(DynamicTable::from_cache(cache));
                    }
                    return;
                }
            }
        }
    }

    let fetched = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(client) => match client.get(LITELLM_PRICES_URL).send().await {
            Ok(resp) => resp.json::<serde_json::Value>().await.ok(),
            Err(_) => None,
        },
        Err(_) => None,
    };

    let (rows, source, fetched_at) = match fetched {
        Some(body) => {
            let rows = parse_litellm_rows(&body);
            let now = now_secs();
            if !rows.is_empty() {
                if let Some(path) = &cache_path {
                    let payload = serde_json::to_string(&PricingCache {
                        fetched_at: now,
                        rows: rows.clone(),
                    })
                    .ok();
                    if let Some(p) = payload {
                        let _ = std::fs::create_dir_all(path.parent().unwrap());
                        let _ = std::fs::write(path, p);
                    }
                }
                (rows, PricingSource::Fetched, Some(now))
            } else {
                // Parseable but unrecognised shape — better to keep the
                // builtins than to install a table that prices nothing.
                (Vec::new(), PricingSource::Builtin, None)
            }
        }
        None => {
            // No network: keep any cached table we already loaded; first
            // run leaves the builtin in charge.
            if let Ok(g) = dynamic_lock().read() {
                if g.is_some() {
                    return;
                }
            }
            (Vec::new(), PricingSource::Builtin, None)
        }
    };

    if rows.is_empty() {
        return;
    }
    if let Ok(mut g) = dynamic_lock().write() {
        *g = Some(DynamicTable {
            rows,
            fetched_at,
            source,
        });
    }
}

/// Swap the dynamic table in-process — used once at startup and by tests
/// to exercise the dynamic path without touching the network.
#[doc(hidden)]
pub fn set_dynamic_table_for_test(table: Option<(Vec<PricingRow>, PricingSource, u64)>) {
    if let Ok(mut g) = dynamic_lock().write() {
        *g = table.map(|(rows, source, fetched_at)| DynamicTable {
            rows,
            fetched_at: Some(fetched_at),
            source,
        });
    }
}

/// Match order matters: put more-specific prefixes first (e.g. `gpt-4o-mini`
/// before `gpt-4o`).
const MODEL_PRICES: &[(&str, ModelPrice)] = &[
    // -- OpenAI --
    (
        "gpt-5-mini",
        ModelPrice {
            input_per_mtok: 0.25,
            output_per_mtok: 2.00,
            cached_input_per_mtok: 0.025,
        },
    ),
    (
        "gpt-5",
        ModelPrice {
            input_per_mtok: 1.25,
            output_per_mtok: 10.00,
            cached_input_per_mtok: 0.125,
        },
    ),
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
        "claude-opus-4-5",
        ModelPrice {
            input_per_mtok: 5.00,
            output_per_mtok: 25.00,
            cached_input_per_mtok: 0.50,
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

    #[test]
    fn gateway_prefixed_ids_are_priced() {
        assert!(price_for("anthropic/claude-sonnet-4.5").is_some());
        assert_eq!(
            price_for("openai/gpt-4o").map(|p| p.input_per_mtok),
            price_for("gpt-4o").map(|p| p.input_per_mtok)
        );
    }

    #[test]
    fn litellm_json_is_parsed_into_prefix_matchable_rows() {
        let body = serde_json::json!({
            "gpt-4o": { "input_cost_per_token": 2.5e-6, "output_cost_per_token": 10e-6 },
            "cached-user-model": { "input_cost_per_token": 1.0e-6, "output_cost_per_token": 3e-6, "cache_read_input_token_cost": 0.1e-6 },
            "no-pricing-fields": {},
        });
        let rows = parse_litellm_rows(&body);
        assert_eq!(rows.len(), 2);
        let gpt = rows.iter().find(|r| r.key == "gpt-4o").unwrap();
        assert_eq!(gpt.input_per_mtok, 2.5);
        assert_eq!(gpt.output_per_mtok, 10.0);
        // Cache miss falls back to fresh input, not a made-up number.
        assert_eq!(gpt.cached_input_per_mtok, 2.5);
        assert!(rows.iter().all(|r| r.key != "no-pricing-fields"));
    }

    #[test]
    fn dynamic_table_wins_then_prefix_falls_back() {
        // Keyed on an invented prefix no other test resolves, so the global
        // slot cannot leak into sibling tests running in parallel.
        set_dynamic_table_for_test(Some((
            vec![PricingRow {
                key: "zz-pullet".into(),
                input_per_mtok: 11.0,
                output_per_mtok: 22.0,
                cached_input_per_mtok: 5.5,
            }],
            PricingSource::Fetched,
            1_500_000_000,
        )));
        let p = price_for("zz-pullet-2026").unwrap();
        assert_eq!(
            p.input_per_mtok, 11.0,
            "dynamic base hit becomes the prefix"
        );
        let p = price_for("openai/zz-pullet").unwrap();
        assert_eq!(p.input_per_mtok, 11.0, "gateway prefix still matches");
        set_dynamic_table_for_test(None);
    }
}
