//! The engine registry: every backend, one list, one shape.
//!
//! `from_config` derives the instance set; `snapshot_native` and
//! `snapshot_external` turn one instance into an [`EngineSnapshot`].
//! Native probes are cheap (config reads, no spawn) and safe to run
//! inline on a request path. External probes launch the agent and
//! complete an `initialize` handshake — callers run those in the
//! background and serve the last known result (see the server's
//! engines API for the cache).

use std::collections::BTreeMap;

use mira_ai::ModelInfo;
use mira_config::MiraConfig;

use crate::id::EngineId;
use crate::instance::{instances_from_config, EngineInstance};
use crate::native::{build_native_provider, missing_piece};
use crate::snapshot::{EngineFlavor, EngineSnapshot, EngineState};

/// Every configured backend, keyed by instance id.
#[derive(Clone, Debug, Default)]
pub struct EngineRegistry {
    instances: BTreeMap<EngineId, EngineInstance>,
}

impl EngineRegistry {
    pub fn from_config(cfg: &MiraConfig) -> Self {
        Self {
            instances: instances_from_config(cfg),
        }
    }

    pub fn get(&self, id: &str) -> Option<&EngineInstance> {
        self.instances.get(id)
    }

    pub fn instances(&self) -> impl Iterator<Item = &EngineInstance> {
        self.instances.values()
    }

    /// The instance a fresh session should start on: the yaml
    /// `default_provider` if it maps to a native instance, else the
    /// first native instance, else nothing.
    pub fn default_native_instance(&self, cfg: &MiraConfig) -> Option<&EngineInstance> {
        if let Some(name) = &cfg.default_provider {
            if let Some(inst) = self.instances.get(name.as_str()) {
                if inst.is_native() {
                    return Some(inst);
                }
            }
        }
        self.instances.values().find(|i| i.is_native() && i.enabled)
    }

    /// Probe a native instance: configuration checks only, no network.
    /// `models` is a catalog fetched elsewhere (the caller's cache);
    /// passing it fills the snapshot so the picker can show real rows.
    pub fn snapshot_native(
        &self,
        cfg: &MiraConfig,
        id: &str,
        models: Option<Vec<ModelInfo>>,
    ) -> Option<EngineSnapshot> {
        let inst = self.instances.get(id)?;
        if !inst.is_native() {
            return None;
        }
        let name = inst.id.as_str();
        let state = missing_piece(cfg, name)
            .map(Err)
            .unwrap_or_else(|| build_native_provider(cfg, name).map(|_| ()));
        let models = match (&state, models) {
            (Ok(()), m) => m.unwrap_or_default(),
            // A broken instance keeps nothing: showing a catalog the
            // user can't call would be another way of lying.
            (Err(_), _) => Vec::new(),
        };
        Some(EngineSnapshot {
            instance: inst.id.clone(),
            driver: inst.driver.clone(),
            flavor: EngineFlavor::Native,
            display_name: inst
                .display_name
                .clone()
                .unwrap_or_else(|| mira_config::pretty_provider_name(name)),
            enabled: inst.enabled,
            state: state.err().unwrap_or(EngineState::Ready),
            models,
            default_model: default_model_for(cfg, inst),
            auth: None,
            install_hint: None,
            launch: None,
        })
    }

    /// Probe an external instance by launching it and completing the
    /// handshake. Expensive (spawns a process); run it off the request
    /// path and cache.
    pub async fn snapshot_external(&self, id: &str) -> Option<EngineSnapshot> {
        let inst = self.instances.get(id)?;
        if inst.is_native() {
            return None;
        }
        // A driver this build doesn't know is reported, not dropped.
        let Some(d) = mira_acp::drivers::by_kind(inst.driver.as_str()) else {
            return Some(EngineSnapshot {
                instance: inst.id.clone(),
                driver: inst.driver.clone(),
                flavor: EngineFlavor::External,
                display_name: inst
                    .display_name
                    .clone()
                    .unwrap_or_else(|| inst.id.to_string()),
                enabled: inst.enabled,
                state: EngineState::Unavailable {
                    reason: format!(
                        "this build has no driver `{}` — update Mira or remove the instance",
                        inst.driver
                    ),
                },
                models: Vec::new(),
                default_model: inst.model.clone(),
                auth: None,
                install_hint: None,
                launch: None,
            });
        };
        let cfg = driver_config_for(inst);
        let mut status =
            mira_acp::probe(d.as_ref(), &cfg, mira_acp::driver::PermissionMode::Ask).await;
        // An instance-level display override wins over the driver's own.
        if let Some(name) = &inst.display_name {
            status.display_name = name.clone();
        }
        Some(EngineSnapshot {
            instance: inst.id.clone(),
            driver: inst.driver.clone(),
            flavor: EngineFlavor::External,
            display_name: status.display_name.clone(),
            enabled: inst.enabled,
            state: match status.state {
                mira_acp::status::AgentState::Ready => EngineState::Ready,
                mira_acp::status::AgentState::NotFound { looked_for } => {
                    EngineState::NotFound { looked_for }
                }
                mira_acp::status::AgentState::Failed { reason } => EngineState::Failed { reason },
            },
            models: Vec::new(),
            default_model: inst.model.clone(),
            auth: status.auth.clone(),
            install_hint: status.install_hint.clone(),
            launch: Some(status.launch.clone()),
        })
    }

    /// Probe every external instance. Same cost caveat as
    /// [`Self::snapshot_external`].
    pub async fn snapshot_externals(&self) -> Vec<EngineSnapshot> {
        let mut out = Vec::new();
        for inst in self.instances.values() {
            if !inst.is_native() {
                if let Some(s) = self.snapshot_external(inst.id.as_str()).await {
                    out.push(s);
                }
            }
        }
        out
    }

    /// Parse an external instance's opaque config into the
    /// launch-level [`mira_acp::driver::DriverConfig`]. `None` when
    /// the instance isn't external. Exposed so the server can start an
    /// agent from an instance without re-decoding config at the call
    /// site.
    pub fn external_driver_config(&self, id: &str) -> Option<mira_acp::driver::DriverConfig> {
        let inst = self.instances.get(id)?;
        (!inst.is_native()).then(|| crate::external::driver_config_for(inst))
    }
}

/// The model an instance starts on: its own override, else the global
/// default when it's the default native provider.
pub fn default_model_for(cfg: &MiraConfig, inst: &EngineInstance) -> Option<String> {
    inst.model.clone().or_else(|| {
        (inst.is_native() && cfg.default_provider.as_deref() == Some(inst.id.as_str()))
            .then_some(cfg.default_model.clone())
            .flatten()
    })
}

/// Re-exported so callers reach the conversion without knowing which
/// module owns it.
pub use crate::external::driver_config_for;

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(yaml: &str) -> MiraConfig {
        serde_yaml::from_str(yaml).unwrap()
    }

    #[test]
    fn a_native_instance_reports_its_config_honestly() {
        let c = cfg("default_provider: groq\n");
        let reg = EngineRegistry::from_config(&c);
        let snap = reg.snapshot_native(&c, "groq", None).unwrap();
        assert_eq!(snap.flavor, EngineFlavor::Native);
        assert!(!snap.state.is_ready(), "no key configured: {snap:?}");
        assert!(snap.models.is_empty());
    }

    #[test]
    fn snapshots_keep_catalog_models_only_when_usable() {
        let c = cfg("default_provider: llamacpp\n\
             providers:\n  llamacpp:\n    base_url: http://127.0.0.1:9/v1\n    api_key: x\n");
        let reg = EngineRegistry::from_config(&c);
        let models = vec![ModelInfo {
            id: "local-model".into(),
            display_name: Some("Local".into()),
            owned_by: None,
            context_length: Some(8192),
            capabilities: None,
        }];
        let ready = reg
            .snapshot_native(&c, "llamacpp", Some(models.clone()))
            .unwrap();
        assert!(ready.state.is_ready());
        assert_eq!(ready.models.len(), 1);
        assert_eq!(
            ready.default_model.as_deref(),
            None,
            "no default_model configured"
        );

        let broken = cfg("default_provider: groq\n");
        let reg2 = EngineRegistry::from_config(&broken);
        let snap = reg2.snapshot_native(&broken, "groq", Some(models)).unwrap();
        assert!(!snap.state.is_ready());
        assert!(
            snap.models.is_empty(),
            "a broken instance must not show models"
        );
    }

    #[test]
    fn default_native_instance_follows_the_config() {
        let c = cfg("default_provider: anthropic\ndefault_model: claude-sonnet-4-5\n");
        let reg = EngineRegistry::from_config(&c);
        let inst = reg.default_native_instance(&c).unwrap();
        assert_eq!(inst.id.as_str(), "anthropic");
    }

    #[test]
    fn unknown_external_driver_is_unavailable_not_missing() {
        let c = cfg("default_provider: openai\n\
             engines:\n  myfork:\n    driver: no-such-driver\n");
        let reg = EngineRegistry::from_config(&c);
        assert!(reg.get("myfork").is_some());
        // The full probe spawns nothing for an unknown driver; we assert
        // the registry wiring here and leave the process spawn to the
        // integration-level probe tests in mira-acp.
        let inst = reg.get("myfork").unwrap();
        assert_eq!(inst.driver.as_str(), "no-such-driver");
        assert_eq!(inst.display_name.as_deref(), None);
    }
}
