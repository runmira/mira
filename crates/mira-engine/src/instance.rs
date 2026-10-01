//! Engine instances — what the registry holds.
//!
//! An instance is one selectable backend: a driver kind plus whatever
//! config that driver needs. The registry derives default instances from
//! existing config (one native instance per configured provider, one per
//! known external agent) and then layers user overrides from the
//! `engines:` yaml block on top. Instances the build doesn't recognize
//! are kept as unavailable shells rather than dropped, so config written
//! by a newer build or a fork survives a round trip.

use crate::id::{DriverKind, EngineId};
use mira_config::MiraConfig;
use serde_json::Value;
use std::collections::BTreeMap;

/// The driver slug that means "Mira's own loop over an LLM provider".
pub const NATIVE_DRIVER: &str = "native";

/// One configured backend.
#[derive(Clone, Debug)]
pub struct EngineInstance {
    pub id: EngineId,
    pub driver: DriverKind,
    /// Display override; drivers and provider presets supply their own
    /// when this is `None`.
    pub display_name: Option<String>,
    /// Whether the user opted in. Probing is never gated on this —
    /// health is a fact about the machine — but pickers may filter.
    pub enabled: bool,
    /// Model to start on, when the instance pins one.
    pub model: Option<String>,
    /// Driver-specific configuration, opaque to this crate. For native
    /// instances the driver name under `providers:` is the real config;
    /// this carries external-agent settings (`binary_path`, `env`,
    /// `api_key`, `launch_args`, `home_path`, `effort`, …).
    pub config: Value,
}

impl EngineInstance {
    /// True when this instance runs through Mira's own harness.
    pub fn is_native(&self) -> bool {
        self.driver.as_str() == NATIVE_DRIVER
    }

    /// Parse the opaque config blob as a JSON object, or an empty one.
    pub fn config_object(&self) -> serde_json::Map<String, Value> {
        self.config.as_object().cloned().unwrap_or_default()
    }
}

/// Build the instance set for a config: native instances from the
/// `providers:` block, external instances from the known agent drivers,
/// then user overrides and additions from `engines:`.
pub fn instances_from_config(cfg: &MiraConfig) -> BTreeMap<EngineId, EngineInstance> {
    let mut out: BTreeMap<EngineId, EngineInstance> = BTreeMap::new();

    // One native instance per configured provider. The default provider
    // is always present, even with no explicit `providers:` entry — its
    // base URL comes from the preset table and its key from the
    // conventional env var.
    let mut native_names: Vec<String> = cfg.providers.keys().cloned().collect();
    if let Some(default) = &cfg.default_provider {
        if !native_names.iter().any(|n| n == default) {
            native_names.push(default.clone());
        }
    }
    for name in native_names {
        if let Some(id) = EngineId::new(&name) {
            out.insert(
                id.clone(),
                EngineInstance {
                    id,
                    driver: DriverKind::new(NATIVE_DRIVER).expect("constant is a valid slug"),
                    display_name: Some(mira_config::pretty_provider_name(&name)),
                    enabled: true,
                    model: default_model_for_native(cfg, &name),
                    config: Value::Null,
                },
            );
        }
    }

    // One external instance per agent driver this build knows. Unknown
    // drivers cannot be instantiated, so they only appear when the user
    // declares them below.
    for d in mira_acp::drivers::all() {
        if let Some(id) = EngineId::new(d.kind()) {
            out.entry(id.clone()).or_insert_with(|| EngineInstance {
                id,
                driver: DriverKind::new(d.kind()).expect("driver kinds are valid slugs"),
                display_name: Some(d.display_name().to_string()),
                enabled: true,
                model: None,
                config: Value::Null,
            });
        }
    }

    // User overrides + additions. `driver:` is required for a slug that
    // isn't already known; overrides for known instances keep theirs.
    for (slug, over) in &cfg.engines {
        let Some(id) = EngineId::new(slug) else {
            continue;
        };
        let entry = out.entry(id.clone()).or_insert_with(|| EngineInstance {
            id,
            // Unknown driver slugs are legal: the snapshot layer will
            // report the instance as unavailable rather than dropping it.
            driver: DriverKind::new(
                over.driver.clone().unwrap_or_else(|| "unknown".to_string()),
            )
            .unwrap_or_else(|| DriverKind::new("unknown").expect("constant is a valid slug")),
            display_name: None,
            enabled: true,
            model: None,
            config: Value::Null,
        });
        if let Some(driver) = &over.driver {
            if let Some(d) = DriverKind::new(driver) {
                entry.driver = d;
            }
        }
        if over.display_name.is_some() {
            entry.display_name = over.display_name.clone();
        }
        if let Some(enabled) = over.enabled {
            entry.enabled = enabled;
        }
        if over.model.is_some() {
            entry.model = over.model.clone();
        }
        match &over.config {
            Some(v) if !v.is_null() => entry.config = v.clone(),
            _ => {}
        }
    }

    out
}

/// The model a native instance starts on: the global default when this
/// instance is the default provider, else the instance's own override.
fn default_model_for_native(cfg: &MiraConfig, provider: &str) -> Option<String> {
    if cfg.default_provider.as_deref() == Some(provider) {
        cfg.default_model.clone().or_else(|| cfg.engines.get(provider).and_then(|e| e.model.clone()))
    } else {
        cfg.engines.get(provider).and_then(|e| e.model.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(yaml: &str) -> MiraConfig {
        serde_yaml::from_str(yaml).unwrap()
    }

    #[test]
    fn default_provider_yields_a_native_instance_even_without_an_entry() {
        let c = cfg("default_provider: anthropic\ndefault_model: claude-sonnet-4-5\n");
        let instances = instances_from_config(&c);
        let a = instances.get(&EngineId::new("anthropic").unwrap()).unwrap();
        assert!(a.is_native());
        assert_eq!(a.model.as_deref(), Some("claude-sonnet-4-5"));
        assert_eq!(a.display_name.as_deref(), Some("Anthropic"));
    }

    #[test]
    fn every_known_agent_driver_appears() {
        let c = cfg("default_provider: openai\n");
        let instances = instances_from_config(&c);
        for kind in ["claude-code", "codex", "opencode", "grok", "cursor", "antigravity"] {
            assert!(
                instances.contains_key(&EngineId::new(kind).unwrap()),
                "missing external instance {kind}"
            );
        }
    }

    #[test]
    fn overrides_merge_and_unknown_drivers_survive() {
        let c = cfg(
            "\
default_provider: openai
engines:
  codex:
    enabled: false
  codex-work:
    driver: codex
    display_name: Codex (work)
    config:
      env:
        CODEX_PROFILE: work
  future-fork:
    driver: myfork
",
        );
        let instances = instances_from_config(&c);
        assert!(!instances[&EngineId::new("codex").unwrap()].enabled);
        let work = &instances[&EngineId::new("codex-work").unwrap()];
        assert_eq!(work.driver.as_str(), "codex");
        assert_eq!(work.display_name.as_deref(), Some("Codex (work)"));
        assert_eq!(
            work.config_object()["env"]["CODEX_PROFILE"],
            "work",
            "the opaque config blob must be carried verbatim"
        );
        // An unknown driver stays in the map so the UI can show it as
        // unavailable instead of silently eating the config.
        assert_eq!(instances[&EngineId::new("future-fork").unwrap()].driver.as_str(), "myfork");
    }

    #[test]
    fn invalid_slugs_are_skipped_not_fatal() {
        let c = cfg(
            "\
default_provider: openai
providers:
  \"not a slug\":
    base_url: http://localhost:1
engines:
  \"\":
    driver: codex
",
        );
        // Must not panic; the valid instances still appear. The
        // non-slug provider and the empty slug are skipped.
        let instances = instances_from_config(&c);
        assert!(instances.contains_key(&EngineId::new("openai").unwrap()));
        assert!(instances.keys().all(|k| crate::id::is_valid_slug(k.as_str())));
    }
}
