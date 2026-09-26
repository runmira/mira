mod acp;
mod approver;
mod cloud;
mod config;
mod config_cmd;
mod doctor;
mod eval;
mod ext_cmd;
mod github;
mod goal;
mod headless;
mod init;
mod login;
mod memory;
mod models;
mod permissions;
mod providers;
mod repl;
mod review;
mod sandbox;
mod serve;
mod slack;
mod tui;

// Entry point for the mira CLI binary.

use std::io::IsTerminal;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use mira_ai::build_chat_provider;
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

    /// Run one task without a UI and exit: `mira -p "fix the failing
    /// test"`. Piped stdin is added as context (`cat log | mira -p
    /// "why?"`), or is the whole prompt when `-p` has no text. Tools that
    /// would ask you are denied; widen with `--mode` or `--allow`.
    #[arg(short = 'p', long = "print", value_name = "PROMPT", num_args = 0..=1, default_missing_value = "")]
    print: Option<String>,

    /// Output for `-p`: `text` (the answer), `json` (one result object)
    /// or `stream-json` (one JSON event per line).
    #[arg(long, value_enum, default_value_t = headless::OutputFormat::Text, requires = "print")]
    output_format: headless::OutputFormat,

    /// Allow a tool without asking, e.g. `--allow "Bash(cargo test*)"`.
    /// Same rules as `permissions.allow` in mira.yaml. Repeatable.
    #[arg(long = "allow", value_name = "RULE")]
    allow: Vec<String>,

    /// Stop after this many model rounds.
    #[arg(long, value_name = "N")]
    max_turns: Option<usize>,

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

    /// Show an interactive picker of recent sessions for the current
    /// folder before launching. Pick fresh with `0` / Enter / Esc.
    #[arg(long, short = 'P')]
    pick: bool,

    /// Skip persistence entirely — sessions are not saved to disk.
    #[arg(long)]
    no_persist: bool,

    /// Enable the `computer` tool for this run: screenshots plus mouse and
    /// keyboard control of your desktop. Every action asks first unless
    /// a `Computer(...)` allow rule covers it. Same as `computer.enabled`
    /// in ~/.mira/mira.yaml.
    #[arg(long, global = true)]
    computer: bool,

    /// Enable the `browser` tool for this run: drive Chrome/Chromium in
    /// a separate Mira profile. Same as `browser.enabled` in mira.yaml.
    #[arg(long, global = true)]
    browser: bool,

    /// Start in a remote environment instead of on your worktree: a name
    /// from `compute.environments`, or the built-ins `scratch` (a copy on
    /// this machine) and `e2b` (a cloud sandbox; needs E2B_API_KEY).
    /// `/remote-env` switches later. Defaults to `compute.default`.
    #[arg(long, global = true, value_name = "BACKEND")]
    sandbox: Option<String>,

    #[command(subcommand)]
    command: Option<Command>,
}

/// Subcommands. Absent = the default chat entrypoint (TUI or REPL).
#[derive(Subcommand, Debug, Clone)]
enum Command {
    /// First-run setup: write a `mira.yaml` with a provider, API key,
    /// and default model. Interactive on a TTY; flag-driven otherwise.
    Init(init::InitArgs),
    /// Diagnose the local install — config, provider, dirs, skills,
    /// MCP servers, memory. `--ping` also probes the provider.
    Doctor(doctor::DoctorArgs),
    /// Inspect or edit `mira.yaml` (path, show, edit, get, set).
    Config(config_cmd::ConfigArgs),
    /// List every provider preset and whether it's configured.
    Providers(providers::ProvidersArgs),
    /// List models offered by the currently configured provider.
    Models(models::ModelsArgs),
    /// List or edit permission rules (allow/ask/deny) in `mira.yaml`.
    Permissions(permissions::PermissionsArgs),
    /// Run Mira as a local web server. Binds to 127.0.0.1 by default; a
    /// browser (or, later, the desktop app) is the frontend.
    Serve(serve::ServeArgs),
    /// Two-stage diff review: generate findings, then hostile re-verify.
    Review(review::ReviewArgs),
    /// Batch-run regression eval tasks and print a summary.
    Eval(eval::EvalArgs),
    /// Manage cross-session memory (MIRA.md + episodic).
    Memory(memory::MemoryArgs),
    /// Set, clear, inspect, or resume the standing `/goal` on the
    /// most recent session for the current folder.
    Goal(goal::GoalArgs),
    /// Run a task in a cloud sandbox and get a pull request back. You can
    /// close your laptop once it's started.
    Cloud(cloud::CloudArgs),
    /// Manage MCP servers: add, list, sign in, enable/disable, approve.
    Mcp(ext_cmd::McpArgs),
    /// Manage plugins and plugin marketplaces (Claude Code format).
    Plugin(ext_cmd::PluginArgs),
    /// Sign in with a provider via browser OAuth (openrouter, openai).
    Login(login::LoginArgs),
    /// Forget a provider's credentials from mira.yaml + auth store.
    Logout(login::LogoutArgs),
    /// Inspect sign-in status or force-refresh a token bundle.
    Auth(login::AuthArgs),
    /// Run as an Agent Client Protocol agent over stdio (for Zed and
    /// other ACP editors).
    Acp(acp::AcpArgs),
    /// Respond to a GitHub Actions event: review pull requests, and work
    /// on `@runmira-bot` requests in issues and PRs. Used by the Mira action.
    Github(github::GithubArgs),
    /// Run Mira as a Slack bot (Socket Mode): mention it or DM it, and it
    /// works in this folder, one session per thread.
    Slack(slack::SlackArgs),
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Subcommand branch — every non-default command short-circuits
    // before we build the session, provider, and tools.
    if let Some(cmd) = cli.command.clone() {
        init_tracing(false, false);
        return match cmd {
            Command::Init(args) => init::run(&cli, args).await,
            Command::Doctor(args) => doctor::run(&cli, args).await,
            Command::Config(args) => config_cmd::run(&cli, args).await,
            Command::Providers(args) => providers::run(&cli, args).await,
            Command::Models(args) => models::run(&cli, args).await,
            Command::Permissions(args) => permissions::run(&cli, args).await,
            Command::Serve(args) => serve::run(&cli, args).await,
            Command::Review(args) => review::run(&cli, args).await,
            Command::Eval(args) => eval::run(&cli, args).await,
            Command::Memory(args) => memory::run(&cli, args).await,
            Command::Goal(args) => goal::run(&cli, args).await,
            Command::Cloud(args) => cloud::run(&cli, args).await,
            Command::Mcp(args) => ext_cmd::run_mcp(args).await,
            Command::Plugin(args) => ext_cmd::run_plugin(args).await,
            Command::Login(args) => login::run_login(args).await,
            Command::Logout(args) => login::run_logout(args).await,
            Command::Auth(args) => login::run_auth(args).await,
            Command::Acp(args) => acp::run(&cli, args).await,
            Command::Github(args) => github::run(&cli, args).await,
            Command::Slack(args) => slack::run(&cli, args).await,
        };
    }

    let headless = cli.print.is_some();
    let use_tui = !headless
        && !cli.simple
        && std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal();

    init_tracing(use_tui, headless);

    // `-p`: read the task up front, so a missing one fails fast.
    let headless_prompt = match &cli.print {
        None => None,
        Some(arg) => {
            let stdin = if std::io::stdin().is_terminal() {
                None
            } else {
                // With `-p` text, piped input is optional context.
                let grace =
                    (!arg.trim().is_empty()).then_some(std::time::Duration::from_millis(200));
                headless::read_stdin(std::io::stdin(), grace)
            };
            match headless::prompt(arg, stdin) {
                Some(p) => Some(p),
                None => {
                    eprintln!("mira: -p needs a task, e.g. mira -p \"explain this repo\"");
                    std::process::exit(2);
                }
            }
        }
    };

    let cwd = std::env::current_dir().context("failed to read cwd")?;
    // If an OAuth-managed provider (openai) is close to expiry, refresh
    // the bundle synchronously before we load config — otherwise the
    // stale api_key in yaml would flow into the provider build below
    // and the first chat turn would 401. Fast no-op when no bundle is
    // stored or the current key is still fresh.
    login::auto_refresh_if_needed().await;
    let cfg = MiraConfig::load(&cwd).context("load config")?;
    // Materialise `keys:` into the process env so tools like web_search
    // read them via their existing env-var conventions without extra
    // plumbing. Yaml never overwrites a shell-set env value.
    mira_config::export_keys_to_env(&cfg);

    // --- resolve settings across CLI / env / config / defaults
    let settings = resolve_settings(&cli, &cfg)?;

    // --- provider
    let provider = build_chat_provider(
        &settings.provider_name,
        settings.base_url,
        settings.api_key,
        settings.extra_headers,
        settings.prompt_caching,
    )
    .context("build provider")?;

    // --- tools + sandbox
    let sandbox = Arc::new(Sandbox::default_scrubbed());
    let mut registry = Registry::new();
    builtin::register_core(&mut registry);
    if cfg.memory.tools_enabled() {
        builtin::register_memory(&mut registry);
    }
    // Skills: load bundled + user (~/.mira/skills) + project (<cwd>/.mira/skills).
    // Wrap in the RwLock-Arc shape the SkillTool expects — no cwd swap in
    // the CLI path, so the lock is effectively read-only, but the shape
    // stays consistent with `mira serve`.
    // MCP servers, plugins and custom commands. Plugin skills join the
    // skill registry; MCP servers connect in the background.
    let extensions = mira_server::extensions::Extensions::new(Some(cwd.clone()));
    let skills_registry =
        mira_server::extensions::load_skills(Some(&cwd), &extensions.plugin_skill_dirs());
    let skills_handle: mira_tools::builtin::skill::SkillHandle = std::sync::Arc::new(
        tokio::sync::RwLock::new(std::sync::Arc::new(skills_registry)),
    );
    builtin::register_skills(&mut registry, skills_handle.clone());
    extensions.attach_skills(skills_handle.clone());
    extensions.reload().await;
    // `memory_consolidate` — dedup/merge a MIRA.md via a cheap model.
    // Gated behind the same `memory.tools_enabled` switch as the other
    // memory tools: consolidation isn't useful without them.
    if cfg.memory.tools_enabled() {
        let consolidate_model = cfg
            .memory
            .extractor_model()
            .map(str::to_owned)
            .or_else(|| settings.small_model.clone())
            .unwrap_or_else(|| settings.model.clone());
        builtin::register_consolidate(&mut registry, provider.clone(), consolidate_model);
    }
    // MCP tools appear live as their servers connect (and leave if one
    // drops). A broken server never stops Mira from starting.
    registry.add_source(extensions.mcp().tool_source());
    register_computer_use(&mut registry, &cli, &cfg).await;
    // Remote environments: the manager owns the switchable compute slot
    // the tools read. `--sandbox` / `compute.default` switch it once the
    // session exists; `/remote-env` switches it later. While remote, the
    // harness hides tools that would touch this machine.
    let environments = Arc::new(mira_compute::EnvironmentManager::new(
        cwd.clone(),
        mira_compute::EnvironmentManager::default_patch_dir(),
        cfg.compute.clone(),
    ));
    // Snapshot the base registry BEFORE the interactive/agent tools
    // land — this is what child sessions inherit when the `agent` tool
    // spawns a subagent. Keeping `agent` OUT of the base prevents an
    // unbounded recursion of "agent spawns agent spawns agent"; the
    // AgentTool re-adds itself into child registries with proper depth
    // controls when it wants nested spawning.
    let base_registry_for_subagents = Arc::new(registry.clone());

    // Interactive prompt tools (`plan`, `ask_user`) ride an mpsc pair
    // into the TUI event loop — same wire pattern as the approver. The
    // receiver travels to `tui::run` via TuiConfig; without a TUI
    // (--simple / non-terminal stdin) the channel just has no reader,
    // and any tool call degrades to a graceful "cancelled" result.
    let (prompt_channel, prompt_rx) = tui::TuiPromptChannel::new();
    registry.register(mira_tools::prompt::PlanTool::new(prompt_channel.clone()));
    registry.register(mira_tools::prompt::AskUserTool::new(prompt_channel));

    // Agent tool + subagent event broadcast. AgentTool reuses the
    // server crate's implementation (it's mature and already wires
    // ScratchpadTool, worktrees, and reviewer flow); we drive its
    // `Subagent*` frames into the TUI via a broadcast channel so the
    // nested cell can render each child's live tool uses. The receiver
    // travels to `tui::run` via TuiConfig; no TUI means no subscriber,
    // and AgentTool silently drops the events (never blocks).
    let (subagent_events_tx, subagent_events_rx) =
        tokio::sync::broadcast::channel::<mira_server::protocol::ServerMsg>(256);
    let agents_registry = Arc::new(mira_agents::load_with_plugins(
        &cwd,
        &extensions.plugin_agent_files(),
    ));
    tracing::info!(
        count = agents_registry.names().len(),
        types = ?agents_registry.names(),
        "mira-cli: loaded agent types"
    );
    // agent_tool registration is deferred until after `store` is
    // created (below) so we can wire .with_store() in one step.
    let agent_tool_builder = mira_server::interactive::AgentTool::new(
        provider.clone(),
        base_registry_for_subagents.clone(),
        settings.model.clone(),
    )
    .with_agents(agents_registry.clone())
    .with_small_model(settings.small_model.clone())
    .with_events_tx(subagent_events_tx.clone());

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
    let tool_ctx = tool_ctx.with_compute_slot(environments.slot());

    // --- policy: rules from config, mode from CLI/config/default
    let policy = Policy::from_config(&PolicyConfig {
        mode: settings.mode,
        allow: cfg
            .permissions
            .allow
            .iter()
            .chain(&cli.allow)
            .cloned()
            .collect(),
        ask: cfg.permissions.ask.clone(),
        deny: cfg.permissions.deny.clone(),
    })
    .context("compile policy")?;
    let policy = Arc::new(Mutex::new(policy));

    // --- approver (branch on frontend)
    let headless_approver = Arc::new(headless::HeadlessApprover::default());
    let (approver, approval_rx) = if headless {
        (headless_approver.clone() as Arc<dyn Approver>, None)
    } else if use_tui {
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

    // Register the agent tool now that `store` is available.
    let mut agent_tool = agent_tool_builder;
    if let Some(s) = &store {
        agent_tool = agent_tool.with_store(s.clone());
    }
    registry.register(agent_tool);

    // --- session (fresh or resumed)
    let mut sess_cfg = SessionConfig::new(settings.model.clone());
    sess_cfg.max_tokens = settings.max_tokens;
    sess_cfg.temperature = settings.temperature;
    sess_cfg.compactor_model = settings.compactor_model.clone();
    sess_cfg.small_model = settings.small_model.clone();
    if let Some(n) = cli.max_turns {
        sess_cfg.max_rounds = n.max(1);
    }

    // --pick short-circuits --resume: show a picker, and use the chosen
    // record as the resume target. Cancelling drops through to a fresh
    // session.
    let picked = if cli.pick {
        match store.as_deref() {
            Some(s) => pick_session(s, &cwd).await?,
            None => {
                eprintln!("--pick needs persistence, but --no-persist is set (or no home dir)");
                None
            }
        }
    } else {
        resume_target(cli.resume.as_deref(), store.as_deref(), &cwd).await?
    };

    let tool_names: Vec<String> = registry.specs().into_iter().map(|t| t.name).collect();
    let session = match picked {
        Some(record) => Session::resume_from(
            record,
            provider.clone(),
            Arc::new(registry),
            policy.clone(),
            approver,
            tool_ctx,
        ),
        None => Session::new(
            sess_cfg,
            system_prompt(&cwd, &registry),
            provider.clone(),
            Arc::new(registry),
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
    // Plugins' and the user's hooks (PreToolUse, Stop, …).
    let session = session.with_hooks(extensions.hook_runner());
    // Live memory: reload user + project MIRA.md on every provider round,
    // plus tail the most-recent episodic entries so cross-session memory
    // is visible immediately after `memory_remember` writes it.
    // `memory.inject_context: false` skips the snapshot wiring entirely,
    // so the model sees exactly the same prompt as before the memory
    // work landed — a clean bisect switch.
    let mut session = session;
    if cfg.memory.inject_context() {
        session = session
            .with_memory_snapshot(mira_server::make_memory_snapshot_with(&cwd, episodic_store));
        session = session.with_memory_retrieval(mira_server::memory_retrieval_from(&cfg.memory));
    }
    if cfg.memory.auto_extract_enabled() {
        session = session.with_auto_extract(mira_harness::AutoExtractConfig::enabled(
            cfg.memory.extractor_model().map(str::to_owned),
        ));
    }
    let session = session;

    let (env_tx, env_rx) = tui::env_channel();
    if let Some(target) = sandbox::startup_target(cli.sandbox.as_deref(), &cfg.compute) {
        let progress: mira_compute::env::Progress =
            Arc::new(|m: String| eprintln!("remote environment: {m}"));
        let report = environments
            .switch(&target, progress)
            .await
            .with_context(|| format!("starting remote environment `{target}`"))?;
        session.push_note(report.model_note.clone()).await;
        eprintln!(
            "remote environment: ready. Tools run in `{target}`; /remote-env local brings the \
             changes back to your worktree."
        );
    }

    let mut exit_code = 0;
    let result: Result<()> = async {
        if let Some(prompt) = &headless_prompt {
            extensions
                .mcp()
                .wait_settled(extensions.mcp().options().connect_timeout)
                .await;
            exit_code = headless::run(
                headless::Run {
                    session,
                    approver: headless_approver.clone(),
                    format: cli.output_format,
                    model: &settings.model,
                    cwd: &cwd,
                    tools: tool_names,
                },
                prompt,
            )
            .await?;
            Ok(())
        } else if use_tui {
            // (#1) Fire-and-forget model catalog fetch — populates the
            // `/model <TAB>` autocomplete without blocking startup. A slow
            // provider (or an offline one) just means the palette shows no
            // completions until the fetch lands. Uses the same
            // ChatProvider handle that just built the session.
            let models = std::sync::Arc::new(tokio::sync::RwLock::new(Vec::<String>::new()));
            {
                let models = models.clone();
                let provider_for_models = provider.clone();
                tokio::spawn(async move {
                    match provider_for_models.list_models().await {
                        Ok(list) => {
                            let mut ids: Vec<String> = list.into_iter().map(|m| m.id).collect();
                            ids.sort();
                            ids.dedup();
                            *models.write().await = ids;
                        }
                        Err(e) => tracing::debug!(
                            %e,
                            "tui: model list fetch failed; /model autocomplete will stay empty"
                        ),
                    }
                });
            }

            tui::run(
                session,
                tui::TuiConfig {
                    model: settings.model,
                    provider: settings.provider_name.clone(),
                    mode: settings.mode,
                    policy,
                    approval_rx: approval_rx.expect("tui branch created a receiver"),
                    prompt_rx,
                    subagent_events_rx,
                    cwd: cwd.clone(),
                    skills: skills_handle,
                    store: store.clone(),
                    models,
                    computer_cfg: cfg.computer.clone(),
                    browser_cfg: cfg.browser.clone(),
                    environments: environments.clone(),
                    env_tx,
                    env_rx,
                    extensions: extensions.clone(),
                },
            )
            .await
        } else {
            // No live UI to show tools arriving: wait for the servers.
            extensions
                .mcp()
                .wait_settled(extensions.mcp().options().connect_timeout)
                .await;
            for notice in extensions.notices() {
                eprintln!("mcp: {notice}");
            }
            repl::run(session, skills_handle).await
        }
    }
    .await;
    // Save (never apply) a remote environment's pending changes, even
    // when the frontend errored, and release every environment.
    sandbox::print_finish(environments.finish().await, &cwd);
    extensions.mcp().shutdown();
    if result.is_ok() && exit_code != 0 {
        std::process::exit(exit_code);
    }
    result
}

/// Values that survive the CLI/env/config/default cascade and get passed
/// down to the provider, policy, and session.
pub(crate) struct ResolvedSettings {
    /// Provider name (`"openrouter"`, `"anthropic"`, …). Used by the
    /// provider factory to decide between the OpenAI-compat adapter and
    /// a native one (Anthropic Messages).
    pub(crate) provider_name: String,
    pub(crate) base_url: String,
    pub(crate) api_key: String,
    pub(crate) model: String,
    pub(crate) mode: Mode,
    pub(crate) max_tokens: Option<u32>,
    pub(crate) temperature: Option<f32>,
    pub(crate) compactor_model: Option<String>,
    /// Cheap model for background work (`small_model` in mira.yaml).
    pub(crate) small_model: Option<String>,
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
        .or_else(|| ProviderConfig::resolved_api_key(&provider));
    // Bedrock can sign with AWS credentials instead of an API key.
    let api_key = if provider_name == "bedrock" {
        api_key.unwrap_or_default()
    } else {
        api_key.with_context(|| missing_api_key_hint(&provider_name))?
    };

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
    let compactor_model = cfg.compactor_model.clone();
    let small_model = std::env::var("MIRA_SMALL_MODEL")
        .ok()
        .or_else(|| cfg.small_model.clone())
        .filter(|m| !m.trim().is_empty());

    let extra_headers = provider
        .extra_headers
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    let prompt_caching =
        mira_config::prompt_caching_enabled(&provider_name, &base_url, provider.prompt_caching);

    Ok(ResolvedSettings {
        provider_name,
        base_url,
        api_key,
        model,
        mode,
        max_tokens,
        temperature,
        compactor_model,
        small_model,
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

/// Interactive picker for `--pick`. Lists up to 20 recent sessions in
/// the current folder, prints a numbered menu, and reads a line from
/// stdin. `0`, empty, or non-numeric input starts fresh. Runs before
/// the TUI takes over the terminal, so plain stdout/stdin is fine.
async fn pick_session(
    store: &dyn SessionStore,
    cwd: &std::path::Path,
) -> Result<Option<mira_harness::SessionRecord>> {
    use std::io::{IsTerminal, Write};

    let recent = store.list_recent(cwd, 20).await?;
    if recent.is_empty() {
        eprintln!(
            "no saved sessions for `{}` — starting fresh.",
            cwd.display()
        );
        return Ok(None);
    }
    if !std::io::stdin().is_terminal() {
        // No TTY → can't prompt; refuse and let the caller fall through.
        eprintln!("--pick needs a terminal; ignoring and starting fresh.");
        return Ok(None);
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    eprintln!("recent sessions in {}:", cwd.display());
    for (i, rec) in recent.iter().enumerate() {
        let title = rec
            .title
            .clone()
            .or_else(|| first_user_message(rec))
            .unwrap_or_else(|| "(no messages yet)".into());
        let title = title.trim().replace('\n', " ");
        let short = if title.chars().count() > 60 {
            let head: String = title.chars().take(60).collect();
            format!("{head}…")
        } else {
            title
        };
        let msgs = rec
            .messages
            .iter()
            .filter(|m| !matches!(m.role, mira_core::Role::System))
            .count();
        let ago = human_ago(now.saturating_sub(rec.updated_at / 1000));
        eprintln!("  [{:2}] {}  · {} msg · {}", i + 1, short, msgs, ago);
    }
    eprint!("pick (1-{} · 0 or empty = fresh): ", recent.len());
    std::io::stderr().flush().ok();

    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .context("read picker input")?;
    let choice: usize = line.trim().parse().unwrap_or(0);
    if choice == 0 || choice > recent.len() {
        return Ok(None);
    }
    Ok(recent.into_iter().nth(choice - 1))
}

fn first_user_message(rec: &mira_harness::SessionRecord) -> Option<String> {
    rec.messages
        .iter()
        .find(|m| matches!(m.role, mira_core::Role::User))
        .and_then(|m| m.content.clone())
}

fn human_ago(secs: u64) -> String {
    const MIN: u64 = 60;
    const HOUR: u64 = 60 * MIN;
    const DAY: u64 = 24 * HOUR;
    if secs < MIN {
        format!("{secs}s ago")
    } else if secs < HOUR {
        format!("{}m ago", secs / MIN)
    } else if secs < DAY {
        format!("{}h ago", secs / HOUR)
    } else {
        format!("{}d ago", secs / DAY)
    }
}

pub(crate) fn parse_mode(s: &str) -> Result<Mode> {
    Ok(match s {
        "plan" => Mode::Plan,
        "manual" => Mode::Manual,
        "auto" => Mode::Auto,
        "edit" => Mode::Edit,
        "yolo" => Mode::Yolo,
        other => anyhow::bail!("unknown mode `{other}` (expected plan|manual|auto|edit|yolo)"),
    })
}

fn init_tracing(use_tui: bool, quiet: bool) {
    // In TUI mode, stderr would corrupt the alternate screen — sink logs.
    // Users who need them can pass --simple. `-p` keeps stderr for
    // warnings only, so scripts see just what matters.
    let default = match (use_tui, quiet) {
        (true, _) => "off",
        (false, true) => "mira=warn",
        (false, false) => "mira=info",
    };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default));
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

/// Register `computer` / `browser` when enabled by flag or global config,
/// printing what happened. Shared by the TUI/REPL and `mira serve` boots.
pub(crate) async fn register_computer_use(registry: &mut Registry, cli: &Cli, cfg: &MiraConfig) {
    let enable_computer = cli.computer || cfg.computer.enabled();
    let enable_browser = cli.browser || cfg.browser.enabled();
    if !enable_computer && !enable_browser {
        return;
    }
    let report = builtin::register_computer_use(
        registry,
        enable_computer,
        &cfg.computer,
        enable_browser,
        &cfg.browser,
    )
    .await;
    for w in &report.warnings {
        eprintln!("warning: {w}");
    }
    if let Some(backend) = report.computer {
        eprintln!("computer use enabled ({backend}); every desktop action asks for approval");
    }
}
