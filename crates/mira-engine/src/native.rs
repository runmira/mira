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

/// A native instance's provider settings with every layer applied.
///
/// A native instance is either a `providers:` entry (`anthropic`), or an
/// `engines:` entry with `driver: native` whose `config` overlays one:
///
/// ```yaml
/// engines:
///   anthropic-work:
///     driver: native
///     config: { provider: anthropic, api_key_env: ANTHROPIC_WORK_KEY }
/// ```
///
/// `provider` names the preset (base URL, key env var, caching, display)
/// and the `providers:` entry to inherit from; it defaults to the
/// instance's own name. The instance's `api_key`, `api_key_env`,
/// `base_url`, `extra_headers` and `prompt_caching` win over it. That is
/// what lets two accounts of one provider run side by side.
#[derive(Clone, Debug)]
pub struct NativeSettings {
    /// The preset the instance is an account of (`anthropic`).
    pub preset: String,
    pub entry: mira_config::ProviderConfig,
    /// Whether an `engines:` entry contributed, so errors can say which
    /// layer to fix.
    pub from_engine: bool,
}

pub fn native_settings(cfg: &MiraConfig, name: &str) -> NativeSettings {
    let overlay = cfg
        .engines
        .get(name)
        .and_then(|e| e.config.as_ref())
        .and_then(|v| v.as_object())
        .cloned();
    let Some(o) = overlay else {
        return NativeSettings {
            preset: name.to_string(),
            entry: cfg.providers.get(name).cloned().unwrap_or_default(),
            from_engine: false,
        };
    };
    let text = |k: &str| {
        o.get(k)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
    };
    let preset = text("provider").unwrap_or_else(|| name.to_string());
    // The instance's own `providers:` entry if it has one, else the preset's.
    let mut entry = cfg
        .providers
        .get(name)
        .or_else(|| cfg.providers.get(&preset))
        .cloned()
        .unwrap_or_default();
    if let Some(k) = text("api_key") {
        entry.api_key = Some(k);
        entry.api_key_env = None;
    } else if let Some(env) = text("api_key_env") {
        // A named env var replaces an inherited literal key: it's this
        // account's key, not the preset's.
        entry.api_key = None;
        entry.api_key_env = Some(env);
    }
    if let Some(url) = text("base_url") {
        entry.base_url = Some(url);
    }
    if let Some(h) = o.get("extra_headers").and_then(|v| v.as_object()) {
        for (k, v) in h {
            if let Some(v) = v.as_str() {
                entry.extra_headers.insert(k.clone(), v.to_string());
            }
        }
    }
    if let Some(b) = o.get("prompt_caching").and_then(|v| v.as_bool()) {
        entry.prompt_caching = Some(b);
    }
    NativeSettings {
        preset,
        entry,
        from_engine: true,
    }
}

/// Why a native provider can't be built, phrased for the user. Pure
/// config inspection: no network, no build.
pub fn missing_piece(cfg: &MiraConfig, name: &str) -> Option<EngineState> {
    let NativeSettings {
        preset,
        entry,
        from_engine,
    } = native_settings(cfg, name);
    // Say which layer to fix: the engine entry, or the provider entry.
    let layer = if from_engine {
        format!("engine `{name}` (engines.{name}.config)")
    } else {
        format!("provider `{name}`")
    };
    if entry.base_url.is_none() && mira_config::default_base_url_for(&preset).is_none() {
        return Some(EngineState::NotConfigured {
            reason: format!("{layer} has no base_url"),
        });
    }
    // Bedrock can sign with AWS credentials instead of an API key.
    if entry.resolved_api_key().is_none() && preset != "bedrock" {
        let hint = if from_engine {
            " (set api_key_env or api_key there)".to_string()
        } else {
            mira_config::default_api_key_env_for(&preset)
                .map(|env| format!(" (set {env} or add it in Settings)"))
                .unwrap_or_default()
        };
        return Some(EngineState::NotConfigured {
            reason: format!("{layer} has no api_key{hint}"),
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
    let NativeSettings { preset, entry, .. } = native_settings(cfg, name);
    let base_url = entry
        .base_url
        .clone()
        .or_else(|| mira_config::default_base_url_for(&preset).map(str::to_owned))
        .expect("missing_piece checked the base URL");
    let api_key = entry.resolved_api_key().unwrap_or_default(); // empty = Bedrock's AWS-credentials path
    let extra_headers: Vec<(String, String)> = entry
        .extra_headers
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let prompt_caching =
        mira_config::prompt_caching_enabled(&preset, &base_url, entry.prompt_caching);
    build_chat_provider(&preset, base_url, api_key, extra_headers, prompt_caching).map_err(|e| {
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
    // Every native instance: `providers:` entries and `driver: native`
    // engines (a second account of a provider).
    let names: Vec<String> = crate::instance::instances_from_config(cfg)
        .into_values()
        .filter(|i| i.is_native())
        .map(|i| i.id.to_string())
        .collect();
    for name in &names {
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
    fn a_second_account_of_a_provider_uses_its_own_key() {
        let c = cfg(
            "default_provider: anthropic\n\
             providers:\n  anthropic: { api_key: personal-key }\n\
             engines:\n  anthropic-work:\n    driver: native\n    config: { provider: anthropic, api_key: work-key }\n",
        );
        let work = native_settings(&c, "anthropic-work");
        assert_eq!(work.preset, "anthropic");
        assert_eq!(work.entry.resolved_api_key().as_deref(), Some("work-key"));
        assert!(work.from_engine);
        let personal = native_settings(&c, "anthropic");
        assert_eq!(
            personal.entry.resolved_api_key().as_deref(),
            Some("personal-key")
        );
        // Both build, and both are in the pool.
        assert!(build_native_provider(&c, "anthropic-work").is_ok());
        let pool = build_native_pool(&c);
        assert!(pool.contains_key("anthropic") && pool.contains_key("anthropic-work"));
    }

    #[test]
    fn an_instance_key_env_replaces_the_inherited_literal() {
        let c = cfg(
            "providers:\n  openai: { api_key: shared }\n\
             engines:\n  openai-team:\n    driver: native\n    config: { provider: openai, api_key_env: MIRA_TEST_UNSET_TEAM_KEY_XYZ, base_url: https://gw.example/v1 }\n",
        );
        let s = native_settings(&c, "openai-team");
        assert_eq!(s.entry.api_key, None);
        assert_eq!(s.entry.base_url.as_deref(), Some("https://gw.example/v1"));
        if std::env::var("MIRA_API_KEY").is_err() {
            let reason = match missing_piece(&c, "openai-team") {
                Some(EngineState::NotConfigured { reason }) => reason,
                other => panic!("{other:?}"),
            };
            assert!(reason.contains("engines.openai-team.config"), "{reason}");
        }
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
