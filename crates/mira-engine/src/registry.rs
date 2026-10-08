//! The engine registry: every backend, one list, one shape.
//!
//! `from_config` derives the instance set; `snapshot_native` and
//! `snapshot_external` turn one instance into an [`EngineSnapshot`].
//! Native probes are cheap (config reads, no spawn) and safe to run
//! inline on a request path. External probes come in two depths: the
//! background sweep learns what it can without opening an authenticated
//! catalog session, while an explicit refresh runs the full probe.
//! Callers run those in the background and serve the last known result
//! (see the server's engines API for the cache).

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

/// A native instance's prompt-caching mode, for the engines API (#89).
fn caching_mode(cfg: &MiraConfig, name: &str) -> String {
    let settings = crate::native::native_settings(cfg, name);
    let base_url = settings
        .entry
        .base_url
        .clone()
        .or_else(|| mira_config::default_base_url_for(&settings.preset).map(str::to_owned))
        .unwrap_or_default();
    if mira_config::prompt_caching_enabled(
        &settings.preset,
        &base_url,
        settings.entry.prompt_caching,
    ) {
        "markers".into()
    } else if mira_config::caching_capability(&settings.preset, &base_url)
        == mira_config::CachingCapability::Automatic
    {
        "automatic".into()
    } else {
        "off".into()
    }
}

/// How far an external probe may go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeDepth {
    /// Background sweep: detection without an authenticated catalog
    /// session (no app-server handshake, no `account/read`).
    Background,
    /// Explicit refresh or chat start: the full probe including catalog.
    Full,
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
            credential_note: None,
            prompt_caching: Some(caching_mode(cfg, name)),
            agent: None,
        })
    }

    /// Probe an external instance by launching it and completing the
    /// handshake. Expensive (spawns a process); run it off the request
    /// path and cache.
    pub async fn snapshot_external(&self, id: &str) -> Option<EngineSnapshot> {
        self.snapshot_external_with_depth(id, ProbeDepth::Full)
            .await
    }

    /// The background-sweep variant: no authenticated catalog session.
    /// See [`mira_acp::status::probe_background`].
    pub async fn snapshot_external_background(&self, id: &str) -> Option<EngineSnapshot> {
        self.snapshot_external_with_depth(id, ProbeDepth::Background)
            .await
    }

    pub async fn snapshot_external_with_depth(
        &self,
        id: &str,
        depth: ProbeDepth,
    ) -> Option<EngineSnapshot> {
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
                credential_note: None,
                prompt_caching: None,
                agent: None,
            });
        };
        let cfg = driver_config_for(inst);
        let mut status = match depth {
            ProbeDepth::Full => {
                mira_acp::probe(d.as_ref(), &cfg, mira_acp::driver::PermissionMode::Ask).await
            }
            ProbeDepth::Background => {
                mira_acp::status::probe_background(
                    d.as_ref(),
                    &cfg,
                    mira_acp::driver::PermissionMode::Ask,
                )
                .await
            }
        };
        // An instance-level display override wins over the driver's own.
        if let Some(name) = &inst.display_name {
            status.display_name = name.clone();
        }
        // The probe already fetched the account-level catalog (Codex:
        // `model/list`). Carry it so the picker can show real rows instead
        // of "its model list appears once it has started". Previously this
        // was dropped (`models: Vec::new()`), so external rows never listed
        // models until a chat started one.
        let models: Vec<ModelInfo> = status
            .models
            .iter()
            .map(|m| ModelInfo {
                id: m.value.clone(),
                display_name: Some(m.label.clone()),
                owned_by: None,
                context_length: None,
                capabilities: None,
            })
            .collect();
        Some(EngineSnapshot {
            instance: inst.id.clone(),
            driver: inst.driver.clone(),
            flavor: EngineFlavor::External,
            display_name: status.display_name.clone(),
            enabled: inst.enabled,
            state: match status.state.clone() {
                mira_acp::status::AgentState::Ready => EngineState::Ready,
                mira_acp::status::AgentState::NotFound { looked_for } => {
                    EngineState::NotFound { looked_for }
                }
                mira_acp::status::AgentState::Failed { reason } => EngineState::Failed { reason },
            },
            models,
            default_model: inst.model.clone(),
            auth: status.auth.clone(),
            install_hint: status.install_hint.clone(),
            launch: Some(status.launch.clone()),
            credential_note: self.credential_sharing_note(inst),
            prompt_caching: None,
            agent: Some(status),
        })
    }

    /// Warn when two enabled instances of one driver would sign in as one
    /// account: same driver and same credential boundary means nothing
    /// separates them, however different the rest of their config is.
    /// Returns the picker-facing note, if any.
    fn credential_sharing_note(&self, inst: &EngineInstance) -> Option<String> {
        if inst.is_native() || !inst.enabled {
            return None;
        }
        let d = mira_acp::drivers::by_kind(inst.driver.as_str())?;
        let mine = d.credential_boundary(&driver_config_for(inst));
        let others: Vec<String> = self
            .instances
            .values()
            .filter(|o| o.id != inst.id && !o.is_native() && o.enabled && o.driver == inst.driver)
            .filter(|o| {
                mira_acp::drivers::by_kind(o.driver.as_str())
                    .is_some_and(|od| od.credential_boundary(&driver_config_for(o)) == mine)
            })
            .map(|o| o.display_name.clone().unwrap_or_else(|| o.id.to_string()))
            .collect();
        if others.is_empty() {
            return None;
        }
        Some(format!("shares credentials with {}", others.join(", ")))
    }

    /// Probe every external instance. Each probe is slow, so a few
    /// run at once — but not all: a fleet of Node-based agents starting
    /// simultaneously is the jank the cap avoids. A per-instance ceiling
    /// keeps one wedged agent from stalling the whole sweep.
    pub async fn snapshot_externals(&self) -> Vec<EngineSnapshot> {
        self.snapshot_externals_with(|_| {}, ProbeDepth::Full).await
    }

    /// [`Self::snapshot_externals`], reporting each agent as soon as its
    /// probe ends — so a picker can show Claude Code ready while a slower
    /// agent is still starting, instead of "checking…" for the whole sweep.
    /// The common agents go first.
    pub async fn snapshot_externals_with<F>(
        &self,
        on_each: F,
        depth: ProbeDepth,
    ) -> Vec<EngineSnapshot>
    where
        F: Fn(&EngineSnapshot) + Sync,
    {
        use futures::stream::{self, StreamExt};
        const PER_INSTANCE_CEILING: std::time::Duration = std::time::Duration::from_secs(75);
        let mut ids: Vec<(String, String)> = self
            .instances
            .values()
            .filter(|i| !i.is_native())
            .map(|i| (i.id.to_string(), i.driver.to_string()))
            .collect();
        let rank = |driver: &str| match driver {
            "claude-code" => 0,
            "codex" => 1,
            _ => 2,
        };
        ids.sort_by_key(|(_, driver)| rank(driver));
        let on_each = &on_each;
        stream::iter(ids.into_iter().map(|(id, _)| id))
            .map(move |id| async move {
                let snap = match tokio::time::timeout(
                    PER_INSTANCE_CEILING,
                    self.snapshot_external_with_depth(&id, depth),
                )
                .await
                {
                    Ok(snap) => snap,
                    Err(_) => {
                        // A wedged probe must still say something: vanishing
                        // from the list would look like "not installed".
                        let inst = self.instances.get(id.as_str())?;
                        Some(EngineSnapshot {
                            instance: inst.id.clone(),
                            driver: inst.driver.clone(),
                            flavor: EngineFlavor::External,
                            display_name: inst
                                .display_name
                                .clone()
                                .unwrap_or_else(|| inst.id.to_string()),
                            enabled: inst.enabled,
                            state: EngineState::Failed {
                                reason: format!(
                                    "probe did not finish within {}s",
                                    PER_INSTANCE_CEILING.as_secs()
                                ),
                            },
                            models: Vec::new(),
                            default_model: inst.model.clone(),
                            auth: None,
                            install_hint: None,
                            launch: None,
                            credential_note: None,
                            prompt_caching: None,
                            agent: None,
                        })
                    }
                };
                if let Some(s) = &snap {
                    on_each(s);
                }
                snap
            })
            .buffer_unordered(2)
            .filter_map(|s| async move { s })
            .collect()
            .await
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

    #[test]
    fn two_default_instances_of_one_driver_share_credentials() {
        // `codex` and `codex-work` with no home and no key resolve to the
        // same ambient boundary: one account wearing two names.
        let c = cfg("default_provider: openai\n\
             engines:\n  codex-work:\n    driver: codex\n");
        let reg = EngineRegistry::from_config(&c);
        let note = reg
            .credential_sharing_note(reg.get("codex-work").unwrap())
            .expect("sharing must be reported");
        assert!(note.contains("Codex"), "{note}");
        // ...unless the second instance brings its own credentials.
        let c = cfg("default_provider: openai\n\
             engines:\n  codex-work:\n    driver: codex\n    config:\n      home_path: /tmp/codex-work\n");
        let reg = EngineRegistry::from_config(&c);
        assert!(reg
            .credential_sharing_note(reg.get("codex-work").unwrap())
            .is_none());
        assert!(reg
            .credential_sharing_note(reg.get("codex").unwrap())
            .is_none());
    }

    #[test]
    fn sharing_ignores_disabled_instances_and_other_drivers() {
        let c = cfg("default_provider: openai\n\
             engines:\n  codex-work:\n    driver: codex\n    enabled: false\n");
        let reg = EngineRegistry::from_config(&c);
        assert!(
            reg.credential_sharing_note(reg.get("codex").unwrap())
                .is_none(),
            "a switched-off instance shares nothing"
        );
    }
}
