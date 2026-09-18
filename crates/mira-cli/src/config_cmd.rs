//! `mira config` — inspect and edit `mira.yaml` without opening it by hand.
//!
//! Supported operations: `path`, `show`, `edit`, `get <key>`, `set <key>
//! <value>`. Keys use dotted notation and target the whole-file scalars
//! (`default_provider`, `default_model`, …) plus per-provider fields
//! (`providers.<name>.base_url`, `.api_key`, `.api_key_env`,
//! `.prompt_caching`). Everything else lives in the yaml only — `edit`
//! opens it in `$EDITOR` for those.

use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use mira_config::{MiraConfig, ProviderConfig};

#[derive(Args, Debug, Clone)]
pub struct ConfigArgs {
    #[command(subcommand)]
    action: ConfigAction,
}

#[derive(Subcommand, Debug, Clone)]
enum ConfigAction {
    /// Print the path of the config file this command would touch.
    Path {
        /// Target the per-repo config (`<cwd>/.mira/config.yaml`).
        #[arg(long)]
        local: bool,
    },
    /// Print the current file contents (or a merged view with `--merged`).
    Show {
        #[arg(long)]
        local: bool,
        /// Print the merged view (global + per-repo) instead of just
        /// the target file.
        #[arg(long)]
        merged: bool,
    },
    /// Open the config file in `$EDITOR` (falls back to `vi`).
    Edit {
        #[arg(long)]
        local: bool,
    },
    /// Read a single field by dotted key. Prints empty for unset fields.
    Get {
        key: String,
        #[arg(long)]
        local: bool,
    },
    /// Write a single field by dotted key. Creates parent tables as
    /// needed and writes the file back atomically.
    Set {
        key: String,
        value: String,
        #[arg(long)]
        local: bool,
    },
}

pub async fn run(_cli: &crate::Cli, args: ConfigArgs) -> Result<()> {
    match args.action {
        ConfigAction::Path { local } => {
            let p = target_path(local)?;
            println!("{}", p.display());
            Ok(())
        }
        ConfigAction::Show { local, merged } => show(local, merged),
        ConfigAction::Edit { local } => edit(local),
        ConfigAction::Get { key, local } => get(&key, local),
        ConfigAction::Set { key, value, local } => set(&key, &value, local),
    }
}

fn target_path(local: bool) -> Result<PathBuf> {
    if local {
        let cwd = std::env::current_dir().context("read cwd")?;
        Ok(cwd.join(".mira").join("config.yaml"))
    } else {
        Ok(mira_config::global_path())
    }
}

fn show(local: bool, merged: bool) -> Result<()> {
    if merged {
        let cwd = std::env::current_dir().context("read cwd")?;
        let cfg = MiraConfig::load(&cwd).context("load config")?;
        let yaml = serde_yaml::to_string(&cfg).context("serialize config")?;
        print!("{yaml}");
        return Ok(());
    }
    let p = target_path(local)?;
    if !p.exists() {
        eprintln!(
            "{} does not exist yet — run `mira init` to create it",
            p.display()
        );
        return Ok(());
    }
    let raw = std::fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?;
    print!("{raw}");
    Ok(())
}

fn edit(local: bool) -> Result<()> {
    let p = target_path(local)?;
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("mkdir {}", parent.display()))?;
    }
    if !p.exists() {
        // Seed the file so `$EDITOR` opens something rather than an
        // empty buffer for a path it might refuse to create.
        std::fs::write(&p, "").with_context(|| format!("create {}", p.display()))?;
    }
    let editor = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| "vi".to_string());
    let status = Command::new(&editor)
        .arg(&p)
        .status()
        .with_context(|| format!("spawn {editor}"))?;
    if !status.success() {
        bail!("{editor} exited with {status}");
    }
    // Best-effort parse check — surface syntax errors without failing
    // the whole command, so the user's edits still land on disk.
    let raw = std::fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?;
    if let Err(e) = serde_yaml::from_str::<MiraConfig>(&raw) {
        eprintln!("warning: file no longer parses as MiraConfig: {e}");
    }
    Ok(())
}

fn get(key: &str, local: bool) -> Result<()> {
    let p = target_path(local)?;
    let cfg = if p.exists() {
        let raw = std::fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?;
        serde_yaml::from_str::<MiraConfig>(&raw)
            .with_context(|| format!("parse {}", p.display()))?
    } else {
        MiraConfig::default()
    };
    if let Some(value) = read_key(&cfg, key)? {
        println!("{value}");
    }
    Ok(())
}

fn set(key: &str, value: &str, local: bool) -> Result<()> {
    let p = target_path(local)?;
    let mut cfg = if p.exists() {
        let raw = std::fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?;
        serde_yaml::from_str::<MiraConfig>(&raw)
            .with_context(|| format!("parse {}", p.display()))?
    } else {
        MiraConfig::default()
    };
    write_key(&mut cfg, key, value)?;

    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("mkdir {}", parent.display()))?;
    }
    let yaml = serde_yaml::to_string(&cfg).context("serialize config")?;
    let tmp = p.with_extension("yaml.tmp");
    let mut f = std::fs::File::create(&tmp).with_context(|| format!("create {}", tmp.display()))?;
    f.write_all(yaml.as_bytes())
        .with_context(|| format!("write {}", tmp.display()))?;
    f.sync_all().ok();
    std::fs::rename(&tmp, &p)
        .with_context(|| format!("rename {} -> {}", tmp.display(), p.display()))?;
    eprintln!("set {key} in {}", p.display());
    Ok(())
}

/// Read a dotted key out of a [`MiraConfig`]. Returns `None` for a
/// well-formed key whose value is unset (so `get` prints nothing rather
/// than "null"); errors on unknown keys so a typo doesn't silently
/// produce an empty line.
fn read_key(cfg: &MiraConfig, key: &str) -> Result<Option<String>> {
    match key {
        "default_provider" => Ok(cfg.default_provider.clone()),
        "default_model" => Ok(cfg.default_model.clone()),
        "default_mode" => Ok(cfg.default_mode.clone()),
        "max_tokens" => Ok(cfg.max_tokens.map(|n| n.to_string())),
        "temperature" => Ok(cfg.temperature.map(|n| n.to_string())),
        "compactor_model" => Ok(cfg.compactor_model.clone()),
        other => {
            let parts: Vec<&str> = other.splitn(3, '.').collect();
            if parts.len() == 3 && parts[0] == "providers" {
                let name = parts[1];
                let field = parts[2];
                let entry = cfg.providers.get(name);
                Ok(match (entry, field) {
                    (Some(p), "base_url") => p.base_url.clone(),
                    (Some(p), "api_key") => p.api_key.clone(),
                    (Some(p), "api_key_env") => p.api_key_env.clone(),
                    (Some(p), "prompt_caching") => p.prompt_caching.map(|b| b.to_string()),
                    (None, _) => None,
                    _ => bail!("unknown provider field `{field}` (expected base_url, api_key, api_key_env, prompt_caching)"),
                })
            } else if parts.len() == 2 && parts[0] == "keys" {
                Ok(cfg.keys.get(parts[1]).cloned())
            } else {
                bail!("unknown key `{other}` — use `mira config edit` for nested tables")
            }
        }
    }
}

fn write_key(cfg: &mut MiraConfig, key: &str, value: &str) -> Result<()> {
    match key {
        "default_provider" => cfg.default_provider = some_or_clear(value),
        "default_model" => cfg.default_model = some_or_clear(value),
        "default_mode" => cfg.default_mode = some_or_clear(value),
        "max_tokens" => cfg.max_tokens = parse_opt_u32(value)?,
        "temperature" => cfg.temperature = parse_opt_f32(value)?,
        "compactor_model" => cfg.compactor_model = some_or_clear(value),
        other => {
            let parts: Vec<&str> = other.splitn(3, '.').collect();
            if parts.len() == 3 && parts[0] == "providers" {
                let name = parts[1].to_string();
                let entry: &mut ProviderConfig = cfg.providers.entry(name).or_default();
                match parts[2] {
                    "base_url" => entry.base_url = some_or_clear(value),
                    "api_key" => entry.api_key = some_or_clear(value),
                    "api_key_env" => entry.api_key_env = some_or_clear(value),
                    "prompt_caching" => entry.prompt_caching = parse_opt_bool(value)?,
                    other => bail!("unknown provider field `{other}` (expected base_url, api_key, api_key_env, prompt_caching)"),
                }
            } else if parts.len() == 2 && parts[0] == "keys" {
                let name = parts[1].to_string();
                if value.is_empty() {
                    cfg.keys.remove(&name);
                } else {
                    cfg.keys.insert(name, value.to_string());
                }
            } else {
                bail!("unknown key `{other}` — use `mira config edit` for nested tables");
            }
        }
    }
    Ok(())
}

fn some_or_clear(v: &str) -> Option<String> {
    if v.is_empty() {
        None
    } else {
        Some(v.to_string())
    }
}

fn parse_opt_u32(v: &str) -> Result<Option<u32>> {
    if v.is_empty() {
        return Ok(None);
    }
    v.parse::<u32>()
        .map(Some)
        .with_context(|| format!("`{v}` is not a valid u32"))
}

fn parse_opt_f32(v: &str) -> Result<Option<f32>> {
    if v.is_empty() {
        return Ok(None);
    }
    v.parse::<f32>()
        .map(Some)
        .with_context(|| format!("`{v}` is not a valid f32"))
}

fn parse_opt_bool(v: &str) -> Result<Option<bool>> {
    Ok(match v {
        "" => None,
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        other => bail!("`{other}` is not a boolean (expected true/false)"),
    })
}
