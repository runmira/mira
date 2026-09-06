//! Layered configuration for Mira.
//!
//! Sources, in decreasing precedence:
//!
//! 1. CLI flags (`--model`, `--mode`, `--api-key`, …) — resolved by the CLI.
//! 2. Environment variables (`MIRA_MODEL`, `MIRA_API_KEY`, …).
//! 3. Per-repo config at `<cwd>/.mira/config.yaml`.
//! 4. Global config at `~/.mira/mira.yaml`.
//! 5. Built-in defaults.
//!
//! Two files are supported so a user can keep credentials + preferred
//! model globally, and pin per-repo permission rules or a specific
//! model in the repo itself.
//!
//! Writing goes to the global file only — per-repo config is treated as
//! human-authored.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

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
    /// Third-party tools exposed via the Model Context Protocol. Each entry
    /// spawns a subprocess (or opens an HTTP session) at startup, discovers
    /// its tool list, and registers each tool as `mcp__<name>__<tool>`.
    pub mcp_servers: BTreeMap<String, McpServerConfig>,
}

/// One MCP server entry. `untagged` so the YAML shape is either a stdio
/// launch (`command` + optional `args`/`env`/`cwd`) or an HTTP endpoint
/// (`url` + optional `headers`) — no explicit `type:` field needed.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum McpServerConfig {
    Stdio(McpStdioConfig),
    Http(McpHttpConfig),
}

/// Launch an MCP server as a subprocess and speak the protocol over its
/// stdio.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct McpStdioConfig {
    pub command: String,
    pub args: Vec<String>,
    /// Extra env vars. Values pass through `${VAR}` expansion against the
    /// parent env, so `${GITHUB_TOKEN}` works without hard-coding secrets.
    pub env: BTreeMap<String, String>,
    pub cwd: Option<String>,
}

/// Connect to a remote MCP server via streamable HTTP.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct McpHttpConfig {
    pub url: String,
    /// Optional value for the `Authorization` header (e.g.
    /// `"Bearer ${MCP_TOKEN}"`). `${VAR}` is expanded against the parent env.
    pub auth: Option<String>,
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

    /// Load only the global file — used by the settings UI, which edits it
    /// directly and shouldn't mix in per-repo overrides.
    pub fn load_global() -> Result<Self> {
        Self::read_optional(global_path())
    }

    /// Serialize the global config back to `~/.mira/mira.yaml`, creating
    /// parent dirs if needed. Returns the path written.
    pub fn save_global(&self) -> Result<PathBuf> {
        let path = global_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("mkdir {}", parent.display()))?;
        }
        let yaml = serde_yaml::to_string(self).context("serialize config")?;
        std::fs::write(&path, yaml).with_context(|| format!("write {}", path.display()))?;
        Ok(path)
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
    pub fn merge(mut self, other: Self) -> Self {
        self.default_provider = other.default_provider.or(self.default_provider);
        self.default_model = other.default_model.or(self.default_model);
        self.default_mode = other.default_mode.or(self.default_mode);
        self.max_tokens = other.max_tokens.or(self.max_tokens);
        self.temperature = other.temperature.or(self.temperature);
        for (name, provider) in other.providers {
            self.providers.insert(name, provider);
        }
        for (name, server) in other.mcp_servers {
            self.mcp_servers.insert(name, server);
        }
        self.permissions.allow.extend(other.permissions.allow);
        self.permissions.ask.extend(other.permissions.ask);
        self.permissions.deny.extend(other.permissions.deny);
        self
    }
}

pub fn global_path() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(".mira")
        .join("mira.yaml")
}

/// Ephemeral cross-restart state — separate from [`MiraConfig`] because
/// this stuff is written by the app on every folder/model swap, not
/// hand-edited by the user. Kept alongside `mira.yaml` at
/// `~/.mira/state.yaml`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RuntimeState {
    /// Last folder the user picked in the web UI. Preferred over the
    /// process's launch cwd when `mira serve` boots.
    pub last_cwd: Option<PathBuf>,
    /// Last model the user picked. Preferred over `default_model` in
    /// `mira.yaml` — the yaml default is a fallback for first-run.
    pub last_model: Option<String>,
}

impl RuntimeState {
    pub fn load() -> Result<Self> {
        let path = state_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("read state {}", path.display()))?;
        serde_yaml::from_str(&raw).with_context(|| format!("parse state {}", path.display()))
    }

    /// Write the state back to `~/.mira/state.yaml`. Failures are the
    /// caller's problem — most callers should log-and-swallow rather than
    /// bail, since losing a preference-persist isn't fatal.
    pub fn save(&self) -> Result<PathBuf> {
        let path = state_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("mkdir {}", parent.display()))?;
        }
        let yaml = serde_yaml::to_string(self).context("serialize state")?;
        std::fs::write(&path, yaml).with_context(|| format!("write {}", path.display()))?;
        Ok(path)
    }
}

pub fn state_path() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(".mira")
        .join("state.yaml")
}

/// Sensible base_url defaults for well-known provider names.
pub fn default_base_url_for(name: &str) -> Option<&'static str> {
    Some(match name {
        "openrouter" => "https://openrouter.ai/api/v1",
        "openai" => "https://api.openai.com/v1",
        "anthropic" => "https://api.anthropic.com/v1",
        "groq" => "https://api.groq.com/openai/v1",
        "ollama" => "http://localhost:11434/v1",
        _ => return None,
    })
}

/// Conventional environment-variable name a well-known provider uses to
/// carry its API key. Used by error messages so we can suggest the right
/// `export` line for the user's provider without them having to look it up.
pub fn default_api_key_env_for(name: &str) -> Option<&'static str> {
    Some(match name {
        "openrouter" => "OPENROUTER_API_KEY",
        "openai" => "OPENAI_API_KEY",
        "anthropic" => "ANTHROPIC_API_KEY",
        "groq" => "GROQ_API_KEY",
        "together" => "TOGETHER_API_KEY",
        "fireworks" => "FIREWORKS_API_KEY",
        "perplexity" => "PERPLEXITY_API_KEY",
        "xai" => "XAI_API_KEY",
        "deepseek" => "DEEPSEEK_API_KEY",
        "moonshot" | "moonshotai" => "MOONSHOT_API_KEY",
        _ => return None,
    })
}

/// Human-facing display name for a known provider — Title-Cased so error
/// hints read naturally ("no API key found for Groq" not "for groq").
pub fn pretty_provider_name(name: &str) -> String {
    match name {
        "openrouter" => "OpenRouter".into(),
        "openai" => "OpenAI".into(),
        "anthropic" => "Anthropic".into(),
        "groq" => "Groq".into(),
        "together" => "Together".into(),
        "fireworks" => "Fireworks".into(),
        "perplexity" => "Perplexity".into(),
        "xai" => "xAI".into(),
        "deepseek" => "DeepSeek".into(),
        "moonshot" | "moonshotai" => "Moonshot AI".into(),
        "ollama" => "Ollama".into(),
        other => {
            let mut chars = other.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        }
    }
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
