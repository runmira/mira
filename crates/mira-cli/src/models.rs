//! `mira models` — fetch and print the configured provider's model catalog.
//!
//! Wraps `ChatProvider::list_models()` so the CLI shows exactly what the
//! server-side model picker would offer. Falls back to a plain error
//! message when the provider doesn't expose a catalog (empty list from
//! the default trait impl).

use anyhow::{Context, Result};
use clap::Args;
use mira_ai::build_chat_provider;
use mira_config::MiraConfig;

#[derive(Args, Debug, Clone)]
pub struct ModelsArgs {
    /// Optional substring filter on the model id (case-insensitive).
    #[arg(long, short = 'q')]
    query: Option<String>,

    /// JSON output for scripting.
    #[arg(long)]
    json: bool,
}

pub async fn run(cli: &crate::Cli, args: ModelsArgs) -> Result<()> {
    let cwd = std::env::current_dir().context("read cwd")?;
    let cfg = MiraConfig::load(&cwd).context("load config")?;
    let settings = crate::resolve_settings(cli, &cfg)?;

    let provider = build_chat_provider(
        &settings.provider_name,
        settings.base_url.clone(),
        settings.api_key.clone(),
        settings.extra_headers.clone(),
        settings.prompt_caching,
    )
    .context("build provider")?;

    let mut models = provider
        .list_models()
        .await
        .with_context(|| format!("list models from {}", settings.provider_name))?;

    if let Some(q) = args.query.as_deref() {
        let needle = q.to_ascii_lowercase();
        models.retain(|m| m.id.to_ascii_lowercase().contains(&needle));
    }
    models.sort_by(|a, b| a.id.cmp(&b.id));

    if args.json {
        println!("{}", serde_json::to_string_pretty(&models)?);
        return Ok(());
    }

    if models.is_empty() {
        eprintln!("no models returned");
        return Ok(());
    }

    let id_w = models.iter().map(|m| m.id.len()).max().unwrap_or(0);
    for m in &models {
        let ctx = m
            .context_length
            .map(|c| format!("{c}"))
            .unwrap_or_else(|| "-".into());
        let owner = m.owned_by.clone().unwrap_or_default();
        println!("  {:<id_w$}  {:>10}  {}", m.id, ctx, owner, id_w = id_w);
    }
    Ok(())
}
