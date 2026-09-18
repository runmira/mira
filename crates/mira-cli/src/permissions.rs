//! `mira permissions` — inspect and edit permission rules in `mira.yaml`.
//!
//! Rules are written verbatim (no wire re-serialization) so a hand-crafted
//! `Bash(cargo test:*)` stays exactly as the user typed it. Each write
//! validates against `mira_policy::Rule::from_str` so a typo doesn't
//! land silently.

use std::path::PathBuf;
use std::str::FromStr;

use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand, ValueEnum};
use mira_config::MiraConfig;
use mira_policy::Rule;

#[derive(Args, Debug, Clone)]
pub struct PermissionsArgs {
    #[command(subcommand)]
    action: PermissionsAction,
}

#[derive(Subcommand, Debug, Clone)]
enum PermissionsAction {
    /// Print every rule in the target config.
    Ls {
        #[arg(long)]
        local: bool,
    },
    /// Add a rule to the allow / ask / deny list.
    Add {
        /// Which list to add to. Defaults to `allow`.
        #[arg(long, value_enum, default_value_t = Bucket::Allow)]
        bucket: Bucket,
        /// The rule, e.g. `Bash(cargo test:*)` or `Edit(src/**)`.
        rule: String,
        #[arg(long)]
        local: bool,
    },
    /// Remove a rule (exact string match) from a list.
    Rm {
        #[arg(long, value_enum, default_value_t = Bucket::Allow)]
        bucket: Bucket,
        rule: String,
        #[arg(long)]
        local: bool,
    },
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
#[value(rename_all = "lowercase")]
enum Bucket {
    Allow,
    Ask,
    Deny,
}

pub async fn run(_cli: &crate::Cli, args: PermissionsArgs) -> Result<()> {
    match args.action {
        PermissionsAction::Ls { local } => ls(local),
        PermissionsAction::Add {
            bucket,
            rule,
            local,
        } => add(bucket, &rule, local),
        PermissionsAction::Rm {
            bucket,
            rule,
            local,
        } => rm(bucket, &rule, local),
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

fn load(path: &std::path::Path) -> Result<MiraConfig> {
    if !path.exists() {
        return Ok(MiraConfig::default());
    }
    let raw =
        std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    serde_yaml::from_str(&raw).with_context(|| format!("parse {}", path.display()))
}

fn save(cfg: &MiraConfig, path: &std::path::Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("mkdir {}", parent.display()))?;
    }
    let yaml = serde_yaml::to_string(cfg).context("serialize config")?;
    std::fs::write(path, yaml).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn ls(local: bool) -> Result<()> {
    let path = target_path(local)?;
    let cfg = load(&path)?;
    let p = &cfg.permissions;
    if p.allow.is_empty() && p.ask.is_empty() && p.deny.is_empty() {
        println!("no rules in {}", path.display());
        return Ok(());
    }
    println!("# {}", path.display());
    print_bucket("allow", &p.allow);
    print_bucket("ask", &p.ask);
    print_bucket("deny", &p.deny);
    Ok(())
}

fn print_bucket(label: &str, rules: &[String]) {
    if rules.is_empty() {
        return;
    }
    println!("[{label}]");
    for r in rules {
        println!("  {r}");
    }
}

fn add(bucket: Bucket, rule: &str, local: bool) -> Result<()> {
    // Parse first so we don't write garbage to the yaml.
    Rule::from_str(rule).with_context(|| format!("invalid rule `{rule}`"))?;

    let path = target_path(local)?;
    let mut cfg = load(&path)?;
    let list = list_for(&mut cfg, bucket);
    if list.iter().any(|r| r == rule) {
        eprintln!("already present in {} of {}", bucket_name(bucket), path.display());
        return Ok(());
    }
    list.push(rule.to_owned());
    save(&cfg, &path)?;
    eprintln!("added to {} in {}", bucket_name(bucket), path.display());
    Ok(())
}

fn rm(bucket: Bucket, rule: &str, local: bool) -> Result<()> {
    let path = target_path(local)?;
    let mut cfg = load(&path)?;
    let list = list_for(&mut cfg, bucket);
    let before = list.len();
    list.retain(|r| r != rule);
    if list.len() == before {
        bail!(
            "no matching rule in {} of {}",
            bucket_name(bucket),
            path.display()
        );
    }
    save(&cfg, &path)?;
    eprintln!("removed from {} in {}", bucket_name(bucket), path.display());
    Ok(())
}

fn list_for(cfg: &mut MiraConfig, bucket: Bucket) -> &mut Vec<String> {
    match bucket {
        Bucket::Allow => &mut cfg.permissions.allow,
        Bucket::Ask => &mut cfg.permissions.ask,
        Bucket::Deny => &mut cfg.permissions.deny,
    }
}

fn bucket_name(b: Bucket) -> &'static str {
    match b {
        Bucket::Allow => "allow",
        Bucket::Ask => "ask",
        Bucket::Deny => "deny",
    }
}
