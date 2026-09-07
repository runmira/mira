//! `mira review` — terminal wrapper around the shared `mira-review` crate.
//!
//! The pure two-stage logic (stage 1 generate, stage 2 hostile re-verify)
//! lives in `mira-review` so `mira-server` can drive the same thing over HTTP.
//! This module handles CLI flags, diff collection (git / gh / stdin), colored
//! terminal rendering, and `--comment` posting to GitHub.

use std::io::Read;
use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use clap::Args;
use crossterm::style::Stylize;
use mira_ai::openai::{OpenAiCompatible, OpenAiConfig};
use mira_ai::ChatProvider;
use mira_review::{review, Finding, Progress, ProgressSink, Severity};

use crate::config::MiraConfig;

#[derive(Args, Debug, Clone)]
pub struct ReviewArgs {
    /// Git range to review. Defaults to the merge-base of `main` and `HEAD`.
    #[arg(long)]
    pub range: Option<String>,

    /// Read a unified diff from a file or `-` for stdin. Overrides `--range`
    /// and `--pr`.
    #[arg(long)]
    pub diff: Option<String>,

    /// Review a GitHub PR by number. Uses `gh pr diff N`.
    #[arg(long)]
    pub pr: Option<u32>,

    /// Post confirmed findings as a summary comment on `--pr`. Requires `gh`.
    #[arg(long)]
    pub comment: bool,

    /// Skip stage 2 (hostile re-verify). Faster; noisier.
    #[arg(long)]
    pub no_verify: bool,

    /// Emit findings as raw JSON on stdout instead of the human-readable
    /// terminal layout. Useful for scripting / editor plugins.
    #[arg(long)]
    pub json: bool,
}

pub async fn run(cli: &crate::Cli, args: ReviewArgs) -> Result<()> {
    let cwd = std::env::current_dir().context("read cwd")?;
    let cfg = MiraConfig::load(&cwd).context("load config")?;
    let settings = crate::resolve_settings(cli, &cfg)?;

    let provider: Box<dyn ChatProvider> = Box::new(
        OpenAiCompatible::new(OpenAiConfig {
            base_url: settings.base_url.clone(),
            api_key: settings.api_key.clone(),
            extra_headers: settings.extra_headers.clone(),
            prompt_caching: settings.prompt_caching,
        })
        .context("build provider")?,
    );

    let diff = collect_diff(&args, &cwd).context("collect diff")?;
    if diff.trim().is_empty() {
        eprintln!("no diff to review");
        return Ok(());
    }

    let progress = StderrProgress;
    let findings = review(
        &*provider,
        &settings.model,
        &diff,
        &cwd,
        !args.no_verify,
        &progress,
    )
    .await?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&findings)?);
    } else {
        render(&findings);
    }

    if args.comment {
        if let Some(pr) = args.pr {
            post_pr_comment(pr, &findings).context("post comment to PR")?;
        } else {
            eprintln!("warning: --comment requires --pr; skipping post");
        }
    }

    if findings.iter().any(|f| f.severity == Severity::Critical) {
        std::process::exit(1);
    }
    Ok(())
}

/* ---------- diff sourcing ---------- */

fn collect_diff(args: &ReviewArgs, cwd: &Path) -> Result<String> {
    if let Some(path) = &args.diff {
        return if path == "-" {
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .context("read stdin")?;
            Ok(buf)
        } else {
            std::fs::read_to_string(path).with_context(|| format!("read {path}"))
        };
    }
    if let Some(pr) = args.pr {
        return run_capture(cwd, &["gh", "pr", "diff", &pr.to_string()])
            .context("gh pr diff (is `gh` installed and authenticated?)");
    }
    ensure_git_repo(cwd)?;
    let range = args
        .range
        .clone()
        .unwrap_or_else(|| default_range(cwd).unwrap_or_else(|| "HEAD".to_string()));
    run_capture(cwd, &["git", "diff", &range])
}

fn ensure_git_repo(cwd: &Path) -> Result<()> {
    let out = Command::new("git")
        .current_dir(cwd)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output();
    match out {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => {
            let stderr = summarize_stderr(String::from_utf8_lossy(&o.stderr).trim());
            let reason = if stderr.is_empty() {
                "not a git repository".to_owned()
            } else {
                stderr
            };
            bail!("git check failed in {}: {reason}", cwd.display())
        }
        Err(e) => bail!("git not runnable in {}: {e}", cwd.display()),
    }
}

/// On a feature branch: `<base>...HEAD`. On the base branch (or no base
/// found): `None`, so the caller falls back to `HEAD` and picks up
/// working-tree changes — otherwise reviewing on `main` with local mods
/// silently produces "no diff to review".
fn default_range(cwd: &Path) -> Option<String> {
    for base in ["main", "master"] {
        if !branch_exists(cwd, base) {
            continue;
        }
        if is_current_branch(cwd, base) {
            return None;
        }
        return Some(format!("{base}...HEAD"));
    }
    None
}

fn branch_exists(cwd: &Path, name: &str) -> bool {
    Command::new("git")
        .current_dir(cwd)
        .args(["show-ref", "--verify", "--quiet"])
        .arg(format!("refs/heads/{name}"))
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn is_current_branch(cwd: &Path, name: &str) -> bool {
    Command::new("git")
        .current_dir(cwd)
        .args(["symbolic-ref", "--quiet", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == name)
        .unwrap_or(false)
}

fn run_capture(cwd: &Path, argv: &[&str]) -> Result<String> {
    let out = Command::new(argv[0])
        .current_dir(cwd)
        .args(&argv[1..])
        .output()
        .with_context(|| format!("spawn {}", argv.join(" ")))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        bail!(
            "{} exited {}: {}",
            argv.join(" "),
            out.status,
            summarize_stderr(stderr.trim()),
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Keep error output scannable: first meaningful line + byte cap.
/// Git's `--no-index` usage dump alone is ~2KB.
fn summarize_stderr(s: &str) -> String {
    const LIMIT: usize = 500;
    let first = s
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    if first.len() >= LIMIT {
        format!("{}…", &first[..LIMIT])
    } else {
        first.to_owned()
    }
}

/* ---------- terminal progress + rendering ---------- */

struct StderrProgress;

#[async_trait]
impl ProgressSink for StderrProgress {
    async fn emit(&self, event: Progress) {
        match event {
            Progress::Stage1Started { diff_lines } => {
                eprintln!(
                    "{} generating findings ({diff_lines} lines of diff)…",
                    "▸".with(crossterm::style::Color::Cyan),
                );
            }
            Progress::Stage1Completed { total_findings } => {
                eprintln!(
                    "{} stage 1 done — {total_findings} candidate finding{}",
                    "▸".with(crossterm::style::Color::Cyan),
                    if total_findings == 1 { "" } else { "s" },
                );
            }
            Progress::Stage2Started { total } => {
                eprintln!(
                    "{} hostile re-verify: {total} finding{}…",
                    "▸".with(crossterm::style::Color::Cyan),
                    if total == 1 { "" } else { "s" },
                );
            }
            Progress::Stage2Item {
                index,
                total,
                title,
                kept,
            } => {
                if let Some(kept) = kept {
                    let (verb, color) = if kept {
                        ("kept", crossterm::style::Color::White)
                    } else {
                        ("dropped", crossterm::style::Color::DarkYellow)
                    };
                    eprintln!(
                        "  [{index}/{total}] {} — {}",
                        title.as_str().with(crossterm::style::Color::White),
                        verb.with(color),
                    );
                }
            }
            Progress::Completed { kept, dropped } => {
                eprintln!(
                    "{} done: {kept} kept, {dropped} dropped",
                    "▸".with(crossterm::style::Color::Cyan)
                );
            }
        }
    }
}

fn render(findings: &[Finding]) {
    if findings.is_empty() {
        println!("{}", "✓ no findings".with(crossterm::style::Color::Green));
        return;
    }
    println!(
        "\n{} confirmed finding{}\n",
        findings.len(),
        if findings.len() == 1 { "" } else { "s" },
    );
    for (i, f) in findings.iter().enumerate() {
        let color = match f.severity {
            Severity::Critical => crossterm::style::Color::Red,
            Severity::High => crossterm::style::Color::Yellow,
            Severity::Medium => crossterm::style::Color::Cyan,
            Severity::Low => crossterm::style::Color::DarkGrey,
        };
        let where_ = match f.line {
            Some(n) => format!("{}:{}", f.file, n),
            None => f.file.clone(),
        };
        println!(
            "{}  {}  {}",
            format!("{}.", i + 1).with(crossterm::style::Color::DarkGrey),
            format!("[{}]", f.severity.label()).with(color).bold(),
            f.title.as_str().bold(),
        );
        println!("    {}", where_.with(crossterm::style::Color::DarkGrey));
        for line in f.explanation.lines() {
            println!("    {line}");
        }
        if let Some(fix) = &f.suggested_fix {
            println!(
                "    {} {}",
                "fix:".with(crossterm::style::Color::Green),
                fix
            );
        }
        if let Some(note) = &f.verify_note {
            println!(
                "    {} {}",
                "verified:".with(crossterm::style::Color::DarkGrey),
                note
            );
        }
        println!();
    }
}

fn post_pr_comment(pr: u32, findings: &[Finding]) -> Result<()> {
    let body = markdown_report(findings);
    let cwd = std::env::current_dir()?;
    let out = Command::new("gh")
        .current_dir(&cwd)
        .args(["pr", "comment", &pr.to_string(), "--body", &body])
        .output()
        .context("spawn gh pr comment")?;
    if !out.status.success() {
        bail!(
            "gh pr comment exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    eprintln!("posted comment on PR #{pr}");
    Ok(())
}

fn markdown_report(findings: &[Finding]) -> String {
    if findings.is_empty() {
        return "**mira review**: no findings.".to_string();
    }
    let mut out = format!(
        "**mira review** — {} finding{}\n",
        findings.len(),
        if findings.len() == 1 { "" } else { "s" },
    );
    for f in findings {
        let where_ = match f.line {
            Some(n) => format!("`{}:{}`", f.file, n),
            None => format!("`{}`", f.file),
        };
        out.push_str(&format!(
            "\n---\n\n### [{}] {}\n{}\n\n{}\n",
            f.severity.label(),
            f.title,
            where_,
            f.explanation,
        ));
        if let Some(fix) = &f.suggested_fix {
            out.push_str(&format!("\n**Suggested fix:** {fix}\n"));
        }
    }
    out
}
