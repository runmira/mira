//! Native instances — Mira's own loop over an LLM provider.
//!
//! The provider-construction recipe used to live in `mira-server`'s
//! settings handler, which meant the CLI and anything else that wanted
//! a provider for a named provider config had to copy it. It lives here
//! now: given a [`MiraConfig`] and a provider name, produce either a
//! working [`ChatProvider`] or the precise reason it can't.
//!
//! There is no `NullProvider` fallback here. Callers that want
//! fail-soft behavior get an [`EngineState::NotConfigured`] and decide
//! themselves — the server keeps running with a null provider, the TUI
//! prints the reason and exits. One recipe, two policies.

use std::collections::BTreeMap;
use std::sync::Arc;

use mira_ai::{build_chat_provider, ChatProvider};
use mira_config::MiraConfig;

use crate::snapshot::EngineState;

/// Why a native provider can't be built, phrased for the user. Pure
/// config inspection: no network, no build.
pub fn missing_piece(cfg: &MiraConfig, name: &str) -> Option<EngineState> {
    let entry = cfg.providers.get(name).cloned().unwrap_or_default();
    if entry.base_url.is_none() && mira_config::default_base_url_for(name).is_none() {
        return Some(EngineState::NotConfigured {
            reason: format!("provider `{name}` has no base_url"),
        });
    }
    // Bedrock can sign with AWS credentials instead of an API key.
    if entry.resolved_api_key().is_none() && name != "bedrock" {
        let hint = mira_config::default_api_key_env_for(name)
            .map(|env| format!(" (set {env} or add it in Settings)"))
            .unwrap_or_default();
        return Some(EngineState::NotConfigured {
            reason: format!("provider `{name}` has no api_key{hint}"),
        });
    }
    None
}

/// Build the provider for native instance `name`, or the reason it
/// can't be built. Cheap: no network, no spawn — `list_models` is the
/// caller's choice.
pub fn build_native_provider(
    cfg: &MiraConfig,
    name: &str,
) -> Result<Arc<dyn ChatProvider>, EngineState> {
    if let Some(state) = missing_piece(cfg, name) {
        return Err(state);
    }
    let entry = cfg.providers.get(name).cloned().unwrap_or_default();
    let base_url = entry
        .base_url
        .clone()
        .or_else(|| mira_config::default_base_url_for(name).map(str::to_owned))
        .expect("missing_piece checked the base URL");
    let api_key = entry.resolved_api_key().unwrap_or_default(); // empty = Bedrock's AWS-credentials path
    let extra_headers: Vec<(String, String)> = entry
        .extra_headers
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let prompt_caching = mira_config::prompt_caching_enabled(name, &base_url, entry.prompt_caching);
    build_chat_provider(name, base_url, api_key, extra_headers, prompt_caching).map_err(|e| {
        EngineState::NotConfigured {
            reason: format!("provider `{name}` failed to build: {e}"),
        }
    })
}

/// Providers for every native instance, keyed by instance id. The pool
/// is what makes "switch engines mid-session" real: selection swaps a
/// pointer instead of rebuilding the world.
pub fn build_native_pool(cfg: &MiraConfig) -> BTreeMap<String, Arc<dyn ChatProvider>> {
    let mut pool = BTreeMap::new();
    for name in cfg.providers.keys() {
        match build_native_provider(cfg, name) {
            Ok(p) => {
                pool.insert(name.clone(), p);
            }
            Err(state) => {
                tracing::debug!(provider = %name, state = ?state, "provider not built at boot");
            }
        }
    }
    // The default provider is always in the pool if it can build, even
    // with no explicit `providers:` entry (preset base URL + env key).
    if let Some(default) = &cfg.default_provider {
        if !pool.contains_key(default) {
            if let Ok(p) = build_native_provider(cfg, default) {
                pool.insert(default.clone(), p);
            }
        }
    }
    pool
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(yaml: &str) -> MiraConfig {
        serde_yaml::from_str(yaml).unwrap()
    }

    #[test]
    fn a_missing_key_names_the_env_var() {
        let c = cfg("default_provider: groq\n");
        let state = missing_piece(&c, "groq");
        let EngineState::NotConfigured { reason } = state.unwrap() else {
            panic!("expected NotConfigured");
        };
        assert!(reason.contains("GROQ_API_KEY"), "{reason}");
    }

    #[test]
    fn bedrock_needs_no_api_key_in_config() {
        // Config-level: Bedrock may sign with AWS credentials, so a
        // missing `api_key` is not a configuration error. (Whether the
        // machine actually has AWS credentials is `Bedrock::new`'s
        // concern — that depends on the environment, not the config.)
        let c = cfg("default_provider: bedrock\n");
        assert!(missing_piece(&c, "bedrock").is_none());
    }

    #[test]
    fn local_endpoints_can_use_a_literal_key() {
        let c = cfg(
            "providers:\n  llamacpp:\n    base_url: http://127.0.0.1:8080/v1\n    api_key: local\n",
        );
        assert!(missing_piece(&c, "llamacpp").is_none());
        assert!(build_native_provider(&c, "llamacpp").is_ok());
    }
}
