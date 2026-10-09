//! `mira eval` — batch runner for regression eval tasks.
//!
//! Discovers YAML task specs under `evals/tasks/`, runs each in an
//! isolated tempdir with a fresh Session, and reports pass/fail per
//! task. This exists so context / prompt / tool changes have a
//! visible number attached — without it, "did that make Mira
//! smarter?" is unanswerable.
//!
//! First-pass shape:
//!
//! - One run per task. LLM output is stochastic, so single-shot
//!   pass/fail is a noisy signal; add multi-run averaging later.
//! - Two verifier kinds: `expect_grep` (regex against the final
//!   assistant text) and `verify` (shell command that must exit 0 in
//!   the tempdir). Tasks set at least one.
//! - Runs in `Yolo` policy mode + `AutoApprover` — evals run
//!   unattended and can't be blocked on prompts.
//! - Uses the same provider / model / config as the interactive CLI,
//!   so a run costs real API money. Not auto-triggered.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use clap::Args;
use futures::StreamExt;
use mira_ai::build_chat_provider;
use mira_core::ToolCall;
use mira_harness::{Approver, HarnessEvent, Session, SessionConfig, UsageTotals};
use mira_policy::{Decision, Mode, Policy, PolicyConfig};
use mira_sandbox::Sandbox;
use mira_tools::{builtin, Registry, ToolContext};
use regex::Regex;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::config::MiraConfig;

#[derive(Args, Debug, Clone)]
pub struct EvalArgs {
    #[command(subcommand)]
    pub command: Option<EvalCommand>,

    /// Directory containing YAML task specs. Defaults to `evals/tasks`
    /// under the current working directory.
    #[arg(long)]
    pub tasks_dir: Option<PathBuf>,

    /// Only run tasks whose file stem contains this substring.
    #[arg(long)]
    pub task: Option<String>,

    /// Emit results as JSON on stdout instead of the human-readable
    /// table. Progress lines still go to stderr in either mode.
    #[arg(long)]
    pub json: bool,

    /// Hard wall-clock cap per task, overriding each task's own
    /// `timeout_secs`. Useful for capping cost in CI.
    #[arg(long)]
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct TaskSpec {
    /// Human-readable name. Falls back to the file stem when omitted.
    #[serde(default)]
    name: Option<String>,
    /// The user turn we send to the model.
    prompt: String,
    /// Optional path (relative to the task file) to a seed directory
    /// that gets copied into the tempdir before the session starts.
    #[serde(default)]
    fixture: Option<String>,
    /// Regex applied case-insensitively against the final assistant
    /// message. Match = pass.
    #[serde(default)]
    expect_grep: Option<String>,
    /// Shell command run in the tempdir after the session finishes.
    /// Exit 0 = pass. Rendered by `bash -lc`.
    #[serde(default)]
    verify: Option<String>,
    /// Per-task timeout. `--timeout-secs` on the CLI overrides this.
    #[serde(default = "default_task_timeout")]
    timeout_secs: u64,
}

fn default_task_timeout() -> u64 {
    300
}

#[derive(Debug, Serialize)]
struct TaskOutcome {
    name: String,
    pass: bool,
    reason: String,
    tokens_in: u64,
    tokens_out: u64,
    /// Input tokens served from the provider's prompt cache.
    cached_tokens: u64,
    /// Input tokens written to the prompt cache (billed at a premium).
    cache_write_tokens: u64,
    /// What the task cost, when the model's price is known.
    #[serde(skip_serializing_if = "Option::is_none")]
    cost_usd: Option<f64>,
    duration_secs: f64,
}

impl TaskOutcome {
    /// Share of input tokens served from the prompt cache.
    fn cache_share(&self) -> f64 {
        if self.tokens_in == 0 {
            0.0
        } else {
            self.cached_tokens as f64 / self.tokens_in as f64
        }
    }
}

/// Auto-approve every gated call. Only used inside evals — we already
/// run in `Yolo` mode so the policy path always answers `Allow`; this
/// is a belt-and-braces so a mis-set policy config can't hang the run
/// on an interactive prompt.
struct AutoApprover;

#[async_trait]
impl Approver for AutoApprover {
    async fn approve(&self, _call: &ToolCall, _decision: Decision) -> bool {
        true
    }
}

/// Benchmarks beyond the local task files.
#[derive(clap::Subcommand, Debug, Clone)]
pub enum EvalCommand {
    /// Run Mira on SWE-bench (real GitHub issues) and score the patches.
    Swebench(crate::swebench::SwebenchArgs),
}

pub async fn run(cli: &crate::Cli, args: EvalArgs) -> Result<()> {
    if let Some(EvalCommand::Swebench(a)) = args.command.clone() {
        return crate::swebench::run(cli, a).await;
    }
    let cwd = std::env::current_dir().context("read cwd")?;
    let cfg = MiraConfig::load(&cwd).context("load config")?;
    let settings = crate::resolve_settings(cli, &cfg)?;

    let tasks_dir = args
        .tasks_dir
        .clone()
        .unwrap_or_else(|| cwd.join("evals/tasks"));
    let specs = discover_tasks(&tasks_dir, args.task.as_deref())
        .with_context(|| format!("discover tasks in {}", tasks_dir.display()))?;
    if specs.is_empty() {
        bail!("no task specs found in {}", tasks_dir.display());
    }

    // Shared provider so all tasks reuse one HTTP client.
    let provider: Arc<dyn mira_ai::ChatProvider> = build_chat_provider(
        &settings.provider_name,
        settings.base_url.clone(),
        settings.api_key.clone(),
        settings.extra_headers.clone(),
        settings.prompt_caching,
    )
    .context("build provider")?;

    let mut outcomes = Vec::with_capacity(specs.len());
    for (path, spec) in specs {
        let outcome = run_task(&spec, &path, provider.clone(), &settings, &args).await;
        // Progress goes to stderr so `--json` on stdout stays parseable.
        eprintln!(
            "[{}] {} ({:.1}s, {}→{} tok, {:.0}% from cache{}) — {}",
            if outcome.pass { "PASS" } else { "FAIL" },
            outcome.name,
            outcome.duration_secs,
            outcome.tokens_in,
            outcome.tokens_out,
            outcome.cache_share() * 100.0,
            outcome
                .cost_usd
                .map(|c| format!(", ${c:.4}"))
                .unwrap_or_default(),
            outcome.reason,
        );
        outcomes.push(outcome);
    }

    if args.json {
        println!("{}", serde_json::to_string_pretty(&outcomes)?);
    } else {
        print_summary(&outcomes);
    }

    let any_fail = outcomes.iter().any(|o| !o.pass);
    if any_fail {
        std::process::exit(1);
    }
    Ok(())
}

fn discover_tasks(dir: &Path, filter: Option<&str>) -> Result<Vec<(PathBuf, TaskSpec)>> {
    if !dir.is_dir() {
        bail!("not a directory: {}", dir.display());
    }
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            matches!(
                p.extension().and_then(|s| s.to_str()),
                Some("yaml") | Some("yml")
            )
        })
        .collect();
    entries.sort();
    let mut out = Vec::new();
    for path in entries {
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        if let Some(f) = filter {
            if !stem.contains(f) {
                continue;
            }
        }
        let text =
            std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        let mut spec: TaskSpec =
            serde_yaml::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
        if spec.name.is_none() {
            spec.name = Some(stem.to_owned());
        }
        if spec.expect_grep.is_none() && spec.verify.is_none() {
            bail!(
                "{}: task must set at least one of `expect_grep` or `verify`",
                path.display()
            );
        }
        out.push((path, spec));
    }
    Ok(out)
}

async fn run_task(
    spec: &TaskSpec,
    task_path: &Path,
    provider: Arc<dyn mira_ai::ChatProvider>,
    settings: &crate::ResolvedSettings,
    args: &EvalArgs,
) -> TaskOutcome {
    let name = spec.name.clone().unwrap_or_else(|| "?".into());
    let start = Instant::now();
    let timeout_secs = args.timeout_secs.unwrap_or(spec.timeout_secs);
    let fixture = spec
        .fixture
        .as_ref()
        .map(|f| task_path.parent().unwrap_or(Path::new(".")).join(f));
    match run_task_inner(spec, task_path, provider, settings, timeout_secs).await {
        Ok((final_text, usage, work_dir)) => {
            let (pass, reason) = evaluate(spec, &final_text, &work_dir, fixture.as_deref());
            TaskOutcome {
                name,
                pass,
                reason,
                tokens_in: usage.prompt_tokens,
                tokens_out: usage.completion_tokens,
                cached_tokens: usage.cached_input_tokens,
                cache_write_tokens: usage.cache_write_tokens,
                cost_usd: mira_ai::pricing::cost_usd(&settings.model, usage.as_token_usage()),
                duration_secs: start.elapsed().as_secs_f64(),
            }
        }
        Err(e) => TaskOutcome {
            name,
            pass: false,
            reason: format!("runner error: {e:#}"),
            tokens_in: 0,
            tokens_out: 0,
            cached_tokens: 0,
            cache_write_tokens: 0,
            cost_usd: None,
            duration_secs: start.elapsed().as_secs_f64(),
        },
    }
}

async fn run_task_inner(
    spec: &TaskSpec,
    task_path: &Path,
    provider: Arc<dyn mira_ai::ChatProvider>,
    settings: &crate::ResolvedSettings,
    timeout_secs: u64,
) -> Result<(String, UsageTotals, PathBuf)> {
    // Fresh tempdir per task. `keep` intentionally not called — TempDir
    // drops (and its cleanup fires) at scope exit.
    let tmp = tempfile::tempdir().context("create tempdir")?;
    let work_dir = tmp.path().to_owned();

    if let Some(fixture) = &spec.fixture {
        let src = task_path.parent().unwrap_or(Path::new(".")).join(fixture);
        copy_dir_recursive(&src, &work_dir)
            .with_context(|| format!("copy fixture {}", src.display()))?;
    }

    let session = unattended_session(&work_dir, provider, settings, None)?;
    let turn = drive_turn_usage(session, &spec.prompt);
    let (final_text, usage) =
        match tokio::time::timeout(Duration::from_secs(timeout_secs), turn).await {
            Ok(r) => r?,
            Err(_) => bail!("timed out after {timeout_secs}s"),
        };

    // Return `work_dir` explicitly — verify commands run there. We
    // must keep `tmp` alive until the verify pass completes; leak
    // (into_path) so the caller can inspect and the drop happens
    // only after the outcome is scored.
    let kept = tmp.keep();
    Ok((final_text, usage, kept))
}

/// A session working in `work_dir` with nobody to ask: core tools, yolo
/// policy, every call approved, inside the usual command sandbox.
pub(crate) fn unattended_session(
    work_dir: &Path,
    provider: Arc<dyn mira_ai::ChatProvider>,
    settings: &crate::ResolvedSettings,
    max_rounds: Option<usize>,
) -> Result<Session> {
    let sandbox = Arc::new(Sandbox::default_scrubbed());
    let mut registry = Registry::new();
    builtin::register_core(&mut registry);
    let registry = Arc::new(registry);

    let policy = Policy::from_config(&PolicyConfig {
        mode: Mode::Yolo,
        allow: Vec::new(),
        ask: Vec::new(),
        deny: Vec::new(),
    })
    .context("compile eval policy")?;
    let policy = Arc::new(Mutex::new(policy));

    let approver: Arc<dyn Approver> = Arc::new(AutoApprover);
    let tool_ctx = ToolContext::new(work_dir.to_path_buf(), sandbox);

    let mut sess_cfg = SessionConfig::new(settings.model.clone());
    sess_cfg.max_tokens = settings.max_tokens;
    sess_cfg.temperature = settings.temperature;
    sess_cfg.compactor_model = settings.compactor_model.clone();
    sess_cfg.small_model = settings.small_model.clone();
    if let Some(n) = max_rounds {
        sess_cfg.max_rounds = n.max(1);
    }

    Ok(Session::new(
        sess_cfg,
        mira_server::system_prompt(work_dir, &registry),
        provider,
        registry,
        policy,
        approver,
        tool_ctx,
    ))
}

/// Send `prompt` and wait for the turn: the last round's text, and
/// input / output tokens.
pub(crate) async fn drive_turn(session: Session, prompt: &str) -> Result<(String, u64, u64)> {
    let (text, usage) = drive_turn_usage(session, prompt).await?;
    Ok((text, usage.prompt_tokens, usage.completion_tokens))
}

/// [`drive_turn`], with the turn's full token usage (cache reads and
/// writes included).
async fn drive_turn_usage(session: Session, prompt: &str) -> Result<(String, UsageTotals)> {
    let mut stream = session.send(prompt.to_owned()).await;
    let mut text = String::new();
    let mut usage = UsageTotals::default();
    let mut warnings: Vec<String> = Vec::new();
    // TurnComplete fires at the end of every model round (including
    // the final one). We keep the accumulated text — clearing on the
    // next round's first Token — so at Done we hold the last round's
    // free-form text, which is what a user would see as the "answer".
    let mut round_just_finished = false;
    while let Some(ev) = stream.next().await {
        match ev {
            HarnessEvent::Token(t) => {
                if round_just_finished {
                    text.clear();
                    round_just_finished = false;
                }
                text.push_str(&t);
            }
            HarnessEvent::TurnComplete => {
                round_just_finished = true;
            }
            HarnessEvent::Usage { round, .. } => usage.add_round(round),
            HarnessEvent::Warning(w) => warnings.push(w),
            HarnessEvent::Done => break,
            _ => {}
        }
    }
    // No model round at all: the turn ended before reaching the model (a
    // provider error such as a bad key, a blocked prompt). Say why instead
    // of grading an empty run.
    if usage.rounds == 0 && text.is_empty() && !warnings.is_empty() {
        bail!("no reply from the model: {}", warnings.join(" | "));
    }
    Ok((text, usage))
}

/// Score a finished task. `verify` runs in the task's directory with
/// `EVAL_FIXTURE` set to the untouched fixture, so it can check what the
/// agent shouldn't have changed.
fn evaluate(
    spec: &TaskSpec,
    final_text: &str,
    work_dir: &Path,
    fixture: Option<&Path>,
) -> (bool, String) {
    if let Some(pat) = &spec.expect_grep {
        let re = match Regex::new(&format!("(?i){pat}")) {
            Ok(r) => r,
            Err(e) => return (false, format!("bad expect_grep regex: {e}")),
        };
        if !re.is_match(final_text) {
            return (
                false,
                format!("expect_grep /{pat}/ did not match final message"),
            );
        }
    }
    if let Some(cmd) = &spec.verify {
        let mut command = std::process::Command::new("bash");
        command.arg("-lc").arg(cmd).current_dir(work_dir);
        if let Some(fixture) = fixture {
            command.env(
                "EVAL_FIXTURE",
                fixture
                    .canonicalize()
                    .unwrap_or_else(|_| fixture.to_owned()),
            );
        }
        let out = command.output();
        match out {
            Ok(o) if o.status.success() => {}
            Ok(o) => {
                let tail = String::from_utf8_lossy(&o.stderr);
                let tail_lines: Vec<&str> = tail.lines().rev().take(3).collect();
                let tail_lines: Vec<&str> = tail_lines.into_iter().rev().collect();
                return (
                    false,
                    format!(
                        "verify failed (exit {}): {}",
                        o.status.code().unwrap_or(-1),
                        tail_lines.join(" | "),
                    ),
                );
            }
            Err(e) => return (false, format!("verify launch: {e}")),
        }
    }
    (true, "ok".into())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    if !src.is_dir() {
        bail!("fixture not a directory: {}", src.display());
    }
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

fn print_summary(outcomes: &[TaskOutcome]) {
    println!();
    println!(
        "{:<32} {:>6} {:>9} {:>8} {:>7} {:>9} {:>7}",
        "task", "result", "tok in", "tok out", "cached", "cost", "time"
    );
    println!("{}", "-".repeat(84));
    for o in outcomes {
        let name = if o.name.len() > 32 {
            format!("{}…", &o.name[..31])
        } else {
            o.name.clone()
        };
        println!(
            "{:<32} {:>6} {:>9} {:>8} {:>6.0}% {:>9} {:>6.1}s",
            name,
            if o.pass { "PASS" } else { "FAIL" },
            o.tokens_in,
            o.tokens_out,
            o.cache_share() * 100.0,
            o.cost_usd
                .map(|c| format!("${c:.4}"))
                .unwrap_or_else(|| "?".into()),
            o.duration_secs,
        );
    }
    let pass = outcomes.iter().filter(|o| o.pass).count();
    let tokens_in: u64 = outcomes.iter().map(|o| o.tokens_in).sum();
    let cached: u64 = outcomes.iter().map(|o| o.cached_tokens).sum();
    let cost: Option<f64> = outcomes.iter().map(|o| o.cost_usd).sum();
    println!();
    println!(
        "{}/{} passed · {:.0}% of input from cache{}",
        pass,
        outcomes.len(),
        if tokens_in == 0 {
            0.0
        } else {
            cached as f64 * 100.0 / tokens_in as f64
        },
        cost.map(|c| format!(" · ${c:.4} total"))
            .unwrap_or_default()
    );
    for o in outcomes.iter().filter(|o| !o.pass) {
        println!("  ✗ {}: {}", o.name, o.reason);
    }
}
