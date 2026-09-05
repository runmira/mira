//! Layered configuration for the Mira CLI.
//!
//! Sources, in decreasing precedence:
//!
//! 1. CLI flags (`--model`, `--mode`, `--api-key`, …)
//! 2. Environment variables (`MIRA_MODEL`, `MIRA_API_KEY`, …)
//! 3. Per-repo config at `<cwd>/.mira/config.yaml`
//! 4. Global config at `~/.mira/mira.yaml`
//! 5. Built-in defaults
//!
//! Two files are supported so a user can keep credentials + preferred
//! model globally, and pin per-repo permission rules or a specific
//! model in the repo itself.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// The whole merged view of `mira.yaml` — everything the CLI reads to
/// decide what to do with a run. Every field is optional so partial
/// files are legal and merging is trivial.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct MiraConfig {
    pub default_provider: Option<String>,
    pub default_model: Option<String>,
    pub default_mode: Option<String>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub providers: BTreeMap<String, ProviderConfig>,
    pub permissions: PermissionsConfig,
}

/// One OpenAI-compatible endpoint. `api_key_env` names an env var to
/// pull the key from; `api_key` is only used for local-only endpoints
/// (llama.cpp, LM Studio) where the key doesn't need protecting.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ProviderConfig {
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub extra_headers: BTreeMap<String, String>,
}

impl ProviderConfig {
    /// Resolve the API key: literal → named env var → fallback to
    /// `MIRA_API_KEY`. Returns `None` if none of them are set.
    pub fn resolved_api_key(&self) -> Option<String> {
        if let Some(k) = &self.api_key {
            return Some(k.clone());
        }
        if let Some(name) = &self.api_key_env {
            if let Ok(v) = std::env::var(name) {
                return Some(v);
            }
        }
        std::env::var("MIRA_API_KEY").ok()
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct PermissionsConfig {
    pub allow: Vec<String>,
    pub ask: Vec<String>,
    pub deny: Vec<String>,
}

impl MiraConfig {
    /// Load and merge the two files. Missing files are treated as empty
    /// configs — starting Mira with zero config on disk is fine.
    pub fn load(cwd: &Path) -> Result<Self> {
        let global = Self::read_optional(global_path())?;
        let local = Self::read_optional(cwd.join(".mira").join("config.yaml"))?;
        Ok(global.merge(local))
    }

    fn read_optional(path: PathBuf) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("read config {}", path.display()))?;
        serde_yaml::from_str(&raw).with_context(|| format!("parse config {}", path.display()))
    }

    /// Field-by-field merge. Scalars: `other` wins if set. Providers:
    /// entries in `other` add or replace by name. Permissions:
    /// concatenated so per-repo can extend global.
    fn merge(mut self, other: Self) -> Self {
        self.default_provider = other.default_provider.or(self.default_provider);
        self.default_model = other.default_model.or(self.default_model);
        self.default_mode = other.default_mode.or(self.default_mode);
        self.max_tokens = other.max_tokens.or(self.max_tokens);
        self.temperature = other.temperature.or(self.temperature);
        for (name, provider) in other.providers {
            self.providers.insert(name, provider);
        }
        self.permissions.allow.extend(other.permissions.allow);
        self.permissions.ask.extend(other.permissions.ask);
        self.permissions.deny.extend(other.permissions.deny);
        self
    }
}

fn global_path() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(".mira")
        .join("mira.yaml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_files_yield_default() {
        let c = MiraConfig::default();
        assert!(c.default_model.is_none());
        assert!(c.providers.is_empty());
    }

    #[test]
    fn merge_prefers_local_scalars() {
        let global = MiraConfig {
            default_model: Some("global-model".into()),
            default_mode: Some("manual".into()),
            ..Default::default()
        };
        let local = MiraConfig {
            default_model: Some("local-model".into()),
            ..Default::default()
        };
        let merged = global.merge(local);
        assert_eq!(merged.default_model.as_deref(), Some("local-model"));
        assert_eq!(merged.default_mode.as_deref(), Some("manual"));
    }

    #[test]
    fn merge_concatenates_permissions() {
        let mut global = MiraConfig::default();
        global.permissions.allow = vec!["Bash(cargo test:*)".into()];
        let mut local = MiraConfig::default();
        local.permissions.allow = vec!["Edit(src/**)".into()];
        let merged = global.merge(local);
        assert_eq!(merged.permissions.allow.len(), 2);
    }

    #[test]
    fn providers_add_and_replace() {
        let mut global = MiraConfig::default();
        global.providers.insert(
            "openrouter".into(),
            ProviderConfig {
                base_url: Some("g-url".into()),
                ..Default::default()
            },
        );
        let mut local = MiraConfig::default();
        local.providers.insert(
            "openrouter".into(),
            ProviderConfig {
                base_url: Some("l-url".into()),
                ..Default::default()
            },
        );
        local.providers.insert(
            "local".into(),
            ProviderConfig {
                base_url: Some("http://localhost".into()),
                ..Default::default()
            },
        );
        let merged = global.merge(local);
        assert_eq!(
            merged.providers["openrouter"].base_url.as_deref(),
            Some("l-url")
        );
        assert!(merged.providers.contains_key("local"));
    }
}
