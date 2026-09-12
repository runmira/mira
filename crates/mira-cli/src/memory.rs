//! `mira memory` — cross-session memory management from the shell.
//!
//! Currently one subcommand:
//!
//!   `mira memory consolidate <scope> [--model MODEL] [--instruction TEXT] [--dry-run] [--yes]`
//!
//! Consolidates a MIRA.md (or the episodic JSONL) via a cheap-model call,
//! sharing [`mira_tools::consolidate::consolidate_bullets`] with the
//! agent-facing tool so the two paths produce identical output. Wraps
//! the model call with backup + atomic overwrite; `--dry-run` prints
//! the proposed consolidation without writing.

use std::io::Write;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Subcommand};
use mira_ai::build_chat_provider;
use mira_config::MiraConfig;
use mira_memory::{
    project_episodic_path, EpisodicEntry, EpisodicSource, EpisodicStore, FileEpisodicStore,
    FileMemoryStore, MemoryScope, MemoryStore,
};
use mira_tools::consolidate::consolidate_bullets;

#[derive(Args, Debug, Clone)]
pub struct MemoryArgs {
    #[command(subcommand)]
    action: MemoryAction,
}

#[derive(Subcommand, Debug, Clone)]
enum MemoryAction {
    /// Dedup, merge, and clean up a memory file via a cheap-model call.
    /// A backup of the original is saved next to the source.
    Consolidate(ConsolidateArgs),
}

#[derive(Args, Debug, Clone)]
struct ConsolidateArgs {
    /// Which memory to consolidate. `user` = ~/.mira/MIRA.md, `project`
    /// = <cwd>/.mira/MIRA.md, `episodic` = <cwd>/.mira/episodic.jsonl.
    #[arg(value_enum)]
    scope: Scope,
    /// Override the model that runs the consolidation. Falls back to
    /// `memory.extractor_model` in `mira.yaml`, then to the default
    /// session model.
    #[arg(long)]
    model: Option<String>,
    /// Free-text nudge threaded into the consolidator's system prompt.
    #[arg(long, short = 'i')]
    instruction: Option<String>,
    /// Show the proposed consolidation without writing to disk.
    #[arg(long)]
    dry_run: bool,
    /// Skip the interactive confirmation prompt (implied when the
    /// process isn't attached to a TTY).
    #[arg(long, short = 'y')]
    yes: bool,
}

#[derive(clap::ValueEnum, Debug, Clone, Copy)]
#[value(rename_all = "snake_case")]
enum Scope {
    User,
    Project,
    Episodic,
}

pub async fn run(cli: &crate::Cli, args: MemoryArgs) -> Result<()> {
    match args.action {
        MemoryAction::Consolidate(a) => consolidate(cli, a).await,
    }
}

async fn consolidate(cli: &crate::Cli, args: ConsolidateArgs) -> Result<()> {
    let cwd = std::env::current_dir().context("read cwd")?;
    let cfg = MiraConfig::load(&cwd).context("load config")?;
    let settings = super::resolve_settings(cli, &cfg)?;

    let model = args
        .model
        .clone()
        .or_else(|| cfg.memory.extractor_model().map(str::to_owned))
        .unwrap_or_else(|| settings.model.clone());

    let provider = build_chat_provider(
        &settings.provider_name,
        settings.base_url.clone(),
        settings.api_key.clone(),
        settings.extra_headers.clone(),
        settings.prompt_caching,
    )
    .context("build provider")?;

    match args.scope {
        Scope::User => {
            consolidate_md(
                Arc::new(FileMemoryStore::new(
                    mira_config::user_memory_path(),
                    mira_config::project_memory_path(&cwd),
                )),
                MemoryScope::User,
                &*provider,
                &model,
                args.instruction.as_deref(),
                args.dry_run,
                args.yes,
            )
            .await
        }
        Scope::Project => {
            consolidate_md(
                Arc::new(FileMemoryStore::new(
                    mira_config::user_memory_path(),
                    mira_config::project_memory_path(&cwd),
                )),
                MemoryScope::Project,
                &*provider,
                &model,
                args.instruction.as_deref(),
                args.dry_run,
                args.yes,
            )
            .await
        }
        Scope::Episodic => {
            let store: Arc<dyn EpisodicStore> =
                Arc::new(FileEpisodicStore::new(project_episodic_path(&cwd)));
            consolidate_episodic(
                store,
                &*provider,
                &model,
                args.instruction.as_deref(),
                args.dry_run,
                args.yes,
            )
            .await
        }
    }
}

async fn consolidate_md(
    store: Arc<dyn MemoryStore>,
    scope: MemoryScope,
    provider: &dyn mira_ai::ChatProvider,
    model: &str,
    instruction: Option<&str>,
    dry_run: bool,
    assume_yes: bool,
) -> Result<()> {
    let path = store.path(scope);
    let current = store.read(scope).await.context("read memory")?;
    if current.trim().is_empty() {
        bail!("{} is empty; nothing to consolidate", path.display());
    }

    eprintln!("consolidating {} …", path.display());
    let consolidated =
        consolidate_bullets(provider, model, &current, instruction).await?;

    println!("--- proposed consolidation ---");
    println!("{consolidated}");
    println!("--- end ---");
    eprintln!(
        "before: {} bytes  ·  after: {} bytes",
        current.len(),
        consolidated.len()
    );

    if dry_run {
        eprintln!("dry run — nothing written");
        return Ok(());
    }
    if !assume_yes && !confirm(&format!("overwrite {}?", path.display()))? {
        eprintln!("aborted");
        return Ok(());
    }

    let backup_path = backup_path_for(&path);
    std::fs::write(&backup_path, current.as_bytes())
        .with_context(|| format!("write backup {}", backup_path.display()))?;
    let bytes = store
        .overwrite(scope, &consolidated)
        .await
        .context("overwrite memory")?;
    eprintln!(
        "wrote {} · {} bytes · backup at {}",
        path.display(),
        bytes,
        backup_path.display()
    );
    Ok(())
}

async fn consolidate_episodic(
    store: Arc<dyn EpisodicStore>,
    provider: &dyn mira_ai::ChatProvider,
    model: &str,
    instruction: Option<&str>,
    dry_run: bool,
    assume_yes: bool,
) -> Result<()> {
    let path = store.path();
    let entries = store.recent(usize::MAX).await.context("read episodic")?;
    if entries.is_empty() {
        bail!("{} is empty; nothing to consolidate", path.display());
    }

    eprintln!("consolidating {} · {} entries …", path.display(), entries.len());
    let serialised = entries
        .iter()
        .map(|e| format!("- {}", e.text.trim()))
        .collect::<Vec<_>>()
        .join("\n");
    let consolidated =
        consolidate_bullets(provider, model, &serialised, instruction).await?;

    let new_entries: Vec<EpisodicEntry> = parse_bullets(&consolidated)
        .into_iter()
        .map(|text| EpisodicEntry {
            text,
            timestamp: now_secs(),
            session_id: None,
            source: EpisodicSource::Auto,
        })
        .collect();

    println!("--- proposed consolidation ---");
    for e in &new_entries {
        println!("- {}", e.text);
    }
    println!("--- end ---");
    eprintln!("before: {} entries  ·  after: {} entries", entries.len(), new_entries.len());

    if dry_run {
        eprintln!("dry run — nothing written");
        return Ok(());
    }
    if !assume_yes && !confirm(&format!("overwrite {}?", path.display()))? {
        eprintln!("aborted");
        return Ok(());
    }

    let backup_path = backup_path_for(&path);
    if let Ok(current) = std::fs::read_to_string(&path) {
        std::fs::write(&backup_path, current.as_bytes())
            .with_context(|| format!("write backup {}", backup_path.display()))?;
    }
    let n = new_entries.len();
    store
        .overwrite_all(new_entries)
        .await
        .map_err(|e| anyhow!("overwrite episodic: {e}"))?;
    eprintln!(
        "wrote {} · {} entries · backup at {}",
        path.display(),
        n,
        backup_path.display()
    );
    Ok(())
}

fn confirm(prompt: &str) -> Result<bool> {
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() {
        // Non-interactive shell — refuse without `--yes` rather than
        // silently proceeding.
        return Ok(false);
    }
    print!("{prompt} [y/N] ");
    std::io::stdout().flush().ok();
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .context("read confirmation")?;
    let a = line.trim().to_ascii_lowercase();
    Ok(a == "y" || a == "yes")
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn backup_path_for(path: &std::path::Path) -> std::path::PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(format!(".pre-consolidate-{}.bak", now_secs()));
    match path.parent() {
        Some(p) => p.join(name),
        None => std::path::PathBuf::from(name),
    }
}

/// Same bullet-folding as the tool wrapper — indented continuation
/// lines join their parent, blank lines close the current bullet.
fn parse_bullets(content: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current: Option<String> = None;
    for line in content.lines() {
        let ts = line.trim_start();
        if ts.starts_with("- ") || ts.starts_with("* ") {
            if let Some(prev) = current.take() {
                push_nonempty(&mut out, prev);
            }
            current = Some(ts[2..].trim().to_string());
            continue;
        }
        if current.is_some() && (line.starts_with("  ") || line.starts_with('\t')) {
            if let Some(buf) = current.as_mut() {
                let piece = ts.trim();
                if !piece.is_empty() {
                    if !buf.is_empty() {
                        buf.push(' ');
                    }
                    buf.push_str(piece);
                }
            }
            continue;
        }
        if ts.is_empty() {
            if let Some(prev) = current.take() {
                push_nonempty(&mut out, prev);
            }
        }
    }
    if let Some(prev) = current.take() {
        push_nonempty(&mut out, prev);
    }
    out
}

fn push_nonempty(out: &mut Vec<String>, s: String) {
    let t = s.trim().to_string();
    if !t.is_empty() {
        out.push(t);
    }
}
