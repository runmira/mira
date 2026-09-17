//! `mira goal` — set, clear, inspect, and resume the standing `/goal`
//! on the most recent session for the current folder.
//!
//! Operates on disk records via [`FileStore`] so terminal-only users
//! don't need `mira serve` running to drive a goal. The next chat or
//! serve session that resumes the mutated record picks up the goal
//! and enters the autonomous loop.
//!
//! Server-integration follow-up (out of scope for v0.1): when
//! `mira serve` is running, the CLI could POST `/api/goal` and the
//! server would apply the change to the live in-memory session
//! without waiting for a reload. Today the workflow is: set the
//! goal, then start (or restart) the chat / server.
//!
//! Four verbs:
//!
//!   `mira goal set "<contract>" [--max-iterations N] [--evaluator-model M]
//!                              [--verify SHELL] [--expected-exit N]
//!                              [--expect-stdout REGEX] [--verify-timeout SECS]
//!                              [--budget-tokens N] [--budget-usd D]`
//!   `mira goal clear`
//!   `mira goal status`
//!   `mira goal resume [--max-iterations N]`
//!
//! All verbs target the newest session for `cwd`. `status` on a
//! folder with no sessions yet prints an empty-state message rather
//! than erroring.

use std::path::PathBuf;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Subcommand};
use mira_harness::{FileStore, Goal, GoalStatus, SessionStore, VerifyCommand};

#[derive(Args, Debug, Clone)]
pub struct GoalArgs {
    #[command(subcommand)]
    action: GoalAction,
}

#[derive(Subcommand, Debug, Clone)]
enum GoalAction {
    /// Set (or replace) the goal on the most recent session for this folder.
    Set(SetArgs),
    /// Drop the current goal without ending the session.
    Clear,
    /// Print the current goal, its status, and the last evaluator reason.
    Status,
    /// Take the last terminal (Met / Impossible / Exhausted / NeedsUser)
    /// goal and re-arm it as Active with iterations reset to 0. Useful
    /// after Exhausted: "keep going, cap raised".
    Resume(ResumeArgs),
}

#[derive(Args, Debug, Clone)]
struct SetArgs {
    /// The goal contract — success criteria written for the evaluator.
    /// Quote it so multi-word shells don't split.
    condition: String,

    /// Hard iteration cap. Defaults to `mira_harness::DEFAULT_MAX_ITERATIONS`.
    #[arg(long)]
    max_iterations: Option<usize>,

    /// Pin the evaluator to a specific (usually cheaper) model. Falls
    /// back to the session's active model when unset.
    #[arg(long)]
    evaluator_model: Option<String>,

    /// Shell command that gates the Met verdict. Runs before the LLM
    /// evaluator every iteration; fail → NotMet with the tail output as
    /// the reason. Skips the LLM call entirely when it fails.
    #[arg(long)]
    verify: Option<String>,

    /// Exit code that means "goal met" for the verify command.
    /// Defaults to 0. Set to 1 for grep-style "no match" checks.
    #[arg(long, requires = "verify")]
    expected_exit: Option<i32>,

    /// Regex that must match the verify command's stdout for the check
    /// to pass. Applied on top of the exit-code check.
    #[arg(long, requires = "verify")]
    expect_stdout: Option<String>,

    /// Wall-clock cap on the verify command in seconds. Defaults to 300.
    #[arg(long, requires = "verify")]
    verify_timeout: Option<u64>,

    /// Hard token budget (input + output, summed across all rounds).
    /// Breach → Exhausted with a "budget exceeded" reason.
    #[arg(long)]
    budget_tokens: Option<u64>,

    /// Hard USD budget. Requires the active model to be in
    /// mira-ai's pricing table; unpriced models skip the check.
    #[arg(long)]
    budget_usd: Option<f64>,
}

#[derive(Args, Debug, Clone)]
struct ResumeArgs {
    /// Optional new iteration cap for the resumed goal — usually you
    /// bump it if the last run hit Exhausted.
    #[arg(long)]
    max_iterations: Option<usize>,
}

pub async fn run(_cli: &super::Cli, args: GoalArgs) -> Result<()> {
    let cwd = std::env::current_dir().context("read cwd")?;
    let store = FileStore::open_default().context("open ~/.mira/sessions")?;
    match args.action {
        GoalAction::Set(a) => set(&store, &cwd, a).await,
        GoalAction::Clear => clear(&store, &cwd).await,
        GoalAction::Status => status(&store, &cwd).await,
        GoalAction::Resume(a) => resume(&store, &cwd, a).await,
    }
}

async fn set(store: &FileStore, cwd: &PathBuf, args: SetArgs) -> Result<()> {
    let condition = args.condition.trim().to_owned();
    if condition.is_empty() {
        bail!("goal condition cannot be empty");
    }

    let mut record = latest_record(store, cwd).await?;

    let mut goal = Goal::new(condition);
    if let Some(n) = args.max_iterations {
        goal = goal.with_max_iterations(n);
    }
    goal = goal.with_evaluator_model(args.evaluator_model);
    if let Some(cmd) = args.verify {
        goal = goal.with_verify(Some(VerifyCommand {
            command: cmd,
            expected_exit: args.expected_exit,
            expect_stdout: args.expect_stdout,
            timeout_secs: args.verify_timeout,
        }));
    }
    goal = goal.with_budget_tokens(args.budget_tokens);
    goal = goal.with_budget_usd(args.budget_usd);

    record.goal = Some(goal.clone());
    store.save(&record).await.context("save session record")?;

    println!("goal set on session {}", record.id);
    print_goal_summary(&goal);
    println!();
    println!(
        "Next chat / `mira serve` session resumed from this record will \
         enter the autonomous loop."
    );
    Ok(())
}

async fn clear(store: &FileStore, cwd: &PathBuf) -> Result<()> {
    let mut record = latest_record(store, cwd).await?;
    if record.goal.is_none() {
        println!("no goal on session {} — nothing to clear", record.id);
        return Ok(());
    }
    record.goal = None;
    store.save(&record).await.context("save session record")?;
    println!("goal cleared on session {}", record.id);
    Ok(())
}

async fn status(store: &FileStore, cwd: &PathBuf) -> Result<()> {
    let record = match latest_record_opt(store, cwd).await? {
        Some(r) => r,
        None => {
            println!("no sessions yet in {}", cwd.display());
            return Ok(());
        }
    };
    match &record.goal {
        None => println!("session {}: no goal set", record.id),
        Some(g) => {
            println!("session {}", record.id);
            print_goal_summary(g);
        }
    }
    Ok(())
}

async fn resume(store: &FileStore, cwd: &PathBuf, args: ResumeArgs) -> Result<()> {
    let mut record = latest_record(store, cwd).await?;
    let mut goal = record
        .goal
        .clone()
        .ok_or_else(|| anyhow!("session {} has no goal to resume", record.id))?;
    // Only resume terminal goals — an Active goal is already running,
    // and re-arming it would silently reset the iteration counter.
    match goal.status {
        GoalStatus::Active => {
            bail!(
                "goal on session {} is already Active (iteration {}/{}). \
                 Nothing to resume.",
                record.id,
                goal.iterations,
                goal.max_iterations,
            );
        }
        GoalStatus::Cleared => {
            bail!(
                "goal on session {} was cleared. Use `mira goal set` to \
                 create a new one.",
                record.id
            );
        }
        _ => {} // Met, Impossible, NeedsUser, Exhausted — all resumable.
    }
    goal.status = GoalStatus::Active;
    goal.iterations = 0;
    goal.last_reason = None;
    if let Some(n) = args.max_iterations {
        goal.max_iterations = n.max(1);
    }
    record.goal = Some(goal.clone());
    store.save(&record).await.context("save session record")?;
    println!("goal resumed on session {}", record.id);
    print_goal_summary(&goal);
    Ok(())
}

async fn latest_record(store: &FileStore, cwd: &PathBuf) -> Result<mira_harness::SessionRecord> {
    latest_record_opt(store, cwd).await?.ok_or_else(|| {
        anyhow!(
            "no sessions yet in {}. Start one with `mira` or `mira serve` first.",
            cwd.display()
        )
    })
}

async fn latest_record_opt(
    store: &FileStore,
    cwd: &PathBuf,
) -> Result<Option<mira_harness::SessionRecord>> {
    // The SessionStore trait's methods are `async fn` — go through the
    // trait to reach them (FileStore itself has no direct copies).
    let list = SessionStore::list_recent(store, cwd, 1)
        .await
        .context("list sessions")?;
    Ok(list.into_iter().next())
}

fn print_goal_summary(goal: &Goal) {
    println!("  status:         {:?}", goal.status);
    println!(
        "  iterations:     {}/{}",
        goal.iterations, goal.max_iterations
    );
    println!("  contract:       {}", goal.condition);
    if let Some(m) = &goal.evaluator_model {
        println!("  evaluator:      {m}");
    }
    if let Some(v) = &goal.verify {
        println!("  verify:         {}", v.command);
        if let Some(e) = v.expected_exit {
            println!("    expected exit: {e}");
        }
        if let Some(re) = &v.expect_stdout {
            println!("    stdout regex:  {re}");
        }
        if let Some(t) = v.timeout_secs {
            println!("    timeout:       {t}s");
        }
    }
    if let Some(n) = goal.budget_tokens {
        println!("  budget_tokens:  {n}");
    }
    if let Some(d) = goal.budget_usd {
        println!("  budget_usd:     ${d:.2}");
    }
    if let Some(r) = &goal.last_reason {
        println!("  last reason:    {r}");
    }
}
