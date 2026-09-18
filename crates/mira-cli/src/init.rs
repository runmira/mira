//! `mira init` — first-run setup.
//!
//! Writes a minimal `mira.yaml` with a provider entry, API key, and
//! default model. Global (`~/.mira/mira.yaml`) by default; `--local`
//! targets `<cwd>/.mira/config.yaml` instead.
//!
//! Fully non-interactive when `--provider`, `--api-key`, and `--model`
//! are all supplied. Missing pieces are prompted on a TTY, and a
//! non-TTY invocation without them fails cleanly rather than hanging.

use std::io::{IsTerminal, Write};
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::Args;
use mira_config::MiraConfig;

/// The presets we offer in the interactive picker. Order matters —
/// this is the display order. Includes every hosted provider the shared
/// `EXPECTED_PROVIDERS` list ships plus the local runtimes at the end.
const PRESET_ORDER: &[&str] = &[
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

/// Suggested default model per provider. Kept modest — a mid-tier fast
/// model most users will recognise. `mira config set default_model …`
/// or `--model` on init overrides.
fn suggested_model(provider: &str) -> &'static str {
    match provider {
        "openrouter" => "anthropic/claude-sonnet-4.5",
        "anthropic" => "claude-sonnet-4-5",
        "openai" => "gpt-5",
        "google" => "gemini-2.5-flash",
        "groq" => "llama-3.3-70b-versatile",
        "cerebras" => "llama-3.3-70b",
        "deepseek" => "deepseek-chat",
        "xai" => "grok-4",
        "together" => "meta-llama/Llama-3.3-70B-Instruct-Turbo",
        "fireworks" => "accounts/fireworks/models/llama-v3p3-70b-instruct",
        "hyperbolic" => "meta-llama/Meta-Llama-3.1-70B-Instruct",
        "novita" => "meta-llama/llama-3.3-70b-instruct",
        "perplexity" => "sonar",
        "mistral" | "mistralai" => "mistral-large-latest",
        "moonshot" | "moonshotai" => "kimi-k2-0905-preview",
        "ollama" => "llama3.3",
        "lmstudio" => "local-model",
        "llamacpp" => "local-model",
        _ => "gpt-5",
    }
}

#[derive(Args, Debug, Clone)]
pub struct InitArgs {
    /// Provider preset (`openrouter`, `anthropic`, `openai`, …). Skips
    /// the picker when set.
    #[arg(long)]
    provider: Option<String>,

    /// API key for the provider. Skips the prompt when set. Pass `-`
    /// to read a single line from stdin.
    #[arg(long)]
    api_key: Option<String>,

    /// Model id to write as `default_model`. Skips the prompt when set.
    #[arg(long)]
    model: Option<String>,

    /// Custom base URL — usually only needed for self-hosted gateways
    /// or provider aliases we don't have a preset for.
    #[arg(long)]
    base_url: Option<String>,

    /// Write to the per-repo file (`<cwd>/.mira/config.yaml`) instead
    /// of the global one (`~/.mira/mira.yaml`).
    #[arg(long)]
    local: bool,

    /// Overwrite an existing file. Without it, `init` refuses to
    /// clobber a config that's already on disk.
    #[arg(long)]
    force: bool,

    /// Accept every default (provider = `openrouter`, model = its
    /// suggested default) without prompting. Requires `--api-key`
    /// unless the API key can be resolved from the env.
    #[arg(long, short = 'y')]
    yes: bool,
}

pub async fn run(_cli: &crate::Cli, args: InitArgs) -> Result<()> {
    let target = target_path(args.local)?;

    if target.exists() && !args.force {
        bail!(
            "{} already exists — pass --force to overwrite (or edit it directly with `mira config edit`)",
            target.display()
        );
    }

    let interactive = std::io::stdin().is_terminal() && !args.yes;

    let provider = match args.provider.clone() {
        Some(p) => p,
        None if !interactive => "openrouter".to_string(),
        None => pick_provider()?,
    };

    let base_url = args
        .base_url
        .clone()
        .or_else(|| mira_config::default_base_url_for(&provider).map(str::to_owned));

    let api_key = resolve_api_key(&args, &provider, interactive)?;

    let model = match args.model.clone() {
        Some(m) => m,
        None if !interactive => suggested_model(&provider).to_string(),
        None => prompt_string(
            &format!("Default model [{}]: ", suggested_model(&provider)),
            Some(suggested_model(&provider)),
        )?,
    };

    let mut cfg = if target.exists() {
        read_existing(&target)?
    } else {
        MiraConfig::default()
    };

    cfg.default_provider = Some(provider.clone());
    cfg.default_model = Some(model.clone());
    let provider_cfg = cfg.providers.entry(provider.clone()).or_default();
    if base_url.is_some() {
        provider_cfg.base_url = base_url;
    }
    // Store the key as a literal only for local runtimes where secrets
    // don't matter. For hosted providers, prefer `api_key_env` — the
    // key stays in the shell env and the yaml records where to find it.
    if let Some(key) = api_key {
        if is_local_runtime(&provider) {
            provider_cfg.api_key = Some(key);
        } else {
            let env_name = mira_config::default_api_key_env_for(&provider)
                .unwrap_or("MIRA_API_KEY")
                .to_string();
            provider_cfg.api_key_env = Some(env_name.clone());
            // Only write the literal key when the env var isn't already
            // populated — otherwise the shell env is enough and we
            // don't want to duplicate the secret on disk.
            if std::env::var_os(&env_name).is_none() {
                provider_cfg.api_key = Some(key);
            }
        }
    }

    let written = write_config(&cfg, &target)?;
    eprintln!("wrote {}", written.display());
    eprintln!("  provider: {}", provider);
    eprintln!("  model:    {}", model);
    Ok(())
}

fn target_path(local: bool) -> Result<PathBuf> {
    if local {
        let cwd = std::env::current_dir().context("read cwd")?;
        Ok(cwd.join(".mira").join("config.yaml"))
    } else {
        Ok(mira_config::global_path())
    }
}

fn read_existing(path: &std::path::Path) -> Result<MiraConfig> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("read existing config {}", path.display()))?;
    serde_yaml::from_str(&raw).with_context(|| format!("parse existing config {}", path.display()))
}

fn write_config(cfg: &MiraConfig, path: &std::path::Path) -> Result<PathBuf> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("mkdir {}", parent.display()))?;
    }
    let yaml = serde_yaml::to_string(cfg).context("serialize config")?;
    std::fs::write(path, yaml).with_context(|| format!("write {}", path.display()))?;
    Ok(path.to_path_buf())
}

fn resolve_api_key(args: &InitArgs, provider: &str, interactive: bool) -> Result<Option<String>> {
    // Explicit flag wins — `-` means "read one line from stdin".
    if let Some(key) = &args.api_key {
        if key == "-" {
            let mut buf = String::new();
            std::io::stdin()
                .read_line(&mut buf)
                .context("read api key from stdin")?;
            let trimmed = buf.trim().to_string();
            if trimmed.is_empty() {
                bail!("stdin was empty; no API key to store");
            }
            return Ok(Some(trimmed));
        }
        return Ok(Some(key.clone()));
    }

    // Local runtimes typically don't need one — offer to skip.
    if is_local_runtime(provider) {
        return Ok(None);
    }

    // Env already populated → nothing to store. init still writes
    // `api_key_env` so the yaml documents where the key comes from.
    if let Some(env_name) = mira_config::default_api_key_env_for(provider) {
        if std::env::var_os(env_name).is_some() {
            eprintln!("using {env_name} from your shell environment");
            return Ok(Some(std::env::var(env_name)?));
        }
    }

    if !interactive {
        bail!(
            "no API key for `{provider}` — pass --api-key or set {} in your shell",
            mira_config::default_api_key_env_for(provider).unwrap_or("MIRA_API_KEY")
        );
    }

    let pretty = mira_config::pretty_provider_name(provider);
    let hint = mira_config::default_api_key_env_for(provider).unwrap_or("MIRA_API_KEY");
    let entered = prompt_string(
        &format!("{pretty} API key (leave blank to store {hint} lookup only): "),
        None,
    )?;
    Ok(if entered.is_empty() {
        None
    } else {
        Some(entered)
    })
}

fn is_local_runtime(name: &str) -> bool {
    matches!(name, "ollama" | "lmstudio" | "llamacpp")
}

fn pick_provider() -> Result<String> {
    eprintln!("Pick a provider:");
    for (i, name) in PRESET_ORDER.iter().enumerate() {
        let pretty = mira_config::pretty_provider_name(name);
        eprintln!("  [{:2}] {}  ({name})", i + 1, pretty);
    }
    loop {
        let raw = prompt_string("Choice [1]: ", Some("1"))?;
        let idx: usize = raw.parse().unwrap_or(0);
        if (1..=PRESET_ORDER.len()).contains(&idx) {
            return Ok(PRESET_ORDER[idx - 1].to_string());
        }
        // Also accept a raw provider name typed into the prompt — makes
        // it easy to pick something not in the preset list.
        if !raw.is_empty() && raw.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Ok(raw);
        }
        eprintln!(
            "  → enter a number 1..{} or a provider name",
            PRESET_ORDER.len()
        );
    }
}

fn prompt_string(prompt: &str, default: Option<&str>) -> Result<String> {
    eprint!("{prompt}");
    std::io::stderr().flush().ok();
    let mut buf = String::new();
    std::io::stdin().read_line(&mut buf).context("read input")?;
    let trimmed = buf.trim().to_string();
    if trimmed.is_empty() {
        if let Some(d) = default {
            return Ok(d.to_string());
        }
    }
    Ok(trimmed)
}
