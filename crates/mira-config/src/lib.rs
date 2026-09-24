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
    /// Cheaper model used only for rolling history compaction. Long
    /// sessions on Opus/Sonnet can compact with Haiku for a large cost
    /// drop; the compactor's job is single-shot summarization, so a
    /// smaller tier handles it fine. Unset falls back to the session's
    /// active model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compactor_model: Option<String>,
    pub providers: BTreeMap<String, ProviderConfig>,
    /// Named third-party API keys (search backends, docs services, …).
    /// Kept separate from `providers` because they aren't LLM providers —
    /// they're keys tools consume via their env-var convention.
    /// On startup, each entry is exported to the process env so tools
    /// that already read `BRAVE_SEARCH_API_KEY` etc. pick it up
    /// transparently. Users can still put keys in the shell env instead;
    /// yaml wins when both are set.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub keys: BTreeMap<String, String>,
    pub permissions: PermissionsConfig,
    /// Third-party tools exposed via the Model Context Protocol. Each entry
    /// spawns a subprocess (or opens an HTTP session) at startup, discovers
    /// its tool list, and registers each tool as `mcp__<name>__<tool>`.
    pub mcp_servers: BTreeMap<String, McpServerConfig>,
    /// Cross-session memory behavior — the auto-extractor and related knobs.
    /// Sensible defaults, so users get the feature without editing yaml.
    #[serde(default)]
    pub memory: MemoryRuntimeConfig,
    /// Desktop control (the `computer` tool). Off unless enabled here or
    /// with `--computer`.
    #[serde(default, skip_serializing_if = "ComputerUseConfig::is_empty")]
    pub computer: ComputerUseConfig,
    /// Browser automation (the `browser` tool). Off unless enabled here
    /// or with `--browser`.
    #[serde(default, skip_serializing_if = "BrowserConfig::is_empty")]
    pub browser: BrowserConfig,
    /// Where tools execute: the local machine (default) or a sandbox.
    /// Only read from the global file.
    #[serde(default, skip_serializing_if = "ComputeConfig::is_empty")]
    pub compute: ComputeConfig,
    /// Cloud tasks (`mira cloud run`). Only read from the global file.
    #[serde(default, skip_serializing_if = "CloudConfig::is_empty")]
    pub cloud: CloudConfig,
}

/// `cloud:` block: defaults for `mira cloud run`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct CloudConfig {
    /// Environment from `compute.environments` (must use the `e2b`
    /// backend). Default: the built-in `e2b`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    /// How Mira gets into the sandbox: `auto` (default: upload this binary
    /// when on Linux x86_64, else install the matching release),
    /// `preinstalled` (the template has it), `upload`, or `release`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install: Option<String>,
    /// Wall-clock budget per task. Match your E2B plan's session limit.
    /// Default 3600.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_runtime_secs: Option<u64>,
    /// Goal-loop iterations per task. Default 20.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_iterations: Option<usize>,
    /// Spend cap per task in USD (when the model's pricing is known).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_usd: Option<f64>,
    /// Cheaper model for the goal evaluator.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evaluator_model: Option<String>,
    /// Env var holding the GitHub token. Default: `GITHUB_TOKEN`, then
    /// `GH_TOKEN`, then `gh auth token`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github_token_env: Option<String>,
    /// Let the sandbox delete itself when the task is delivered (the E2B
    /// key goes into the sandbox for that). Default true; without it the
    /// sandbox idles until its timeout.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_shutdown: Option<bool>,
}

impl CloudConfig {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// `compute:` block: named remote environments and their defaults.
///
/// ```yaml
/// compute:
///   default: dev            # used by `--sandbox` with no name, and at startup
///   e2b:
///     api_key_env: E2B_API_KEY
///   environments:
///     dev:
///       backend: e2b
///       template: base
///       env: { RUST_LOG: debug }
///       setup: |
///         cargo fetch
/// ```
///
/// `scratch` (a copy of the project on this machine) and `e2b` (an E2B
/// sandbox with default settings) are always available without being
/// declared here. `local` is reserved for the user's own machine.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ComputeConfig {
    /// Environment to start sessions in. Unset: tools run on this
    /// machine unless `--sandbox` / `/remote-env` picks one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    /// Account-level E2B settings shared by every `e2b` environment.
    #[serde(skip_serializing_if = "E2bConfig::is_empty")]
    pub e2b: E2bConfig,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub environments: BTreeMap<String, EnvironmentConfig>,
}

impl ComputeConfig {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// One named environment under `compute.environments`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct EnvironmentConfig {
    /// `e2b` or `scratch`.
    pub backend: String,
    /// One line shown in `/remote-env` listings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// E2B template (image). Overrides `compute.e2b.template`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    /// Idle lifetime in seconds. Overrides `compute.e2b.timeout_secs`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    /// Environment variables for every command in the environment.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// Shell script run once after the project is first uploaded
    /// (install toolchains, fetch dependencies, …).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub setup: Option<String>,
}

/// `compute.e2b:` settings.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct E2bConfig {
    /// Env var holding the API key. Default `E2B_API_KEY` (which can
    /// also live under `keys:`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    /// Sandbox template. Default `base`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    /// Idle lifetime in seconds, refreshed while in use. Default 3600.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    /// Control-plane URL. Default `https://api.e2b.app`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_url: Option<String>,
    /// Sandbox domain. Default `e2b.app`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
}

impl E2bConfig {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Name of the env var the API key is read from.
    pub fn api_key_env(&self) -> &str {
        self.api_key_env.as_deref().unwrap_or("E2B_API_KEY")
    }
}

/// `computer:` block. Every field is optional so a per-repo file can
/// tune one knob; `enabled` itself is only honored from the global file
/// (see [`MiraConfig::merge`]).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ComputerUseConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Screenshot after every mouse/keyboard action. Default on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screenshot_after_action: Option<bool>,
    /// Longest screenshot edge sent to the model, in pixels. Default
    /// 1568. Lower it (e.g. 1280) to cut image-token cost.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_long_edge: Option<u32>,
    /// Pause before the follow-up screenshot, in ms. Default 600.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settle_ms: Option<u64>,
}

impl ComputerUseConfig {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    pub fn enabled(&self) -> bool {
        self.enabled.unwrap_or(false)
    }
}

/// `browser:` block. Same global-only rule for `enabled`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct BrowserConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Browser binary. Unset auto-detects Chrome, Chromium, Edge, Brave.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable: Option<String>,
    /// Run without a window. Default: headed when a display exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headless: Option<bool>,
    /// Profile directory (cookies, logins). Default
    /// `~/.mira/browser/profile` — never the user's everyday profile.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_dir: Option<String>,
}

impl BrowserConfig {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// `executable` with a leading `~` expanded.
    pub fn executable_path(&self) -> Option<PathBuf> {
        self.executable.as_deref().map(expand_tilde)
    }

    /// `profile_dir` with a leading `~` expanded.
    pub fn profile_dir_path(&self) -> Option<PathBuf> {
        self.profile_dir.as_deref().map(expand_tilde)
    }

    pub fn enabled(&self) -> bool {
        self.enabled.unwrap_or(false)
    }
}

/// Runtime knobs for the auto-extractor (post-round background pass that
/// writes durable facts to `<cwd>/.mira/episodic.jsonl`).
///
/// Fields are `Option`-typed so per-repo config can override a single knob
/// (e.g. turn auto-extract off for a specific repo) without having to
/// restate the whole block. Use [`Self::auto_extract_enabled`] and
/// [`Self::extractor_model`] to read the *effective* value with defaults
/// applied.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct MemoryRuntimeConfig {
    /// Master switch. Default on: cross-session memory is only meaningful if
    /// something accumulates without the user asking. Set to `false` in
    /// yaml to disable auto-extraction while keeping the `memory_remember`
    /// tool available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_extract: Option<bool>,
    /// Model used for the extraction call. Unset falls back to the
    /// session's active model (expensive but always works). Point this at
    /// the provider's cheap tier — e.g. `"claude-haiku-4-5"` on Anthropic,
    /// `"gpt-5-nano"` on OpenAI — to keep per-round cost negligible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extractor_model: Option<String>,
    /// Register the `memory_*` agent tools. Default on. Turn off to shrink
    /// the tool list and see whether tool-count is causing over-exploration
    /// — useful as a bisect knob when the model seems distracted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools_enabled: Option<bool>,
    /// Inject the live memory block (user + project MIRA.md + episodic tail)
    /// into the system prompt each round. Default on. Turn off to prove
    /// out whether the injected content is priming exploratory tool use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inject_context: Option<bool>,
    /// Score-and-select memory entries against the current turn's
    /// context instead of dumping every file wholesale. Default on
    /// (harness-level). Turn off to bisect whether retrieval or memory
    /// itself is causing a regression, or when you want the model to
    /// always see every entry regardless of turn topic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retrieval_enabled: Option<bool>,
    /// Token budget for the rendered memory block when retrieval is on.
    /// Approximate: `chars / 4`. Default 1500 tokens (~6 KB) — enough
    /// for ~30 medium bullets. Bump for verbose memory files, drop if
    /// you're squeezing every last token for the conversation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retrieval_token_budget: Option<u32>,
}

impl MemoryRuntimeConfig {
    /// Effective on/off — defaults to `true` when unset.
    pub fn auto_extract_enabled(&self) -> bool {
        self.auto_extract.unwrap_or(true)
    }

    /// Configured extractor model, if any. `None` means "use the session's
    /// active model."
    pub fn extractor_model(&self) -> Option<&str> {
        self.extractor_model.as_deref()
    }

    /// Whether the memory tools should be registered. Defaults to `true`.
    pub fn tools_enabled(&self) -> bool {
        self.tools_enabled.unwrap_or(true)
    }

    /// Whether the live memory block should be injected into the system
    /// prompt. Defaults to `true`.
    pub fn inject_context(&self) -> bool {
        self.inject_context.unwrap_or(true)
    }

    /// Whether retrieval-based selection is on. Defaults to `true`.
    pub fn retrieval_enabled(&self) -> bool {
        self.retrieval_enabled.unwrap_or(true)
    }

    /// Effective token budget for retrieval. `None` in yaml → harness
    /// default (from `mira_memory::DEFAULT_TOKEN_BUDGET`).
    pub fn retrieval_token_budget(&self) -> Option<u32> {
        self.retrieval_token_budget
    }
}

/// One MCP server entry. `untagged` so the YAML shape is either a stdio
/// launch (`command` + optional `args`/`env`/`cwd`) or an HTTP endpoint
/// (`url` + optional `headers`) — no explicit `type:` field needed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum McpServerConfig {
    Stdio(McpStdioConfig),
    Http(McpHttpConfig),
}

/// Launch an MCP server as a subprocess and speak the protocol over its
/// stdio.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Opt in to Anthropic-style prompt caching (marks the system prompt
    /// with `cache_control: ephemeral`). `None` = auto: on for the
    /// `anthropic` provider name and any base_url containing `anthropic.com`;
    /// off otherwise. Set explicitly to force one way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_caching: Option<bool>,
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

/// Export every key in `cfg.keys` to the current process env so tools
/// that read via `std::env::var(...)` (e.g. `BRAVE_SEARCH_API_KEY`) pick
/// them up without any further wiring. Does not overwrite existing env
/// values — an env-set key wins over a yaml-stored one, matching the
/// "yaml is fallback for env" mental model.
pub fn export_keys_to_env(cfg: &MiraConfig) {
    for (name, value) in &cfg.keys {
        if std::env::var_os(name).is_none() && !value.is_empty() {
            // Safety: setting env vars is documented as `unsafe` in Rust
            // 2024, but our callers are all on startup / single-threaded
            // boot paths. Wrapping so this compiles on both editions.
            #[allow(unused_unsafe)]
            unsafe {
                std::env::set_var(name, value);
            }
        }
    }
}

/// Effective prompt-caching decision given a provider name, resolved base
/// URL, and the (possibly None) explicit override from `ProviderConfig`.
/// The rule: explicit `Some(x)` always wins; otherwise auto-enable for
/// Anthropic-flavored endpoints and leave off for everything else.
pub fn prompt_caching_enabled(name: &str, base_url: &str, explicit: Option<bool>) -> bool {
    if let Some(v) = explicit {
        return v;
    }
    name.eq_ignore_ascii_case("anthropic") || base_url.contains("anthropic.com")
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
        self.compactor_model = other.compactor_model.or(self.compactor_model);
        for (name, provider) in other.providers {
            self.providers.insert(name, provider);
        }
        for (name, key) in other.keys {
            self.keys.insert(name, key);
        }
        for (name, server) in other.mcp_servers {
            self.mcp_servers.insert(name, server);
        }
        self.permissions.allow.extend(other.permissions.allow);
        self.permissions.ask.extend(other.permissions.ask);
        self.permissions.deny.extend(other.permissions.deny);
        // Memory: per-field merge. `other` (per-repo) wins if it set the
        // field; otherwise the global value stays.
        self.memory.auto_extract = other.memory.auto_extract.or(self.memory.auto_extract);
        self.memory.extractor_model = other.memory.extractor_model.or(self.memory.extractor_model);
        self.memory.tools_enabled = other.memory.tools_enabled.or(self.memory.tools_enabled);
        self.memory.inject_context = other.memory.inject_context.or(self.memory.inject_context);
        self.memory.retrieval_enabled = other
            .memory
            .retrieval_enabled
            .or(self.memory.retrieval_enabled);
        self.memory.retrieval_token_budget = other
            .memory
            .retrieval_token_budget
            .or(self.memory.retrieval_token_budget);
        // Computer / browser: per-repo files may tune knobs but never
        // switch the tools on — a cloned repo must not be able to grant
        // itself control of the user's desktop or browser.
        let c = other.computer;
        self.computer.screenshot_after_action = c
            .screenshot_after_action
            .or(self.computer.screenshot_after_action);
        self.computer.max_long_edge = c.max_long_edge.or(self.computer.max_long_edge);
        self.computer.settle_ms = c.settle_ms.or(self.computer.settle_ms);
        let b = other.browser;
        self.browser.headless = b.headless.or(self.browser.headless);
        // `compute` and `cloud` are global-only: a repo must not be able to
        // decide that its code gets shipped to a third-party sandbox.
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

/// Expand a leading `~` / `~/` to `$HOME`.
fn expand_tilde(p: &str) -> PathBuf {
    match p.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default();
            home.join(rest.trim_start_matches('/'))
        }
        _ => PathBuf::from(p),
    }
}

/// `~/.mira/MIRA.md` — user-global memory file path. Kept here (config
/// crate) because the memory *reader/writer* lives in `mira-memory` and
/// takes concrete paths; the path convention itself is config-shaped.
pub fn user_memory_path() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(".mira")
        .join("MIRA.md")
}

/// `<cwd>/.mira/MIRA.md` — project-local memory file path.
pub fn project_memory_path(cwd: &Path) -> PathBuf {
    cwd.join(".mira").join("MIRA.md")
}

/// `~/.mira/skills/` — user-global skills directory. Every `*.md` file
/// under here overrides bundled skills of the same name.
pub fn user_skills_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(".mira")
        .join("skills")
}

/// `~/.agents/skills/` — the emerging cross-tool convention for locally
/// installed skills. `npx skills add owner/pkg@name` (mattpocock's
/// installer, Codex, and other agents that follow the same convention)
/// drops files here, so scanning it means the same one-line install
/// command works for Mira with zero extra tooling.
pub fn shared_skills_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(".agents")
        .join("skills")
}

/// `<cwd>/.mira/skills/` — project-local skills directory. Files here
/// override same-named user + bundled skills, so a repo can ship its
/// own "how we deploy" or "how we release" playbook.
pub fn project_skills_dir(cwd: &Path) -> PathBuf {
    cwd.join(".mira").join("skills")
}

/// `<cwd>/.agents/skills/` — project-local cross-tool convention.
/// `npx skills add …` drops files here when the current directory
/// already has an `.agents` folder (Codex-style project layout).
/// Scanned alongside `<cwd>/.mira/skills/` so both work interchangeably.
pub fn project_agents_skills_dir(cwd: &Path) -> PathBuf {
    cwd.join(".agents").join("skills")
}

/// Well-known project-local skill directories, in precedence order —
/// later entries override same-named skills from earlier ones. Includes
/// every convention we've seen in the wild:
///
///   `<cwd>/.mira/skills/`     — Mira's own convention
///   `<cwd>/.agents/skills/`   — cross-tool "agents" convention
///   `<cwd>/.claude/skills/`   — Claude Code
///   `<cwd>/.codex/skills/`    — Codex
///   `<cwd>/.cursor/skills/`   — Cursor
///
/// The `npx skills add …` installer picks whichever of these already
/// exists in the project, so scanning the whole set means the same
/// one-line install works whether the user has one, several, or none
/// of the tool-specific folders.
pub fn well_known_project_skills_dirs(cwd: &Path) -> Vec<PathBuf> {
    vec![
        cwd.join(".mira").join("skills"),
        cwd.join(".agents").join("skills"),
        cwd.join(".claude").join("skills"),
        cwd.join(".codex").join("skills"),
        cwd.join(".cursor").join("skills"),
    ]
}

/// Sensible base_url defaults for well-known provider names. Keep in
/// sync with the frontend's `PROVIDER_PRESETS` in
/// `mira-server/frontend/src/components/Settings.tsx` — the two lists
/// should agree so the CLI and web UI resolve the same URL from the
/// same short name.
pub fn default_base_url_for(name: &str) -> Option<&'static str> {
    Some(match name {
        // Gateways
        "openrouter" => "https://openrouter.ai/api/v1",
        // Major hosted
        "openai" => "https://api.openai.com/v1",
        "anthropic" => "https://api.anthropic.com/v1",
        "google" => "https://generativelanguage.googleapis.com/v1beta/openai",
        // Fast / cheap inference
        "deepseek" => "https://api.deepseek.com/v1",
        "groq" => "https://api.groq.com/openai/v1",
        "cerebras" => "https://api.cerebras.ai/v1",
        "xai" => "https://api.x.ai/v1",
        // Model bazaars
        "together" => "https://api.together.xyz/v1",
        "fireworks" => "https://api.fireworks.ai/inference/v1",
        "hyperbolic" => "https://api.hyperbolic.xyz/v1",
        "novita" => "https://api.novita.ai/v3/openai",
        // Search-augmented + regionals
        "perplexity" => "https://api.perplexity.ai",
        "mistral" => "https://api.mistral.ai/v1",
        "moonshot" | "moonshotai" => "https://api.moonshot.ai/v1",
        // Local runtimes
        "ollama" => "http://localhost:11434/v1",
        "lmstudio" => "http://localhost:1234/v1",
        "llamacpp" => "http://localhost:8080/v1",
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
        // Google's OpenAI-compat endpoint accepts a Gemini API key.
        // GEMINI_API_KEY is the conventional name in google's docs.
        "google" => "GEMINI_API_KEY",
        "groq" => "GROQ_API_KEY",
        "cerebras" => "CEREBRAS_API_KEY",
        "together" => "TOGETHER_API_KEY",
        "fireworks" => "FIREWORKS_API_KEY",
        "hyperbolic" => "HYPERBOLIC_API_KEY",
        "novita" => "NOVITA_API_KEY",
        "perplexity" => "PERPLEXITY_API_KEY",
        "mistral" => "MISTRAL_API_KEY",
        "xai" => "XAI_API_KEY",
        "deepseek" => "DEEPSEEK_API_KEY",
        "moonshot" | "moonshotai" => "MOONSHOT_API_KEY",
        // Local runtimes typically don't require a key — leaving no
        // env-var hint is the right signal.
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
        "google" => "Google (Gemini)".into(),
        "groq" => "Groq".into(),
        "cerebras" => "Cerebras".into(),
        "together" => "Together".into(),
        "fireworks" => "Fireworks".into(),
        "hyperbolic" => "Hyperbolic".into(),
        "novita" => "Novita".into(),
        "perplexity" => "Perplexity".into(),
        "mistral" => "Mistral".into(),
        "xai" => "xAI".into(),
        "deepseek" => "DeepSeek".into(),
        "moonshot" | "moonshotai" => "Moonshot AI".into(),
        "ollama" => "Ollama".into(),
        "lmstudio" => "LM Studio".into(),
        "llamacpp" => "llama.cpp".into(),
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
    fn compute_block_is_global_only() {
        let global: MiraConfig = serde_yaml::from_str(
            "compute:\n  default: dev\n  environments:\n    dev:\n      backend: e2b\n      \
             env: {A: b}\n      setup: make deps\n",
        )
        .unwrap();
        let local: MiraConfig = serde_yaml::from_str(
            "compute:\n  default: evil\n  e2b:\n    template: evil\n  environments:\n    \
             evil:\n      backend: e2b\n",
        )
        .unwrap();
        let merged = global.merge(local);
        assert_eq!(merged.compute.default.as_deref(), Some("dev"));
        assert!(!merged.compute.environments.contains_key("evil"));
        let dev = &merged.compute.environments["dev"];
        assert_eq!(dev.env["A"], "b");
        assert_eq!(dev.setup.as_deref(), Some("make deps"));
        assert!(merged.compute.e2b.template.is_none());
        assert_eq!(merged.compute.e2b.api_key_env(), "E2B_API_KEY");
    }

    #[test]
    fn per_repo_config_cannot_enable_computer_or_browser() {
        let global = MiraConfig::default();
        let local: MiraConfig = serde_yaml::from_str(
            "computer:\n  enabled: true\n  max_long_edge: 1024\n\
             browser:\n  enabled: true\n  executable: /tmp/evil\n  headless: true\n",
        )
        .unwrap();
        let merged = global.merge(local);
        assert!(!merged.computer.enabled());
        assert_eq!(merged.computer.max_long_edge, Some(1024));
        assert!(!merged.browser.enabled());
        assert!(merged.browser.executable.is_none());
        assert_eq!(merged.browser.headless, Some(true));

        let global: MiraConfig =
            serde_yaml::from_str("computer:\n  enabled: true\nbrowser:\n  enabled: true\n")
                .unwrap();
        let merged = global.merge(MiraConfig::default());
        assert!(merged.computer.enabled() && merged.browser.enabled());
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

    /* ---- provider preset registry ---- */

    /// The full roster the UI's PROVIDER_PRESETS list ships. Keeping
    /// the test hardcoded so a UI/config drift shows up as a build
    /// failure rather than a silent gap where the CLI can't resolve
    /// a name the picker offers.
    const EXPECTED_PROVIDERS: &[&str] = &[
        "openrouter",
        "openai",
        "anthropic",
        "google",
        "deepseek",
        "groq",
        "cerebras",
        "xai",
        "together",
        "fireworks",
        "hyperbolic",
        "novita",
        "perplexity",
        "mistral",
        "moonshot",
        "ollama",
        "lmstudio",
        "llamacpp",
    ];

    #[test]
    fn every_expected_provider_has_a_base_url() {
        for name in EXPECTED_PROVIDERS {
            assert!(
                default_base_url_for(name).is_some(),
                "missing base URL for `{name}`"
            );
        }
    }

    #[test]
    fn hosted_providers_declare_an_api_key_env() {
        // Local runtimes (ollama, lmstudio, llamacpp) don't need a
        // key — everything else should have a conventional env var.
        for name in EXPECTED_PROVIDERS
            .iter()
            .filter(|n| !matches!(**n, "ollama" | "lmstudio" | "llamacpp"))
        {
            assert!(
                default_api_key_env_for(name).is_some(),
                "hosted provider `{name}` should declare an api-key env var"
            );
        }
    }

    #[test]
    fn every_expected_provider_has_a_pretty_name() {
        for name in EXPECTED_PROVIDERS {
            let pretty = pretty_provider_name(name);
            assert!(!pretty.is_empty(), "empty pretty name for `{name}`");
            // Not just the raw input (which would mean the fallback
            // ran) — every listed provider deserves an intentional
            // display name. Brands that go lowercase-first on
            // purpose (`xAI`, `llama.cpp`) are still fine because
            // the whole string differs from the raw slug.
            assert_ne!(
                pretty, *name,
                "pretty name for `{name}` should have an explicit branch, got the raw slug"
            );
        }
    }

    #[test]
    fn moonshot_and_moonshotai_alias() {
        // The picker uses `moonshot` but historic yaml files may say
        // `moonshotai` — both must resolve.
        assert_eq!(
            default_api_key_env_for("moonshotai"),
            default_api_key_env_for("moonshot"),
        );
        assert_eq!(
            pretty_provider_name("moonshotai"),
            pretty_provider_name("moonshot"),
        );
    }
}
