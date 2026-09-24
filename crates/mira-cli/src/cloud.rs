//! `mira cloud`: start a task in a cloud sandbox, close the laptop, come
//! back to a pull request.
//!
//! - `mira cloud run "<task>"` launches it (seconds) and returns.
//! - `mira cloud list | logs <id> | stop <id>` check on it.
//! - `mira cloud worker` is what runs inside the sandbox.

use std::path::Path;
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Subcommand};
use mira_cloud::launcher::{Launch, MiraInstall};
use mira_cloud::spec::{self, GitIdentity, Limits, ModelSpec, RepoSpec, Secrets, TaskSpec};
use mira_cloud::store::TaskStore;
use mira_compute::env::BackendSpec;
use mira_compute::{E2bBackend, EnvironmentSpec};
use mira_config::MiraConfig;

#[derive(Args, Debug, Clone)]
pub struct CloudArgs {
    #[command(subcommand)]
    action: CloudAction,
}

#[derive(Subcommand, Debug, Clone)]
enum CloudAction {
    /// Start a task in a cloud sandbox. It works from the base branch on
    /// GitHub, pushes to a new `mira/…` branch, and opens a pull request.
    Run(RunArgs),
    /// Cloud tasks started from this machine, with their PR and state.
    List,
    /// Recent log lines of a running task.
    Logs {
        id: String,
        #[arg(short = 'n', long, default_value_t = 40)]
        lines: usize,
    },
    /// Stop a task and delete its sandbox. Pushed work stays on the branch.
    Stop { id: String },
    /// Runs inside the sandbox; started by `mira cloud run`.
    #[command(hide = true)]
    Worker {
        #[arg(long)]
        spec: String,
        #[arg(long)]
        secrets: String,
    },
}

#[derive(Args, Debug, Clone)]
struct RunArgs {
    /// What to do, in plain words. Quote it.
    task: String,
    /// Environment from `compute.environments` (must be E2B). Default:
    /// `cloud.environment`, else the built-in `e2b`.
    #[arg(long)]
    env: Option<String>,
    /// Branch to start from. Default: the current branch.
    #[arg(long)]
    base: Option<String>,
    /// Wall-clock budget in minutes. Default: `cloud.max_runtime_secs`.
    #[arg(long)]
    max_minutes: Option<u64>,
    /// Goal-loop iterations. Default: `cloud.max_iterations` (20).
    #[arg(long)]
    max_iterations: Option<usize>,
    /// Spend cap in USD.
    #[arg(long)]
    budget_usd: Option<f64>,
}

pub async fn run(cli: &super::Cli, args: CloudArgs) -> Result<()> {
    match args.action {
        CloudAction::Worker { spec, secrets } => worker(&spec, &secrets).await,
        action => {
            let cwd = std::env::current_dir().context("read cwd")?;
            let cfg = MiraConfig::load(&cwd).context("load config")?;
            mira_config::export_keys_to_env(&cfg);
            match action {
                CloudAction::Run(a) => launch(cli, &cfg, &cwd, a).await,
                CloudAction::List => list(&cfg).await,
                CloudAction::Logs { id, lines } => logs(&cfg, &id, lines).await,
                CloudAction::Stop { id } => stop(&cfg, &id).await,
                CloudAction::Worker { .. } => unreachable!(),
            }
        }
    }
}

fn git_out(dir: &Path, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .filter(|s| !s.is_empty())
}

fn github_token(cfg: &MiraConfig) -> Result<String> {
    let names: Vec<&str> = match &cfg.cloud.github_token_env {
        Some(n) => vec![n.as_str()],
        None => vec!["GITHUB_TOKEN", "GH_TOKEN"],
    };
    for n in &names {
        if let Ok(v) = std::env::var(n) {
            if !v.trim().is_empty() {
                return Ok(v.trim().to_owned());
            }
        }
    }
    if cfg.cloud.github_token_env.is_none() {
        if let Ok(out) = std::process::Command::new("gh")
            .args(["auth", "token"])
            .output()
        {
            let t = String::from_utf8_lossy(&out.stdout).trim().to_owned();
            if out.status.success() && !t.is_empty() {
                return Ok(t);
            }
        }
    }
    bail!(
        "cloud tasks need a GitHub token to push and open the PR: set {} (a fine-grained token \
         for this repo with Contents + Pull requests read/write), or sign in with `gh auth login`",
        names.join(" or ")
    )
}

fn environment(cfg: &MiraConfig, flag: Option<&str>) -> Result<EnvironmentSpec> {
    let name = flag
        .map(str::to_owned)
        .or_else(|| cfg.cloud.environment.clone())
        .unwrap_or_else(|| "e2b".to_owned());
    let spec = EnvironmentSpec::resolve(&name, &cfg.compute).map_err(|e| anyhow!("{e}"))?;
    if !matches!(spec.backend, BackendSpec::E2b(_)) {
        bail!(
            "cloud tasks run on E2B; environment `{name}` uses `{}`",
            spec.backend_name()
        );
    }
    Ok(spec)
}

fn install_mode(cfg: &MiraConfig) -> Result<MiraInstall> {
    let release = || MiraInstall::Release(format!("v{}", env!("CARGO_PKG_VERSION")));
    // An explicit Linux build (`cloud.binary`, or MIRA_CLOUD_BINARY) wins;
    // otherwise the running binary when it can run in the sandbox.
    let linux_binary = || -> Result<Option<std::path::PathBuf>> {
        let explicit = std::env::var("MIRA_CLOUD_BINARY")
            .ok()
            .or_else(|| cfg.cloud.binary.clone());
        if let Some(p) = explicit {
            let p = std::path::PathBuf::from(shellexpand::tilde(&p).as_ref());
            if !p.is_file() {
                bail!("cloud.binary `{}` doesn't exist", p.display());
            }
            return Ok(Some(p));
        }
        Ok(cfg!(all(target_os = "linux", target_arch = "x86_64"))
            .then(std::env::current_exe)
            .transpose()?)
    };
    Ok(match cfg.cloud.install.as_deref().unwrap_or("auto") {
        "preinstalled" => MiraInstall::Preinstalled,
        "release" => release(),
        "upload" => MiraInstall::Upload(linux_binary()?.ok_or_else(|| {
            anyhow!(
                "`install: upload` from this OS needs a Linux x86_64 build: set cloud.binary \
                 (see docs/cloud-tasks.md)"
            )
        })?),
        "auto" => match linux_binary()? {
            Some(p) => MiraInstall::Upload(p),
            None => release(),
        },
        other => bail!("cloud.install: unknown value `{other}` (auto|preinstalled|upload|release)"),
    })
}

async fn launch(cli: &super::Cli, cfg: &MiraConfig, cwd: &Path, a: RunArgs) -> Result<()> {
    let remote = git_out(cwd, &["remote", "get-url", "origin"])
        .ok_or_else(|| anyhow!("no `origin` remote here; cloud tasks work from a GitHub repo"))?;
    let (owner, name) = spec::parse_github_remote(&remote)
        .ok_or_else(|| anyhow!("`origin` ({remote}) isn't a GitHub repository"))?;
    let base = match a.base {
        Some(b) => b,
        None => git_out(cwd, &["rev-parse", "--abbrev-ref", "HEAD"])
            .filter(|b| b != "HEAD")
            .ok_or_else(|| anyhow!("detached HEAD; pass --base <branch>"))?,
    };
    // The sandbox clones from GitHub, so the base must be there. Only a
    // definite "not there" stops us; a failed lookup (auth, network) is
    // left for the sandbox to report.
    if let Ok(out) = std::process::Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["ls-remote", "--heads", "origin", &base])
        .output()
    {
        if out.status.success() && out.stdout.is_empty() {
            bail!("branch `{base}` isn't on GitHub yet; push it first (or pass --base <branch>)");
        }
    }
    // The task starts from GitHub, not this checkout: say what that leaves out.
    if git_out(cwd, &["status", "--porcelain"]).is_some() {
        eprintln!(
            "note: uncommitted changes here aren't included; the task starts from origin/{base}"
        );
    }
    if let Some(n) = git_out(
        cwd,
        &["rev-list", "--count", &format!("origin/{base}..HEAD")],
    ) {
        if n != "0" {
            eprintln!(
                "note: {n} local commit(s) aren't pushed; the task starts from origin/{base}"
            );
        }
    }

    let settings = super::resolve_settings(cli, cfg)?;
    let github_token = github_token(cfg)?;
    let environment = environment(cfg, a.env.as_deref())?;
    let BackendSpec::E2b(e2b) = &environment.backend else {
        unreachable!("checked in environment()")
    };
    let identity = GitIdentity {
        name: git_out(cwd, &["config", "user.name"]).unwrap_or_else(|| "Mira".into()),
        email: git_out(cwd, &["config", "user.email"]).unwrap_or_else(|| "mira@localhost".into()),
    };
    let max_runtime_secs = a
        .max_minutes
        .map(|m| m * 60)
        .or(cfg.cloud.max_runtime_secs)
        .unwrap_or(3600)
        .clamp(300, 24 * 3600);
    let id = spec::new_task_id();
    let spec = TaskSpec {
        branch: spec::branch_name(&a.task, &id),
        id,
        prompt: a.task,
        repo: RepoSpec {
            clone_url: format!("https://github.com/{owner}/{name}.git"),
            api_url: "https://api.github.com".into(),
            owner,
            name,
        },
        base_branch: base,
        workdir: Default::default(),
        git_identity: identity,
        model: ModelSpec {
            provider: settings.provider_name.clone(),
            base_url: settings.base_url.clone(),
            model: settings.model.clone(),
            evaluator_model: cfg.cloud.evaluator_model.clone(),
            prompt_caching: settings.prompt_caching,
        },
        limits: Limits {
            max_runtime_secs,
            max_iterations: a.max_iterations.or(cfg.cloud.max_iterations).unwrap_or(20),
            budget_usd: a.budget_usd.or(cfg.cloud.budget_usd),
        },
        setup: None,
        sandbox_id: None,
        e2b_api_url: None,
    };
    let secrets = Secrets {
        model_api_key: settings.api_key.clone(),
        github_token: github_token.clone(),
        e2b_api_key: cfg
            .cloud
            .auto_shutdown
            .unwrap_or(true)
            .then(|| e2b.api_key.clone()),
    };

    let progress: mira_cloud::launcher::Progress = Arc::new(|m: String| eprintln!("cloud: {m}"));
    let record = Launch {
        spec: spec.clone(),
        secrets,
        environment,
        install: install_mode(cfg)?,
        worker_command: None,
    }
    .run(progress)
    .await
    .map_err(|e| anyhow!("{e}"))?;
    TaskStore::new(TaskStore::default_path())
        .add(record.clone())
        .map_err(|e| anyhow!("{e}"))?;

    println!(
        "\nCloud task {} is running in E2B sandbox {}.\n  branch  {} (from {})\n  limit   {} min, {} iterations\n  PR      opens as a draft on https://github.com/{}/pulls within a minute\n\n\
         You can close your laptop. Check on it with `mira cloud list` or `mira cloud logs {}`.",
        record.id,
        record.sandbox_id,
        record.branch,
        record.base_branch,
        spec.limits.max_runtime_secs / 60,
        spec.limits.max_iterations,
        record.repo,
        record.id,
    );
    Ok(())
}

fn e2b_opts(cfg: &MiraConfig, record_env: &str) -> Result<mira_compute::E2bOptions> {
    match EnvironmentSpec::resolve(record_env, &cfg.compute)
        .or_else(|_| EnvironmentSpec::resolve("e2b", &cfg.compute))
        .map_err(|e| anyhow!("{e}"))?
        .backend
    {
        BackendSpec::E2b(o) => Ok(o),
        BackendSpec::Scratch => bail!("environment `{record_env}` isn't E2B"),
    }
}

async fn list(cfg: &MiraConfig) -> Result<()> {
    let tasks = TaskStore::new(TaskStore::default_path())
        .load()
        .map_err(|e| anyhow!("{e}"))?;
    if tasks.is_empty() {
        println!("No cloud tasks yet. Start one with `mira cloud run \"<task>\"`.");
        return Ok(());
    }
    let token = github_token(cfg).ok();
    for t in tasks.iter().rev().take(20) {
        let running = match e2b_opts(cfg, &t.environment) {
            Ok(o) => E2bBackend::exists(&o, &t.sandbox_id).await.ok(),
            Err(_) => None,
        };
        let (owner, repo) = t.repo.split_once('/').unwrap_or((&t.repo, ""));
        let pr = match &token {
            Some(tok) => match mira_cloud::github::GitHub::new(&t.api_url, tok) {
                Ok(gh) => gh.find_pr(owner, repo, &t.branch).await.ok().flatten(),
                Err(_) => None,
            },
            None => None,
        };
        let state = match (&pr, running) {
            (Some(p), _) if p.merged_at.is_some() => "merged",
            (Some(p), _) if p.state == "closed" => "closed",
            (_, Some(true)) => "running",
            (Some(p), _) if !p.draft => "ready for review",
            (Some(_), _) => "finished (draft: needs a look)",
            (None, Some(false)) => "ended, no PR",
            (None, _) => "unknown",
        };
        let prompt: String = t.prompt.chars().take(60).collect();
        println!("{}  {state:<30} {prompt}", t.id);
        match pr {
            Some(p) => println!("          {}", p.html_url),
            None => println!("          branch {}", t.branch),
        }
    }
    Ok(())
}

async fn logs(cfg: &MiraConfig, id: &str, lines: usize) -> Result<()> {
    let t = TaskStore::new(TaskStore::default_path())
        .get(id)
        .map_err(|e| anyhow!("{e}"))?;
    let opts = e2b_opts(cfg, &t.environment)?;
    match mira_cloud::launcher::tail_log(opts, &t, lines).await {
        Ok(text) => println!("{text}"),
        Err(e) => bail!(
            "couldn't read the log ({e}). If the task has finished, its sandbox is gone; the \
             summary is on the pull request (`mira cloud list`)."
        ),
    }
    Ok(())
}

async fn stop(cfg: &MiraConfig, id: &str) -> Result<()> {
    let t = TaskStore::new(TaskStore::default_path())
        .get(id)
        .map_err(|e| anyhow!("{e}"))?;
    let opts = e2b_opts(cfg, &t.environment)?;
    E2bBackend::kill(&opts, &t.sandbox_id)
        .await
        .map_err(|e| anyhow!("{e}"))?;
    println!(
        "Stopped {}. Work pushed so far stays on {}.",
        t.id, t.branch
    );
    Ok(())
}

/// Inside the sandbox: read the brief and the secrets, do the task.
async fn worker(spec_path: &str, secrets_path: &str) -> Result<()> {
    let spec: TaskSpec = serde_json::from_slice(
        &std::fs::read(spec_path).with_context(|| format!("reading {spec_path}"))?,
    )
    .context("parsing the task spec")?;
    let secrets = Secrets::take(Path::new(secrets_path)).map_err(|e| anyhow!("{e}"))?;
    let provider = mira_ai::build_chat_provider(
        &spec.model.provider,
        spec.model.base_url.clone(),
        secrets.model_api_key.clone(),
        Vec::new(),
        spec.model.prompt_caching,
    )
    .context("building the model provider")?;
    println!("mira cloud worker: task {} · {}", spec.id, spec.prompt);
    let result = mira_cloud::worker::Worker {
        spec: spec.clone(),
        secrets: secrets.clone(),
        provider,
        log: mira_cloud::worker::stdout_log(),
    }
    .run()
    .await;
    if let Err(e) = &result {
        println!("worker failed: {e}");
    }
    mira_cloud::worker::shutdown_own_sandbox(&spec, &secrets).await;
    result.map(drop).map_err(|e| anyhow!("{e}"))
}
