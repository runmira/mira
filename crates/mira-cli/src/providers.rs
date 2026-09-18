//! `mira providers` — list every provider preset we know about.
//!
//! Renders one row per preset: slug, pretty name, default base URL,
//! conventional env var, and whether the current config has an entry
//! for it. Useful before `mira init` to see the shortlist, and after
//! setup to see what's actually wired.

use anyhow::{Context, Result};
use clap::Args;
use mira_config::MiraConfig;

const ROSTER: &[&str] = &[
    "openrouter",
    "anthropic",
    "openai",
    "google",
    "groq",
    "cerebras",
    "deepseek",
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

#[derive(Args, Debug, Clone)]
pub struct ProvidersArgs {
    /// JSON output for scripting.
    #[arg(long)]
    json: bool,
}

pub async fn run(_cli: &crate::Cli, args: ProvidersArgs) -> Result<()> {
    let cwd = std::env::current_dir().context("read cwd")?;
    let cfg = MiraConfig::load(&cwd).unwrap_or_default();

    if args.json {
        let rows: Vec<serde_json::Value> = ROSTER
            .iter()
            .map(|name| {
                let entry = cfg.providers.get(*name);
                serde_json::json!({
                    "name": name,
                    "display_name": mira_config::pretty_provider_name(name),
                    "base_url": entry
                        .and_then(|p| p.base_url.clone())
                        .or_else(|| mira_config::default_base_url_for(name).map(str::to_owned)),
                    "api_key_env": entry
                        .and_then(|p| p.api_key_env.clone())
                        .or_else(|| mira_config::default_api_key_env_for(name).map(str::to_owned)),
                    "configured": entry.is_some(),
                    "is_default": cfg.default_provider.as_deref() == Some(*name),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }

    let name_w = ROSTER.iter().map(|s| s.len()).max().unwrap_or(0);
    let pretty_w = ROSTER
        .iter()
        .map(|s| mira_config::pretty_provider_name(s).len())
        .max()
        .unwrap_or(0);

    println!(
        "  {:<name_w$}  {:<pretty_w$}  {:<40}  {:<20}  {}",
        "name",
        "display",
        "base_url",
        "api_key_env",
        "state",
        name_w = name_w,
        pretty_w = pretty_w,
    );
    for name in ROSTER {
        let entry = cfg.providers.get(*name);
        let base = entry
            .and_then(|p| p.base_url.clone())
            .or_else(|| mira_config::default_base_url_for(name).map(str::to_owned))
            .unwrap_or_default();
        let env = entry
            .and_then(|p| p.api_key_env.clone())
            .or_else(|| mira_config::default_api_key_env_for(name).map(str::to_owned))
            .unwrap_or_default();

        let default_marker = cfg.default_provider.as_deref() == Some(*name);
        let configured = entry.is_some();
        let state = match (configured, default_marker) {
            (true, true) => "default",
            (true, false) => "configured",
            (false, _) => "-",
        };

        let leader = if default_marker { "* " } else { "  " };
        println!(
            "{leader}{:<name_w$}  {:<pretty_w$}  {:<40}  {:<20}  {}",
            name,
            mira_config::pretty_provider_name(name),
            base,
            env,
            state,
            name_w = name_w,
            pretty_w = pretty_w,
        );
    }
    Ok(())
}
