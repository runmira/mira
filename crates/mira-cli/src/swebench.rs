//! `mira eval swebench`: run Mira on SWE-bench and score it.
//!
//! SWE-bench tasks are real GitHub issues in real Python repos. Scoring
//! means running each repo's hidden tests in a task-specific Docker
//! image, which the official `swebench` harness does. So this works the
//! way every agent on the leaderboard does:
//!
//! 1. `mira eval swebench` checks out each task's repo at its commit,
//!    gives Mira the issue, and writes the resulting `git diff` to a
//!    predictions file in the harness's format (plus a log with tokens,
//!    cost and time per task).
//! 2. `mira eval swebench score` runs the harness on that file and
//!    prints how many tasks were resolved.
//!
//! Datasets: `lite` (300 tasks), `verified` (500), `full`, or a local
//! `.jsonl` / `.json` file with `instance_id`, `repo`, `base_commit` and
//! `problem_statement`. Named ones are exported once with Python's
//! `datasets` package (which `pip install swebench` brings) and cached
//! in `~/.mira/evals/swebench/`.
//!
//! Repos are cloned once into `~/.mira/evals/repos/` from
//! `https://github.com`, or from `$MIRA_SWEBENCH_REPO_BASE` (a mirror).

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::config::MiraConfig;

#[derive(Args, Debug, Clone)]
pub struct SwebenchArgs {
    #[command(subcommand)]
    pub action: Option<SwebenchAction>,

    /// `lite`, `verified`, `full`, or a path to a .jsonl / .json file.
    #[arg(long, default_value = "lite")]
    pub dataset: String,

    /// Only the first N tasks (after `--instance`).
    #[arg(long)]
    pub limit: Option<usize>,

    /// Only these tasks, by `instance_id`. Repeatable.
    #[arg(long = "instance", value_name = "ID")]
    pub instances: Vec<String>,

    /// Where predictions go (appended; finished tasks are skipped, so a
    /// stopped run picks up where it left off). A `.log.jsonl` with
    /// tokens, cost and time sits next to it.
    #[arg(long, default_value = "predictions.jsonl")]
    pub out: PathBuf,

    /// Tasks to run at the same time.
    #[arg(long, default_value_t = 1)]
    pub workers: usize,

    /// Time limit per task, in seconds.
    #[arg(long, default_value_t = 1800)]
    pub timeout_secs: u64,

    /// Cap on model rounds per task.
    #[arg(long)]
    pub max_turns: Option<usize>,

    /// Keep each task's checkout (under ~/.mira/evals/work/) to look at.
    #[arg(long)]
    pub keep: bool,
}

#[derive(Subcommand, Debug, Clone)]
pub enum SwebenchAction {
    /// Score predictions with the official harness (needs Python with
    /// `pip install swebench`, and Docker running).
    Score(ScoreArgs),
}

#[derive(Args, Debug, Clone)]
pub struct ScoreArgs {
    /// The predictions file `mira eval swebench` wrote.
    #[arg(long, default_value = "predictions.jsonl")]
    pub predictions: PathBuf,

    /// The dataset the predictions are for.
    #[arg(long, default_value = "lite")]
    pub dataset: String,

    /// Tasks the harness scores at the same time.
    #[arg(long, default_value_t = 4)]
    pub workers: usize,

    /// Names this scoring run (and its report file).
    #[arg(long)]
    pub run_id: Option<String>,
}

/// The fields of a SWE-bench task Mira uses.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Instance {
    pub instance_id: String,
    pub repo: String,
    pub base_commit: String,
    pub problem_statement: String,
}

pub async fn run(cli: &crate::Cli, args: SwebenchArgs) -> Result<()> {
    if let Some(SwebenchAction::Score(s)) = args.action.clone() {
        return score(s);
    }

    let cwd = std::env::current_dir().context("read cwd")?;
    let cfg = MiraConfig::load(&cwd).context("load config")?;
    let settings = Arc::new(crate::resolve_settings(cli, &cfg)?);
    let provider = mira_ai::build_chat_provider(
        &settings.provider_name,
        settings.base_url.clone(),
        settings.api_key.clone(),
        settings.extra_headers.clone(),
        settings.prompt_caching,
    )
    .context("build provider")?;

    let all = load_dataset(&args.dataset)?;
    let done = finished_ids(&args.out)?;
    let tasks = select(all, &args.instances, args.limit, &done);
    if tasks.is_empty() {
        eprintln!(
            "Nothing to run: {} task(s) already in {}.",
            done.len(),
            args.out.display()
        );
        return Ok(());
    }
    eprintln!(
        "SWE-bench ({}): {} task(s) with {} · {} worker(s){}",
        args.dataset,
        tasks.len(),
        settings.model,
        args.workers.max(1),
        if done.is_empty() {
            String::new()
        } else {
            format!(" · {} already done", done.len())
        }
    );

    let model_name = format!("mira/{}", settings.model);
    let out = Arc::new(Mutex::new(Outputs::open(&args.out)?));
    let repo_locks: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>> = Default::default();
    let total = tasks.len();
    let started = Instant::now();

    let runs = tasks.into_iter().enumerate().map(|(i, task)| {
        let provider = provider.clone();
        let settings = settings.clone();
        let out = out.clone();
        let repo_locks = repo_locks.clone();
        let model_name = model_name.clone();
        let args = args.clone();
        async move {
            let rec = run_one(&task, provider, &settings, &args, &repo_locks).await;
            eprintln!(
                "[{}/{}] {} · {} · {:.0}s · {}→{} tok{}",
                i + 1,
                total,
                task.instance_id,
                rec.status,
                rec.secs,
                rec.tokens_in,
                rec.tokens_out,
                rec.error
                    .as_deref()
                    .map(|e| format!(" · {e}"))
                    .unwrap_or_default()
            );
            let mut o = out.lock().await;
            if let Err(e) = o.write(&task, &model_name, &rec) {
                eprintln!("  couldn't save {}: {e:#}", task.instance_id);
            }
            rec
        }
    });
    let records: Vec<Record> = futures::stream::iter(runs)
        .buffer_unordered(args.workers.max(1))
        .collect()
        .await;

    print_summary(&records, &settings.model, started.elapsed(), &args);
    Ok(())
}

/// What happened on one task, for the log file.
#[derive(Debug, Default, Serialize)]
struct Record {
    instance_id: String,
    /// `patched` (a non-empty diff), `empty` (no changes) or `error`.
    status: &'static str,
    error: Option<String>,
    patch: String,
    tokens_in: u64,
    tokens_out: u64,
    cost_usd: Option<f64>,
    secs: f64,
}

async fn run_one(
    task: &Instance,
    provider: Arc<dyn mira_ai::ChatProvider>,
    settings: &crate::ResolvedSettings,
    args: &SwebenchArgs,
    repo_locks: &Mutex<HashMap<String, Arc<Mutex<()>>>>,
) -> Record {
    let started = Instant::now();
    let mut rec = Record {
        instance_id: task.instance_id.clone(),
        ..Default::default()
    };
    let result = async {
        // One clone per repo; tasks on the same repo wait for it.
        let lock = repo_locks
            .lock()
            .await
            .entry(task.repo.clone())
            .or_default()
            .clone();
        let cache = {
            let _held = lock.lock().await;
            let (repo, commit) = (task.repo.clone(), task.base_commit.clone());
            tokio::task::spawn_blocking(move || repo_cache(&repo, &commit))
                .await
                .context("repo cache task")??
        };
        let work = work_dir(&task.instance_id)?;
        let (w, c, commit) = (work.clone(), cache.clone(), task.base_commit.clone());
        tokio::task::spawn_blocking(move || checkout(&c, &w, &commit))
            .await
            .context("checkout task")??;

        let session = crate::eval::unattended_session(&work, provider, settings, args.max_turns)?;
        let prompt = task_prompt(task);
        let turn = crate::eval::drive_turn(session, &prompt);
        let outcome = tokio::time::timeout(Duration::from_secs(args.timeout_secs), turn).await;
        // Keep whatever it changed, even when time ran out.
        let patch = diff(&work);
        if !args.keep {
            let _ = std::fs::remove_dir_all(&work);
        }
        let (tokens_in, tokens_out) = match outcome {
            Ok(Ok((_, i, o))) => (i, o),
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                return Ok((
                    patch?,
                    0,
                    0,
                    Some(format!("timed out after {}s", args.timeout_secs)),
                ))
            }
        };
        Ok::<_, anyhow::Error>((patch?, tokens_in, tokens_out, None))
    }
    .await;

    rec.secs = started.elapsed().as_secs_f64();
    match result {
        Ok((patch, tokens_in, tokens_out, note)) => {
            rec.tokens_in = tokens_in;
            rec.tokens_out = tokens_out;
            rec.cost_usd = mira_ai::cost_usd(
                &settings.model,
                mira_ai::TokenUsage {
                    prompt_tokens: tokens_in.min(u32::MAX as u64) as u32,
                    completion_tokens: tokens_out.min(u32::MAX as u64) as u32,
                    cached_input_tokens: 0,
                },
            );
            rec.status = if patch.trim().is_empty() {
                "empty"
            } else {
                "patched"
            };
            rec.error = note;
            rec.patch = patch;
        }
        Err(e) => {
            rec.status = "error";
            rec.error = Some(format!("{e:#}"));
        }
    }
    rec
}

/// What Mira is asked for each task. The issue text is the task; the
/// rest keeps the change reviewable and honest about the environment.
pub fn task_prompt(task: &Instance) -> String {
    format!(
        "You're working in a checkout of the {repo} repository. Resolve the issue below by \
         changing the code.\n\
         \n\
         - Find the cause and make the smallest change that fixes it, in the library code.\n\
         - Hidden tests will check the fix. Don't change existing tests; you don't need to add any.\n\
         - The project's dependencies may not be installed, so its test suite might not run. \
         If a quick check works, use it; otherwise read the code carefully.\n\
         - Don't commit. When you're done, reply with one line on what you changed.\n\
         \n\
         <issue>\n{issue}\n</issue>",
        repo = task.repo,
        issue = task.problem_statement.trim(),
    )
}

/* ---------- dataset ---------- */

fn hf_name(dataset: &str) -> Option<&'static str> {
    match dataset.to_ascii_lowercase().as_str() {
        "lite" => Some("princeton-nlp/SWE-bench_Lite"),
        "verified" => Some("princeton-nlp/SWE-bench_Verified"),
        "full" => Some("princeton-nlp/SWE-bench"),
        _ => None,
    }
}

fn evals_dir() -> PathBuf {
    mira_config::global_path()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(".mira"))
        .join("evals")
}

/// The tasks in `dataset`: a named SWE-bench split (exported once and
/// cached) or a local file.
pub fn load_dataset(dataset: &str) -> Result<Vec<Instance>> {
    let path = match hf_name(dataset) {
        Some(name) => {
            let cached = evals_dir()
                .join("swebench")
                .join(format!("{}.jsonl", dataset.to_ascii_lowercase()));
            if !cached.exists() {
                export_dataset(name, &cached)?;
            }
            cached
        }
        None => PathBuf::from(dataset),
    };
    let text = std::fs::read_to_string(&path).with_context(|| {
        format!(
            "read {} (use `lite`, `verified`, `full`, or a .jsonl file)",
            path.display()
        )
    })?;
    parse_dataset(&text).with_context(|| format!("parse {}", path.display()))
}

/// A JSON array, or one JSON object per line.
pub fn parse_dataset(text: &str) -> Result<Vec<Instance>> {
    let trimmed = text.trim_start();
    if trimmed.starts_with('[') {
        return Ok(serde_json::from_str(trimmed)?);
    }
    trimmed
        .lines()
        .filter(|l| !l.trim().is_empty())
        .enumerate()
        .map(|(i, l)| serde_json::from_str(l).with_context(|| format!("line {}", i + 1)))
        .collect()
}

fn export_dataset(name: &str, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to.parent().unwrap_or(Path::new(".")))?;
    eprintln!("Downloading {name} (once)…");
    let script = format!(
        "from datasets import load_dataset\n\
         load_dataset({name:?}, split='test').to_json({to:?}, lines=True)\n",
        to = to.display().to_string(),
    );
    let status = Command::new("python3")
        .args(["-c", &script])
        .status()
        .context("run python3 (needed to download SWE-bench)")?;
    if !status.success() {
        let _ = std::fs::remove_file(to);
        bail!(
            "couldn't download {name}. Install the tools with `pip install swebench` \
             (it brings `datasets`), or pass --dataset path/to/tasks.jsonl"
        );
    }
    Ok(())
}

/// `instances` (all if empty) not already `done`, the first `limit`.
fn select(
    all: Vec<Instance>,
    instances: &[String],
    limit: Option<usize>,
    done: &HashSet<String>,
) -> Vec<Instance> {
    let wanted: HashSet<&str> = instances.iter().map(String::as_str).collect();
    all.into_iter()
        .filter(|t| wanted.is_empty() || wanted.contains(t.instance_id.as_str()))
        .filter(|t| !done.contains(&t.instance_id))
        .take(limit.unwrap_or(usize::MAX))
        .collect()
}

/* ---------- repos ---------- */

fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .context("run git")?;
    if !out.status.success() {
        bail!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// A bare clone of `repo` that has `commit`, cloned or fetched as needed.
fn repo_cache(repo: &str, commit: &str) -> Result<PathBuf> {
    let base = std::env::var("MIRA_SWEBENCH_REPO_BASE")
        .unwrap_or_else(|_| "https://github.com".to_owned());
    let url = format!("{}/{repo}.git", base.trim_end_matches('/'));
    let cache = evals_dir()
        .join("repos")
        .join(format!("{}.git", repo.replace('/', "__")));
    if !cache.exists() {
        eprintln!("Cloning {repo} (once)…");
        std::fs::create_dir_all(cache.parent().unwrap_or(Path::new(".")))?;
        let parent = cache.parent().unwrap_or(Path::new("."));
        let name = cache.file_name().and_then(|n| n.to_str()).unwrap_or(repo);
        git(parent, &["clone", "--quiet", "--bare", &url, name])
            .with_context(|| format!("clone {url}"))?;
    }
    let has = |c: &str| git(&cache, &["cat-file", "-e", &format!("{c}^{{commit}}")]).is_ok();
    if !has(commit) {
        git(
            &cache,
            &[
                "fetch",
                "--quiet",
                "origin",
                "+refs/heads/*:refs/heads/*",
                "+refs/tags/*:refs/tags/*",
            ],
        )
        .with_context(|| format!("fetch {repo}"))?;
        if !has(commit) {
            bail!("{repo} has no commit {commit}");
        }
    }
    Ok(cache)
}

fn work_dir(instance_id: &str) -> Result<PathBuf> {
    let dir = evals_dir().join("work").join(instance_id);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).with_context(|| format!("clear {}", dir.display()))?;
    }
    std::fs::create_dir_all(dir.parent().unwrap_or(Path::new(".")))?;
    Ok(dir)
}

/// `dir` becomes a checkout of `commit` sharing `cache`'s objects.
fn checkout(cache: &Path, dir: &Path, commit: &str) -> Result<()> {
    let parent = dir.parent().unwrap_or(Path::new("."));
    let name = dir
        .file_name()
        .and_then(|n| n.to_str())
        .context("work dir name")?;
    git(
        parent,
        &[
            "clone",
            "--quiet",
            "--shared",
            "--no-checkout",
            &cache.display().to_string(),
            name,
        ],
    )?;
    git(dir, &["checkout", "--quiet", "--detach", commit])?;
    Ok(())
}

/// Everything changed since the checkout, new files included, except
/// Mira's own `.mira/` folder (undo snapshots and the like).
fn diff(dir: &Path) -> Result<String> {
    git(dir, &["add", "-A", "--", ".", ":(exclude).mira"])?;
    git(dir, &["diff", "--cached", "--binary"])
}

/* ---------- output ---------- */

fn log_path(out: &Path) -> PathBuf {
    let name = out
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("predictions");
    out.with_file_name(format!("{name}.log.jsonl"))
}

/// Tasks already in the predictions file.
fn finished_ids(out: &Path) -> Result<HashSet<String>> {
    let Ok(text) = std::fs::read_to_string(out) else {
        return Ok(HashSet::new());
    };
    Ok(text
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter_map(|v| v["instance_id"].as_str().map(str::to_owned))
        .collect())
}

struct Outputs {
    predictions: std::fs::File,
    log: std::fs::File,
}

impl Outputs {
    fn open(out: &Path) -> Result<Self> {
        let open = |p: &Path| {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(p)
                .with_context(|| format!("open {}", p.display()))
        };
        Ok(Self {
            predictions: open(out)?,
            log: open(&log_path(out))?,
        })
    }

    /// A prediction for every task that ran (an empty patch scores as
    /// unresolved), except errors, so a rerun tries those again.
    fn write(&mut self, task: &Instance, model: &str, rec: &Record) -> Result<()> {
        if rec.status != "error" {
            let line = json!({
                "instance_id": task.instance_id,
                "model_name_or_path": model,
                "model_patch": rec.patch,
            });
            writeln!(self.predictions, "{line}")?;
            self.predictions.flush()?;
        }
        let mut log = serde_json::to_value(rec)?;
        if let Some(o) = log.as_object_mut() {
            o.remove("patch");
            o.insert("patch_lines".into(), json!(rec.patch.lines().count()));
        }
        writeln!(self.log, "{log}")?;
        self.log.flush()?;
        Ok(())
    }
}

fn print_summary(records: &[Record], model: &str, took: Duration, args: &SwebenchArgs) {
    let count = |s: &str| records.iter().filter(|r| r.status == s).count();
    let tokens_in: u64 = records.iter().map(|r| r.tokens_in).sum();
    let tokens_out: u64 = records.iter().map(|r| r.tokens_out).sum();
    let cost: Option<f64> = records.iter().map(|r| r.cost_usd).sum();
    println!();
    println!(
        "Ran {} task(s) with {model} in {:.0} min",
        records.len(),
        took.as_secs_f64() / 60.0
    );
    println!(
        "  {} with a patch · {} with no changes · {} errors",
        count("patched"),
        count("empty"),
        count("error")
    );
    println!(
        "  {tokens_in} tokens in · {tokens_out} out{}",
        cost.map(|c| format!(" · ${c:.2}")).unwrap_or_default()
    );
    println!(
        "  Predictions: {} · log: {}",
        args.out.display(),
        log_path(&args.out).display()
    );
    if count("error") > 0 {
        println!("  Run the same command again to retry the errors.");
    }
    println!();
    println!("Score it (needs `pip install swebench` and Docker running):");
    println!(
        "  mira eval swebench score --predictions {} --dataset {}",
        args.out.display(),
        args.dataset
    );
}

/* ---------- scoring ---------- */

fn score(args: ScoreArgs) -> Result<()> {
    let python_ok = Command::new("python3")
        .args(["-c", "import swebench"])
        .output()
        .is_ok_and(|o| o.status.success());
    if !python_ok {
        bail!("scoring needs the official harness: `pip install swebench` (Python 3.9+)");
    }
    let docker_ok = Command::new("docker")
        .arg("info")
        .output()
        .is_ok_and(|o| o.status.success());
    if !docker_ok {
        bail!("scoring runs each task's tests in Docker; start Docker and try again");
    }
    if !args.predictions.exists() {
        bail!("no predictions at {}", args.predictions.display());
    }
    let dataset = hf_name(&args.dataset)
        .map(str::to_owned)
        .unwrap_or_else(|| args.dataset.clone());
    let run_id = args
        .run_id
        .clone()
        .unwrap_or_else(|| format!("mira-{}", unix_secs()));
    eprintln!("Scoring with the SWE-bench harness (this builds Docker images the first time)…");
    let status = Command::new("python3")
        .args(["-m", "swebench.harness.run_evaluation", "--dataset_name"])
        .arg(&dataset)
        .arg("--predictions_path")
        .arg(&args.predictions)
        .args([
            "--max_workers",
            &args.workers.max(1).to_string(),
            "--run_id",
            &run_id,
        ])
        .status()
        .context("run the SWE-bench harness")?;
    if !status.success() {
        bail!("the harness exited with {status}");
    }
    let report = find_report(Path::new("."), &run_id)
        .context("the harness finished but its report file wasn't found")?;
    let v: Value = serde_json::from_str(&std::fs::read_to_string(&report)?)?;
    println!();
    println!("{}", report_line(&v));
    println!("  Report: {}", report.display());
    Ok(())
}

/// Seconds since 1970, to keep run ids unique.
fn unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The harness writes `<model>.<run_id>.json` in the working directory.
fn find_report(dir: &Path, run_id: &str) -> Option<PathBuf> {
    let suffix = format!(".{run_id}.json");
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(&suffix))
        })
}

/// "Resolved 12 of 50 submitted (24.0%) · 300 in the dataset".
pub fn report_line(v: &Value) -> String {
    let n = |k: &str| v.get(k).and_then(Value::as_u64);
    let resolved = n("resolved_instances").unwrap_or(0);
    let submitted = n("submitted_instances").unwrap_or(0);
    let pct = if submitted > 0 {
        resolved as f64 * 100.0 / submitted as f64
    } else {
        0.0
    };
    let mut line = format!("Resolved {resolved} of {submitted} submitted ({pct:.1}%)");
    if let Some(total) = n("total_instances") {
        line.push_str(&format!(" · {total} in the dataset"));
    }
    if let Some(e) = n("error_instances").filter(|e| *e > 0) {
        line.push_str(&format!(" · {e} couldn't be evaluated"));
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inst(id: &str) -> Instance {
        Instance {
            instance_id: id.into(),
            repo: "o/r".into(),
            base_commit: "abc".into(),
            problem_statement: "It breaks.".into(),
        }
    }

    #[test]
    fn datasets_in_both_shapes() {
        let line = r#"{"instance_id":"a__b-1","repo":"a/b","base_commit":"c1","problem_statement":"bug","patch":"ignored"}"#;
        let jsonl = format!("{line}\n\n{line}\n");
        assert_eq!(parse_dataset(&jsonl).unwrap().len(), 2);
        let arr = format!("[{line}]");
        let got = parse_dataset(&arr).unwrap();
        assert_eq!(got[0].repo, "a/b");
        assert!(parse_dataset("{\"instance_id\": 1}").is_err());
    }

    #[test]
    fn selection_skips_done_and_honours_limit_and_ids() {
        let all = vec![inst("a"), inst("b"), inst("c"), inst("d")];
        let done: HashSet<String> = ["b".to_owned()].into();
        let ids: Vec<_> = select(all.clone(), &[], Some(2), &done)
            .into_iter()
            .map(|t| t.instance_id)
            .collect();
        assert_eq!(ids, ["a", "c"]);
        let ids: Vec<_> = select(all, &["d".into(), "b".into()], None, &done)
            .into_iter()
            .map(|t| t.instance_id)
            .collect();
        assert_eq!(ids, ["d"]);
    }

    #[test]
    fn prompt_has_the_issue_and_the_rules() {
        let p = task_prompt(&inst("x"));
        assert!(p.contains("checkout of the o/r repository"));
        assert!(p.contains("<issue>\nIt breaks.\n</issue>"));
        assert!(p.contains("Don't change existing tests"));
    }

    #[test]
    fn report_lines() {
        let v = json!({"total_instances": 300, "submitted_instances": 50,
                       "resolved_instances": 12, "error_instances": 1});
        assert_eq!(
            report_line(&v),
            "Resolved 12 of 50 submitted (24.0%) · 300 in the dataset · 1 couldn't be evaluated"
        );
        assert_eq!(
            hf_name("Verified"),
            Some("princeton-nlp/SWE-bench_Verified")
        );
        assert_eq!(hf_name("mine.jsonl"), None);
        assert_eq!(
            log_path(Path::new("out/preds.jsonl")),
            Path::new("out/preds.log.jsonl")
        );
    }
}
