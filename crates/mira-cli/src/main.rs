mod approver;
mod repl;
mod tui;

use std::io::IsTerminal;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use mira_ai::openai::{OpenAiCompatible, OpenAiConfig};
use mira_harness::{Approver, Session, SessionConfig};
use mira_policy::{Mode, Policy, PolicyConfig};
use mira_sandbox::Sandbox;
use mira_tools::{builtin, Registry, ToolContext};
use tokio::sync::Mutex;
use tracing_subscriber::EnvFilter;

/// Mira — an open-source coding agent.
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Cli {
    /// Base URL of an OpenAI-compatible endpoint.
    #[arg(
        long,
        env = "MIRA_BASE_URL",
        default_value = "https://openrouter.ai/api/v1"
    )]
    base_url: String,

    /// API key for the endpoint.
    #[arg(long, env = "MIRA_API_KEY")]
    api_key: String,

    /// Model ID.
    #[arg(long, env = "MIRA_MODEL", default_value = "google/gemini-2.5-flash")]
    model: String,

    /// Permission mode: plan|manual|auto|edit|yolo.
    #[arg(long, default_value_t = String::from("manual"))]
    mode: String,

    /// Cap on tokens the model may produce per turn. Skip to use the model default.
    #[arg(long)]
    max_tokens: Option<u32>,

    /// Sampling temperature. Skip to use the model default.
    #[arg(long)]
    temperature: Option<f32>,

    /// Force the simple line-based REPL even when running in a TTY.
    /// Piped input (e.g. `echo foo | mira`) always uses the REPL.
    #[arg(long)]
    simple: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let use_tui = !cli.simple && std::io::stdin().is_terminal() && std::io::stdout().is_terminal();

    init_tracing(use_tui);

    let cwd = std::env::current_dir().context("failed to read cwd")?;

    // --- provider
    let provider = Arc::new(
        OpenAiCompatible::new(OpenAiConfig {
            base_url: cli.base_url,
            api_key: cli.api_key,
            extra_headers: Vec::new(),
        })
        .context("build provider")?,
    );

    // --- tools + sandbox
    let sandbox = Arc::new(Sandbox::default_scrubbed());
    let mut registry = Registry::new();
    builtin::register_default(&mut registry);
    let registry = Arc::new(registry);
    let tool_ctx = ToolContext::new(cwd.clone(), sandbox);

    // --- policy
    let mode = parse_mode(&cli.mode)?;
    let policy = Policy::from_config(&PolicyConfig {
        mode,
        ..Default::default()
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

    // --- session
    let mut sess_cfg = SessionConfig::new(cli.model.clone());
    sess_cfg.max_tokens = cli.max_tokens;
    sess_cfg.temperature = cli.temperature;
    let session = Session::new(
        sess_cfg,
        system_prompt(&cwd),
        provider,
        registry,
        policy.clone(),
        approver,
        tool_ctx,
    );

    if use_tui {
        tui::run(
            session,
            tui::TuiConfig {
                model: cli.model,
                mode,
                policy,
                approval_rx: approval_rx.expect("tui branch created a receiver"),
            },
        )
        .await
    } else {
        repl::run(session).await
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

fn system_prompt(cwd: &std::path::Path) -> String {
    format!(
        "You are Mira, an interactive coding agent running in a terminal.\n\
         Working directory: {}\n\n\
         Prefer tool use over guessing. Read files before editing them; use `edit_file` \
         with enough context in `old_string` to disambiguate. When running commands, \
         keep them small and explain what you're doing.",
        cwd.display()
    )
}
