mod approver;
mod repl;

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use mira_ai::openai::{OpenAiCompatible, OpenAiConfig};
use mira_harness::{Session, SessionConfig};
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
    #[arg(long, env = "MIRA_MODEL", default_value = "anthropic/claude-sonnet-5")]
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
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "mira=info".into()))
        .with_writer(std::io::stderr)
        .compact()
        .init();

    let cli = Cli::parse();
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
    let mode: Mode = match cli.mode.as_str() {
        "plan" => Mode::Plan,
        "manual" => Mode::Manual,
        "auto" => Mode::Auto,
        "edit" => Mode::Edit,
        "yolo" => Mode::Yolo,
        other => anyhow::bail!("unknown mode `{other}` (expected plan|manual|auto|edit|yolo)"),
    };
    let policy = Policy::from_config(&PolicyConfig {
        mode,
        ..Default::default()
    })
    .context("compile policy")?;
    let policy = Arc::new(Mutex::new(policy));

    // --- approver
    let approver = Arc::new(approver::TerminalApprover);

    // --- session
    let mut sess_cfg = SessionConfig::new(cli.model);
    sess_cfg.max_tokens = cli.max_tokens;
    sess_cfg.temperature = cli.temperature;
    let session = Session::new(
        sess_cfg,
        system_prompt(&cwd),
        provider,
        registry,
        policy,
        approver,
        tool_ctx,
    );

    repl::run(session).await
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
