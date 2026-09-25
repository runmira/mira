//! The worker: runs inside the sandbox, turns a [`TaskSpec`] into a pull
//! request, and needs nobody watching.
//!
//! 1. Clone the repo, branch off, push a start commit, open a **draft**
//!    PR, so progress is visible on GitHub from minute one.
//! 2. Run a headless Mira session in `/goal` mode: the model works, an
//!    evaluator checks the result, and the loop continues until the goal
//!    is met or a limit hits. `plan` / `ask_user` are answered
//!    automatically, since there's no human to wait for.
//! 3. After every iteration, commit, push, and refresh the PR description.
//! 4. Stop before the time limit, push, post a summary, and mark the PR
//!    ready for review if the evaluator judged the goal met.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::StreamExt;
use mira_ai::ChatProvider;
use mira_core::Role;
use mira_harness::{AutoApprover, Goal, GoalStatus, HarnessEvent, Session, SessionConfig};
use mira_policy::{Mode, Policy, PolicyConfig};
use mira_tools::prompt::{
    AskUserAnswer, AskUserResponse, PlanResponse, PromptChannel, PromptRequest, PromptResponse,
};
use mira_tools::{Registry, ToolContext};
use tokio::sync::Mutex;

use crate::git::Git;
use crate::github::{GitHub, PullRequest};
use crate::spec::{Secrets, TaskSpec};
use crate::CloudError;

/// How the task ended.
#[derive(Clone, Debug, PartialEq)]
pub enum TaskStatus {
    /// The evaluator judged the goal met; the PR is ready for review.
    Done,
    /// Needs a human decision or credential; PR left as draft.
    NeedsUser(String),
    /// The evaluator judged it impossible; PR left as draft.
    Impossible(String),
    /// Iteration or spend limit reached.
    Exhausted(String),
    /// Ran out of wall-clock time.
    TimedOut,
    /// Something broke (setup, provider, git).
    Failed(String),
}

impl TaskStatus {
    pub fn label(&self) -> &'static str {
        match self {
            TaskStatus::Done => "done",
            TaskStatus::NeedsUser(_) => "needs your input",
            TaskStatus::Impossible(_) => "stopped: not possible as specified",
            TaskStatus::Exhausted(_) => "stopped: limit reached",
            TaskStatus::TimedOut => "stopped: time limit",
            TaskStatus::Failed(_) => "failed",
        }
    }

    fn detail(&self) -> Option<&str> {
        match self {
            TaskStatus::NeedsUser(r)
            | TaskStatus::Impossible(r)
            | TaskStatus::Exhausted(r)
            | TaskStatus::Failed(r) => Some(r),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Outcome {
    pub status: TaskStatus,
    pub pr: Option<PullRequest>,
    pub iterations: usize,
}

/// Answers `plan` / `ask_user` without a human: plans are approved, and
/// questions get the model's recommended option (or an instruction to
/// pick one and say so in the summary).
pub struct HeadlessPrompts;

const NO_HUMAN: &str = "No one is available to answer: this is an unattended cloud task. \
     Choose the option you'd recommend, proceed, and list this assumption in your final summary.";

#[async_trait]
impl PromptChannel for HeadlessPrompts {
    async fn ask(&self, _prompt_id: String, request: PromptRequest) -> Option<PromptResponse> {
        Some(match request {
            PromptRequest::Plan(_) => PromptResponse::Plan(PlanResponse {
                approved: true,
                steps: None,
                note: Some("Auto-approved: unattended cloud task.".into()),
            }),
            PromptRequest::AskUser(p) => PromptResponse::AskUser(AskUserResponse {
                answers: p
                    .questions
                    .iter()
                    .map(|q| match q.options.iter().find(|o| o.recommended) {
                        Some(o) => AskUserAnswer {
                            picked: vec![o.label.clone()],
                            custom: None,
                        },
                        None => AskUserAnswer {
                            picked: vec![],
                            custom: Some(NO_HUMAN.into()),
                        },
                    })
                    .collect(),
                cancelled: false,
            }),
        })
    }
}

fn system_prompt(spec: &TaskSpec, registry: &Registry) -> String {
    let tools: Vec<String> = registry
        .specs()
        .into_iter()
        .map(|s| {
            let first = s
                .description
                .split(". ")
                .next()
                .unwrap_or("")
                .trim_end_matches('.');
            format!("- {}: {first}.", s.name)
        })
        .collect();
    format!(
        "You are Mira, an autonomous coding agent running as an unattended cloud task.\n\
         Repository: {owner}/{name}, checked out at {dir} on branch `{branch}` (from `{base}`).\n\n\
         Available tools:\n{tools}\n\n\
         How this works:\n\
         - Nobody is watching live and nobody will answer questions. Make reasonable decisions \
           and record your assumptions; don't stop to ask.\n\
         - The runner commits and pushes your changes and maintains the pull request. Do NOT \
           run `git commit`, `git push`, create branches, or open pull requests yourself.\n\
         - Verify your work the way a careful engineer would: build, run the relevant tests, \
           lint. Don't claim something works without checking.\n\
         - Keep changes focused on the task.\n\
         - When you're done, finish with a concise summary for the pull request: what changed \
           and why, how you verified it, and any assumptions or follow-ups.",
        owner = spec.repo.owner,
        name = spec.repo.name,
        dir = spec.workdir.display(),
        branch = spec.branch,
        base = spec.base_branch,
        tools = tools.join("\n"),
    )
}

fn pr_title(prompt: &str) -> String {
    let first = prompt.lines().next().unwrap_or("Mira task").trim();
    let mut t: String = first.chars().take(72).collect();
    if first.chars().count() > 72 {
        t.push('…');
    }
    t
}

struct Progress {
    lines: Vec<String>,
    status: String,
}

fn pr_body(spec: &TaskSpec, p: &Progress) -> String {
    let mut body = format!(
        "> **Mira cloud task** `{}` · status: **{}**\n\n### Task\n\n{}\n",
        spec.id, p.status, spec.prompt
    );
    if !p.lines.is_empty() {
        body.push_str("\n### Progress\n\n");
        for l in &p.lines {
            body.push_str(&format!("- {l}\n"));
        }
    }
    body.push_str("\n<sub>This description is updated automatically while the task runs.</sub>\n");
    body
}

fn one_line(s: &str, max: usize) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        flat
    } else {
        format!("{}…", flat.chars().take(max).collect::<String>())
    }
}

/// Log sink: the launcher redirects the worker's stdout to the task log.
pub type Log = Arc<dyn Fn(String) + Send + Sync>;

pub fn stdout_log() -> Log {
    Arc::new(|line: String| println!("{line}"))
}

pub struct Worker {
    pub spec: TaskSpec,
    pub secrets: Secrets,
    pub provider: Arc<dyn ChatProvider>,
    pub log: Log,
}

impl Worker {
    /// Run the task to completion. Always tries to leave the PR in an
    /// accurate final state, even when something fails midway.
    pub async fn run(mut self) -> Result<Outcome, CloudError> {
        let started = Instant::now();
        if self.spec.workdir.is_relative() {
            self.spec.workdir = std::env::current_dir()?.join(&self.spec.workdir);
        }
        let limit = Duration::from_secs(self.spec.limits.max_runtime_secs.max(60));
        // Leave time to commit, push and report before the sandbox dies.
        let margin = (limit / 5).min(Duration::from_secs(300));
        let deadline = tokio::time::Instant::from_std(started + limit - margin);

        let log = self.log.clone();
        let spec = &self.spec;
        let gh = GitHub::new(&spec.repo.api_url, &self.secrets.github_token)?;
        let git = Git::new(
            &spec.workdir,
            &self.secrets.github_token,
            spec.git_identity.clone(),
        );

        log(format!(
            "cloning {}/{} ({})…",
            spec.repo.owner, spec.repo.name, spec.base_branch
        ));
        git.clone_and_branch(&spec.repo.clone_url, &spec.base_branch, &spec.branch)
            .await?;
        let base_ref = git.head().await?;
        if let Some(script) = &spec.setup {
            log("running the environment's setup script…".into());
            run_setup(&spec.workdir, script, &log).await?;
        }
        git.commit_empty(&format!("Start Mira task: {}", pr_title(&spec.prompt)))
            .await?;
        git.push(&spec.branch).await?;
        let progress = Mutex::new(Progress {
            lines: Vec::new(),
            status: "working".into(),
        });
        let pr = gh
            .create_draft_pr(
                &spec.repo.owner,
                &spec.repo.name,
                &spec.branch,
                &spec.base_branch,
                &pr_title(&spec.prompt),
                &pr_body(spec, &*progress.lock().await),
            )
            .await?;
        log(format!("draft pull request: {}", pr.html_url));

        let result = self.work(&git, &gh, &pr, &progress, deadline).await;
        let (status, iterations, summary) = match result {
            Ok(v) => v,
            Err(e) => (TaskStatus::Failed(e.to_string()), 0, String::new()),
        };

        // Deliver whatever exists, whatever happened.
        if let Err(e) = async {
            git.commit_all(&format!("Mira: {}", status.label())).await?;
            git.push(&spec.branch).await
        }
        .await
        {
            log(format!("final push failed: {e}"));
        }
        let stat = git.stat_since(&base_ref).await.unwrap_or_default();
        let runtime_min = started.elapsed().as_secs() / 60;
        let mut report = String::new();
        if !summary.trim().is_empty() {
            report.push_str(&format!("## Summary\n\n{}\n\n", summary.trim()));
        }
        if !stat.trim().is_empty() {
            report.push_str(&format!("## Changes\n\n```\n{}\n```\n\n", stat.trim_end()));
        } else {
            report.push_str("No files were changed.\n\n");
        }
        report.push_str(&format!(
            "**Status:** {}{} · **Iterations:** {iterations} · **Runtime:** {runtime_min} min\n",
            status.label(),
            status
                .detail()
                .map(|d| format!(" ({})", one_line(d, 200)))
                .unwrap_or_default(),
        ));
        {
            let mut p = progress.lock().await;
            p.status = status.label().into();
            let _ = gh
                .update_body(
                    &spec.repo.owner,
                    &spec.repo.name,
                    pr.number,
                    &pr_body(spec, &p),
                )
                .await;
        }
        if let Err(e) = gh
            .comment(&spec.repo.owner, &spec.repo.name, pr.number, &report)
            .await
        {
            log(format!("posting the summary failed: {e}"));
        }
        if status == TaskStatus::Done && !stat.trim().is_empty() {
            if let Err(e) = gh.mark_ready(&pr.node_id).await {
                log(format!("marking ready failed: {e}"));
            }
        }
        log(format!("finished: {} · {}", status.label(), pr.html_url));
        Ok(Outcome {
            status,
            pr: Some(pr),
            iterations,
        })
    }

    async fn work(
        &self,
        git: &Git,
        gh: &GitHub,
        pr: &PullRequest,
        progress: &Mutex<Progress>,
        deadline: tokio::time::Instant,
    ) -> Result<(TaskStatus, usize, String), CloudError> {
        let spec = &self.spec;
        let log = self.log.clone();
        let mut registry = Registry::new();
        mira_tools::builtin::register_core(&mut registry);
        let prompts: Arc<dyn PromptChannel> = Arc::new(HeadlessPrompts);
        registry.register(mira_tools::prompt::PlanTool::new(prompts.clone()));
        registry.register(mira_tools::prompt::AskUserTool::new(prompts));
        let system = system_prompt(spec, &registry);

        let policy = Policy::from_config(&PolicyConfig {
            mode: Mode::Yolo,
            ..Default::default()
        })
        .map_err(|e| CloudError::Config(e.to_string()))?;
        let ctx = ToolContext::new(
            spec.workdir.clone(),
            Arc::new(mira_sandbox::Sandbox::new(&spec.workdir)),
        );
        let session = Session::new(
            SessionConfig::new(spec.model.model.clone()),
            system,
            self.provider.clone(),
            Arc::new(registry),
            Arc::new(Mutex::new(policy)),
            Arc::new(AutoApprover { approve_asks: true }),
            ctx,
        );
        let goal = Goal::new(spec.prompt.clone())
            .with_max_iterations(spec.limits.max_iterations.max(1))
            .with_evaluator_model(spec.model.evaluator_model.clone())
            .with_budget_usd(spec.limits.budget_usd);
        session.set_goal(goal).await;

        let kickoff = format!(
            "Your task:\n\n{}\n\nStart working on it now. Plan briefly if it helps, then make the \
             changes and verify them.",
            spec.prompt
        );
        let mut events = session.send(kickoff).await;
        let mut status: Option<TaskStatus> = None;
        let mut iterations = 0;
        loop {
            let evt = tokio::select! {
                e = events.next() => e,
                _ = tokio::time::sleep_until(deadline) => {
                    log("time limit reached; stopping to deliver the work".into());
                    session.cancel().await;
                    status = Some(TaskStatus::TimedOut);
                    break;
                }
            };
            let Some(evt) = evt else { break };
            match evt {
                HarnessEvent::ToolStart(call) => {
                    log(format!(
                        "tool: {} {}",
                        call.function.name,
                        one_line(&call.function.arguments, 160)
                    ));
                }
                HarnessEvent::Warning(w) => log(format!("warning: {w}")),
                HarnessEvent::GoalProgress {
                    iteration,
                    max_iterations,
                    reason,
                    ..
                } => {
                    iterations = iteration;
                    let note = reason
                        .as_deref()
                        .map(|r| one_line(r, 160))
                        .unwrap_or_default();
                    log(format!("iteration {iteration}/{max_iterations}: {note}"));
                    let msg = format!("Mira: iteration {iteration}\n\n{note}");
                    match git.commit_all(&msg).await {
                        Ok(true) => {
                            if let Err(e) = git.push(&spec.branch).await {
                                log(format!("push failed: {e}"));
                            }
                        }
                        Ok(false) => {}
                        Err(e) => log(format!("commit failed: {e}")),
                    }
                    let mut p = progress.lock().await;
                    p.lines.push(format!("Iteration {iteration}: {note}"));
                    p.status = format!("working (iteration {iteration}/{max_iterations})");
                    let _ = gh
                        .update_body(
                            &spec.repo.owner,
                            &spec.repo.name,
                            pr.number,
                            &pr_body(spec, &p),
                        )
                        .await;
                }
                HarnessEvent::GoalDone { status: s, reason } => {
                    let reason = reason.unwrap_or_default();
                    status = Some(match s {
                        GoalStatus::Met => TaskStatus::Done,
                        GoalStatus::NeedsUser => TaskStatus::NeedsUser(reason),
                        GoalStatus::Impossible => TaskStatus::Impossible(reason),
                        GoalStatus::Cleared => TaskStatus::Failed("goal cleared".into()),
                        _ => TaskStatus::Exhausted(reason),
                    });
                }
                _ => {}
            }
        }
        let summary = session
            .history()
            .await
            .into_iter()
            .rev()
            .find(|m| {
                m.role == Role::Assistant
                    && m.content.as_deref().is_some_and(|c| !c.trim().is_empty())
            })
            .and_then(|m| m.content)
            .unwrap_or_default();
        let status = status.unwrap_or_else(|| {
            TaskStatus::Failed("the session ended without a verdict (see the task log)".into())
        });
        Ok((status, iterations, summary))
    }
}

async fn run_setup(dir: &std::path::Path, script: &str, log: &Log) -> Result<(), CloudError> {
    let out = tokio::process::Command::new("bash")
        .args(["-lc", script])
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .output()
        .await?;
    for line in String::from_utf8_lossy(&out.stdout)
        .lines()
        .chain(String::from_utf8_lossy(&out.stderr).lines())
    {
        log(format!("  setup: {line}"));
    }
    if !out.status.success() {
        return Err(CloudError::Config(format!(
            "setup script failed ({})",
            out.status
        )));
    }
    Ok(())
}

/// Delete the sandbox this worker runs in (best effort), so it stops
/// billing as soon as the task is delivered.
pub async fn shutdown_own_sandbox(spec: &TaskSpec, secrets: &Secrets) {
    let (Some(id), Some(key)) = (&spec.sandbox_id, &secrets.e2b_api_key) else {
        return;
    };
    let mut opts = mira_compute::E2bOptions::new(key.clone());
    if let Some(url) = &spec.e2b_api_url {
        opts.api_url = url.clone();
    }
    let _ = mira_compute::E2bBackend::kill(&opts, id).await;
}
