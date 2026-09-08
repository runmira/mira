mod approver;
mod config;
mod repl;
mod review;
mod serve;
mod tui;

// Entry point for the mira CLI binary.

use std::io::IsTerminal;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use mira_ai::openai::{OpenAiCompatible, OpenAiConfig};
use mira_core::SessionId;
use mira_harness::{Approver, FileStore, Session, SessionConfig, SessionStore};
use mira_policy::{Mode, Policy, PolicyConfig};
use mira_sandbox::Sandbox;
use mira_tools::{builtin, Registry, ToolContext};
use tokio::sync::Mutex;
use tracing_subscriber::EnvFilter;

use crate::config::{MiraConfig, ProviderConfig};

/// Mira — an open-source coding agent.
///
/// Every flag can also be set in `~/.mira/mira.yaml` (global) or
/// `<cwd>/.mira/config.yaml` (per-repo). CLI flags win over env vars,
/// which win over per-repo config, which wins over global config.
#[derive(Parser, Debug, Clone)]
#[command(version, about, long_about = None)]
pub(crate) struct Cli {
    /// Provider name — a key in `providers` in your `mira.yaml`.
    #[arg(long)]
    provider: Option<String>,

    /// Base URL of an OpenAI-compatible endpoint. Overrides the provider's
    /// `base_url` from config.
    #[arg(long)]
    base_url: Option<String>,

    /// API key for the endpoint. Overrides the provider's key from config
    /// or its `api_key_env` env variable.
    #[arg(long)]
    api_key: Option<String>,

    /// Model ID.
    #[arg(long)]
    model: Option<String>,

    /// Permission mode: plan|manual|auto|edit|yolo.
    #[arg(long)]
    mode: Option<String>,

    /// Cap on tokens the model may produce per turn.
    #[arg(long)]
    max_tokens: Option<u32>,

    /// Sampling temperature.
    #[arg(long)]
    temperature: Option<f32>,

    /// Force the simple line-based REPL even when running in a TTY.
    /// Piped input (e.g. `echo foo | mira`) always uses the REPL.
    #[arg(long)]
    simple: bool,

    /// Resume a previous session. Without an ID, picks the most recent
    /// session for the current directory.
    #[arg(long, value_name = "ID", num_args = 0..=1, default_missing_value = "")]
    resume: Option<String>,

    /// Skip persistence entirely — sessions are not saved to disk.
    #[arg(long)]
    no_persist: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

/// Subcommands. Absent = the default chat entrypoint (TUI or REPL).
#[derive(Subcommand, Debug, Clone)]
enum Command {
    /// Run Mira as a local web server. Binds to 127.0.0.1 by default; a
    /// browser (or, later, the desktop app) is the frontend.
    Serve(serve::ServeArgs),
    /// Two-stage diff review: generate findings, then hostile re-verify.
    Review(review::ReviewArgs),
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Subcommand branch — `mira serve` short-circuits the TUI path.
    if let Some(Command::Serve(args)) = cli.command.clone() {
        init_tracing(false);
        return serve::run(&cli, args).await;
    }
    if let Some(Command::Review(args)) = cli.command.clone() {
        init_tracing(false);
        return review::run(&cli, args).await;
    }

    let use_tui = !cli.simple && std::io::stdin().is_terminal() && std::io::stdout().is_terminal();

    init_tracing(use_tui);

    let cwd = std::env::current_dir().context("failed to read cwd")?;
    let cfg = MiraConfig::load(&cwd).context("load config")?;
    // Materialise `keys:` into the process env so tools like web_search
    // read them via their existing env-var conventions without extra
    // plumbing. Yaml never overwrites a shell-set env value.
    mira_config::export_keys_to_env(&cfg);

    // --- resolve settings across CLI / env / config / defaults
    let settings = resolve_settings(&cli, &cfg)?;

    // --- provider
    let provider = Arc::new(
        OpenAiCompatible::new(OpenAiConfig {
            base_url: settings.base_url,
            api_key: settings.api_key,
            extra_headers: settings.extra_headers,
            prompt_caching: settings.prompt_caching,
        })
        .context("build provider")?,
    );

    // --- tools + sandbox
    let sandbox = Arc::new(Sandbox::default_scrubbed());
    let mut registry = Registry::new();
    builtin::register_core(&mut registry);
    if cfg.memory.tools_enabled() {
        builtin::register_memory(&mut registry);
    }
    // Configured MCP servers layer on top of the built-ins. A single broken
    // entry mustn't stop Mira from starting, so failures degrade to a
    // warning and we move on.
    for (name, server_cfg) in &cfg.mcp_servers {
        match mira_tools::connect_mcp(name, server_cfg).await {
            Ok(conn) => {
                for tool in conn.tools {
                    registry.register_arc(tool);
                }
            }
            Err(e) => {
                eprintln!("warning: mcp `{name}` disabled ({e:#})");
            }
        }
    }
    let registry = Arc::new(registry);
    // Shared memory + episodic stores. `memory_remember` writes episodic
    // entries here, and the memory snapshot below reads from the same
    // instance so the writer and the snapshot renderer share a mutex.
    let memory_store: Arc<dyn mira_memory::MemoryStore> =
        Arc::new(mira_memory::FileMemoryStore::new(
            mira_config::user_memory_path(),
            mira_config::project_memory_path(&cwd),
        ));
    let episodic_store: Arc<dyn mira_memory::EpisodicStore> = Arc::new(
        mira_memory::FileEpisodicStore::new(mira_memory::project_episodic_path(&cwd)),
    );
    let tool_ctx = ToolContext::new(cwd.clone(), sandbox)
        .with_memory(memory_store)
        .with_episodic(episodic_store.clone());

    // --- policy: rules from config, mode from CLI/config/default
    let policy = Policy::from_config(&PolicyConfig {
        mode: settings.mode,
        allow: cfg.permissions.allow.clone(),
        ask: cfg.permissions.ask.clone(),
        deny: cfg.permissions.deny.clone(),
    })
    .context("compile policy")?;
    let policy = Arc::new(Mutex::new(policy));

    // --- approver (branch on frontend)
    let (approver, approval_rx) = if use_tui {
        let (a, rx) = tui::TuiApprover::new();
        (a as Arc<dyn Approver>, Some(rx))
    } else {
        (
            Arc::new(approver::TerminalApprover) as Arc<dyn Approver>,
            None,
        )
    };

    // --- store (optional)
    let store: Option<Arc<dyn SessionStore>> = if cli.no_persist {
        None
    } else {
        match FileStore::open_default() {
            Ok(s) => Some(Arc::new(s)),
            Err(e) => {
                eprintln!("warning: persistence disabled ({e})");
                None
            }
        }
    };

    // --- session (fresh or resumed)
    let mut sess_cfg = SessionConfig::new(settings.model.clone());
    sess_cfg.max_tokens = settings.max_tokens;
    sess_cfg.temperature = settings.temperature;

    let session = match resume_target(cli.resume.as_deref(), store.as_deref(), &cwd).await? {
        Some(record) => Session::resume_from(
            record,
            provider,
            registry,
            policy.clone(),
            approver,
            tool_ctx,
        ),
        None => Session::new(
            sess_cfg,
            system_prompt(&cwd, &registry),
            provider,
            registry,
            policy.clone(),
            approver,
            tool_ctx,
        ),
    };
    let session = if let Some(s) = store.clone() {
        session.with_store(s)
    } else {
        session
    };
    // Live memory: reload user + project MIRA.md on every provider round,
    // plus tail the most-recent episodic entries so cross-session memory
    // is visible immediately after `memory_remember` writes it.
    // `memory.inject_context: false` skips the snapshot wiring entirely,
    // so the model sees exactly the same prompt as before the memory
    // work landed — a clean bisect switch.
    let mut session = session;
    if cfg.memory.inject_context() {
        session = session.with_memory_snapshot(mira_server::make_memory_snapshot_with(
            &cwd,
            episodic_store,
        ));
    }
    if cfg.memory.auto_extract_enabled() {
        session = session.with_auto_extract(mira_harness::AutoExtractConfig::enabled(
            cfg.memory.extractor_model().map(str::to_owned),
        ));
    }
    let session = session;

    if use_tui {
        tui::run(
            session,
            tui::TuiConfig {
                model: settings.model,
                mode: settings.mode,
                policy,
                approval_rx: approval_rx.expect("tui branch created a receiver"),
                cwd: cwd.clone(),
            },
        )
        .await
    } else {
        repl::run(session).await
    }
}

/// Values that survive the CLI/env/config/default cascade and get passed
/// down to the provider, policy, and session.
pub(crate) struct ResolvedSettings {
    pub(crate) base_url: String,
    pub(crate) api_key: String,
    pub(crate) model: String,
    pub(crate) mode: Mode,
    pub(crate) max_tokens: Option<u32>,
    pub(crate) temperature: Option<f32>,
    pub(crate) extra_headers: Vec<(String, String)>,
    /// Effective prompt-caching flag after applying the config override
    /// and the auto-enable-for-Anthropic rule.
    pub(crate) prompt_caching: bool,
}

pub(crate) fn resolve_settings(cli: &Cli, cfg: &MiraConfig) -> Result<ResolvedSettings> {
    // Provider selection: --provider > MIRA_PROVIDER env > config default > "openrouter".
    let provider_name = cli
        .provider
        .clone()
        .or_else(|| std::env::var("MIRA_PROVIDER").ok())
        .or_else(|| cfg.default_provider.clone())
        .unwrap_or_else(|| "openrouter".to_owned());

    // Provider entry: take from config, or synthesise a bare one so
    // env-only setups (no config file at all) still work.
    let provider = cfg
        .providers
        .get(&provider_name)
        .cloned()
        .unwrap_or_default();

    let base_url = cli
        .base_url
        .clone()
        .or_else(|| std::env::var("MIRA_BASE_URL").ok())
        .or(provider.base_url.clone())
        .or_else(|| default_base_url_for(&provider_name))
        .with_context(|| {
            format!("no base_url for provider `{provider_name}` — set --base-url, MIRA_BASE_URL, or providers.{provider_name}.base_url in config")
        })?;

    let api_key = cli
        .api_key
        .clone()
        .or_else(|| std::env::var("MIRA_API_KEY").ok())
        .or_else(|| ProviderConfig::resolved_api_key(&provider))
        .with_context(|| missing_api_key_hint(&provider_name))?;

    let model = cli
        .model
        .clone()
        .or_else(|| std::env::var("MIRA_MODEL").ok())
        .or_else(|| cfg.default_model.clone())
        .unwrap_or_else(|| "google/gemini-2.5-flash".to_owned());

    let mode_str = cli
        .mode
        .clone()
        .or_else(|| std::env::var("MIRA_MODE").ok())
        .or_else(|| cfg.default_mode.clone())
        .unwrap_or_else(|| "manual".to_owned());
    let mode = parse_mode(&mode_str)?;

    let max_tokens = cli.max_tokens.or(cfg.max_tokens);
    let temperature = cli.temperature.or(cfg.temperature);

    let extra_headers = provider
        .extra_headers
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    let prompt_caching =
        mira_config::prompt_caching_enabled(&provider_name, &base_url, provider.prompt_caching);

    Ok(ResolvedSettings {
        base_url,
        api_key,
        model,
        mode,
        max_tokens,
        temperature,
        extra_headers,
        prompt_caching,
    })
}

fn default_base_url_for(name: &str) -> Option<String> {
    mira_config::default_base_url_for(name).map(|s| s.to_owned())
}

/// Human-friendly "no API key" error tailored to the provider — points at
/// its conventional env var (e.g. `GROQ_API_KEY`) plus the escape hatches
/// (`MIRA_API_KEY`, `mira init`, per-provider yaml). Falls back to a
/// generic hint for providers we don't have a preset env-var name for.
fn missing_api_key_hint(provider: &str) -> String {
    let pretty = mira_config::pretty_provider_name(provider);
    match mira_config::default_api_key_env_for(provider) {
        Some(env) => format!(
            "no API key found for {pretty}. Run `mira init` to set one up globally, \
             or set {env} or MIRA_API_KEY in your shell environment."
        ),
        None => format!(
            "no API key found for {pretty}. Run `mira init` to set one up globally, \
             set MIRA_API_KEY in your shell, or add providers.{provider}.api_key \
             (or api_key_env) to your mira.yaml."
        ),
    }
}

/// Resolve `--resume` into a concrete session record.
///
/// - `None`: no resume requested → fresh session.
/// - `Some("")`: resume most recent session in `cwd`.
/// - `Some(id)`: resume by id.
async fn resume_target(
    flag: Option<&str>,
    store: Option<&dyn SessionStore>,
    cwd: &std::path::Path,
) -> Result<Option<mira_harness::SessionRecord>> {
    let Some(flag) = flag else {
        return Ok(None);
    };
    let Some(store) = store else {
        anyhow::bail!("--resume needs persistence, but --no-persist is set (or no home dir)");
    };
    if flag.is_empty() {
        let recent = store.list_recent(cwd, 1).await?;
        match recent.into_iter().next() {
            Some(r) => Ok(Some(r)),
            None => anyhow::bail!("no saved sessions for `{}`", cwd.display()),
        }
    } else {
        Ok(Some(store.load(&SessionId::from(flag)).await?))
    }
}

fn parse_mode(s: &str) -> Result<Mode> {
    Ok(match s {
        "plan" => Mode::Plan,
        "manual" => Mode::Manual,
        "auto" => Mode::Auto,
        "edit" => Mode::Edit,
        "yolo" => Mode::Yolo,
        other => anyhow::bail!("unknown mode `{other}` (expected plan|manual|auto|edit|yolo)"),
    })
}

fn init_tracing(use_tui: bool) {
    // In TUI mode, stderr would corrupt the alternate screen — sink logs.
    // Users who need them can pass --simple.
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(if use_tui { "off" } else { "mira=info" }));
    let builder = tracing_subscriber::fmt().with_env_filter(filter).compact();
    if use_tui {
        builder.with_writer(std::io::sink).init();
    } else {
        builder.with_writer(std::io::stderr).init();
    }
}

/// CLI-side wrapper: adds "running in a terminal" to the server's shared
/// prompt (which is otherwise identical — enumeration of tools, bash-unlocks
/// hint, cwd). Delegates to `mira_server::system_prompt` so the tool list
/// stays a single source of truth.
fn system_prompt(cwd: &std::path::Path, registry: &Registry) -> String {
    let base = mira_server::system_prompt(cwd, registry);
    // Tack on a terminal-flavored preamble so TUI/REPL responses don't
    // reference "the browser" or "the panel" that only the web UI has.
    base.replacen(
        "You are Mira, an interactive coding agent.",
        "You are Mira, an interactive coding agent running in a terminal.",
        1,
    )
}
