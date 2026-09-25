//! `mira github`: respond to a GitHub Actions event.
//!
//! Runs inside a workflow (see `action.yml` at the repository root):
//!
//! - **Pull request opened / updated / ready:** review the diff and post
//!   the findings as one review with inline comments.
//! - **`@mira review` on a pull request:** the same, on demand.
//! - **`@mira <task>` on an issue:** do the task on a new branch and open
//!   a pull request (the cloud-task worker, run on the Actions runner).
//! - **`@mira <task>` on a pull request:** do it on a branch off the PR's
//!   branch and open a follow-up PR into it.
//!
//! Only people with write access (owners, members, collaborators by
//! default) can start a task, and comments from bots are ignored, so the
//! agent can't be triggered by strangers or by its own replies.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use mira_cloud::github::GitHub;
use mira_cloud::spec::{GitIdentity, Limits, ModelSpec, RepoSpec, Secrets, TaskSpec};
use mira_review::{Finding, Progress, ProgressSink};
use serde_json::{json, Value};

use crate::config::MiraConfig;

#[derive(clap::Args, Debug, Clone)]
pub struct GithubArgs {
    /// The event payload. Defaults to `$GITHUB_EVENT_PATH`.
    #[arg(long)]
    event_path: Option<PathBuf>,
    /// The event name (`pull_request`, `issue_comment`, …). Defaults to
    /// `$GITHUB_EVENT_NAME`.
    #[arg(long)]
    event_name: Option<String>,
    /// What people type to call Mira in a comment.
    #[arg(long, default_value = "@mira")]
    trigger: String,
    /// Author associations allowed to start tasks, comma-separated.
    #[arg(long, default_value = "OWNER,MEMBER,COLLABORATOR")]
    allow: String,
    /// Time limit for a task.
    #[arg(long, default_value_t = 30)]
    max_runtime_minutes: u64,
    /// Work → evaluate rounds for a task.
    #[arg(long, default_value_t = 20)]
    max_iterations: usize,
    /// Shell command run in the checkout before a task starts (e.g.
    /// `npm ci`).
    #[arg(long)]
    setup: Option<String>,
    /// Skip the review's second, verifying pass. Faster; noisier.
    #[arg(long)]
    no_verify: bool,
}

/// What an event asks for.
#[derive(Debug, PartialEq)]
enum Plan {
    Review {
        pr: u64,
    },
    Task {
        /// Issue or PR number the request came from.
        number: u64,
        /// Set when the request is on a pull request.
        on_pr: bool,
        /// Comment to react to, when the request is a comment.
        comment_id: Option<u64>,
        prompt: String,
    },
    Ignore(String),
}

pub async fn run(cli: &crate::Cli, args: GithubArgs) -> Result<()> {
    let event_name = args
        .event_name
        .clone()
        .or_else(|| std::env::var("GITHUB_EVENT_NAME").ok())
        .context("no event name: pass --event-name or run inside GitHub Actions")?;
    let event_path = args
        .event_path
        .clone()
        .or_else(|| std::env::var_os("GITHUB_EVENT_PATH").map(PathBuf::from))
        .context("no event payload: pass --event-path or run inside GitHub Actions")?;
    let event: Value = serde_json::from_slice(
        &std::fs::read(&event_path).with_context(|| format!("reading {}", event_path.display()))?,
    )
    .context("parsing the event payload")?;

    let allow: Vec<String> = args
        .allow
        .split(',')
        .map(|s| s.trim().to_uppercase())
        .filter(|s| !s.is_empty())
        .collect();
    let plan = plan(&event_name, &event, &args.trigger, &allow);
    if let Plan::Ignore(why) = &plan {
        println!("mira: nothing to do ({why})");
        return Ok(());
    }

    let (owner, repo) = event["repository"]["full_name"]
        .as_str()
        .and_then(|s| s.split_once('/'))
        .map(|(o, r)| (o.to_owned(), r.to_owned()))
        .context("the event has no repository")?;
    let token = std::env::var("GITHUB_TOKEN")
        .or_else(|_| std::env::var("GH_TOKEN"))
        .context("GITHUB_TOKEN isn't set")?;
    let api = std::env::var("GITHUB_API_URL").unwrap_or_else(|_| "https://api.github.com".into());
    let gh = GitHub::new(&api, &token).map_err(|e| anyhow!("{e}"))?;

    let workspace = std::env::var_os("GITHUB_WORKSPACE")
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir()?);
    let cfg = MiraConfig::load(&workspace).context("load config")?;
    mira_config::export_keys_to_env(&cfg);
    let settings = crate::resolve_settings(cli, &cfg)?;
    let provider = mira_ai::build_chat_provider(
        &settings.provider_name,
        settings.base_url.clone(),
        settings.api_key.clone(),
        settings.extra_headers.clone(),
        settings.prompt_caching,
    )
    .context("build provider")?;

    match plan {
        Plan::Review { pr } => {
            review(
                &gh,
                &owner,
                &repo,
                pr,
                &*provider,
                &settings.model,
                &workspace,
                &args,
            )
            .await
        }
        Plan::Task {
            number,
            on_pr,
            comment_id,
            prompt,
        } => {
            if let Some(id) = comment_id {
                // 👀 so people see it was picked up.
                let _ = gh
                    .post(
                        &format!("/repos/{owner}/{repo}/issues/comments/{id}/reactions"),
                        &json!({"content": "eyes"}),
                    )
                    .await;
            }
            let ctx = TaskCtx {
                gh: &gh,
                owner: &owner,
                repo: &repo,
                api: &api,
                token: &token,
                settings: &settings,
                provider,
                args: &args,
                event: &event,
            };
            let result = task(&ctx, number, on_pr, &prompt).await;
            if let Err(e) = &result {
                let _ = gh
                    .comment(
                        &owner,
                        &repo,
                        number,
                        &format!("Mira couldn't finish this: {e:#}"),
                    )
                    .await;
            }
            result
        }
        Plan::Ignore(_) => Ok(()),
    }
}

/* ---------- deciding what to do ---------- */

fn plan(event_name: &str, event: &Value, trigger: &str, allow: &[String]) -> Plan {
    let action = event["action"].as_str().unwrap_or("");
    match event_name {
        "pull_request" | "pull_request_target" => {
            if !matches!(
                action,
                "opened" | "reopened" | "synchronize" | "ready_for_review"
            ) {
                return Plan::Ignore(format!("pull_request {action}"));
            }
            if event["pull_request"]["draft"].as_bool() == Some(true) {
                return Plan::Ignore("draft pull request".into());
            }
            match event["pull_request"]["number"].as_u64() {
                Some(pr) => Plan::Review { pr },
                None => Plan::Ignore("no pull request number".into()),
            }
        }
        "issue_comment" | "pull_request_review_comment" => {
            if action != "created" {
                return Plan::Ignore(format!("comment {action}"));
            }
            let comment = &event["comment"];
            let on_pr = event_name == "pull_request_review_comment"
                || event["issue"].get("pull_request").is_some();
            let number = event["issue"]["number"]
                .as_u64()
                .or_else(|| event["pull_request"]["number"].as_u64());
            let Some(number) = number else {
                return Plan::Ignore("no issue or pull request number".into());
            };
            let Some(ask) = request_in(comment, trigger, allow) else {
                return Plan::Ignore(format!("no {trigger} request from an allowed author"));
            };
            if on_pr && is_review_request(&ask) {
                return Plan::Review { pr: number };
            }
            let mut prompt = String::new();
            if let Some(title) = event["issue"]["title"]
                .as_str()
                .or_else(|| event["pull_request"]["title"].as_str())
            {
                let kind = if on_pr { "Pull request" } else { "Issue" };
                prompt.push_str(&format!("{kind} #{number}: {title}\n\n"));
            }
            if let Some(body) = event["issue"]["body"]
                .as_str()
                .or_else(|| event["pull_request"]["body"].as_str())
                .filter(|b| !b.trim().is_empty())
            {
                prompt.push_str(&format!("{}\n\n", body.trim()));
            }
            if let (Some(path), Some(hunk)) =
                (comment["path"].as_str(), comment["diff_hunk"].as_str())
            {
                prompt.push_str(&format!(
                    "The request is a review comment on `{path}`{}:\n```diff\n{hunk}\n```\n\n",
                    comment["line"]
                        .as_u64()
                        .map(|l| format!(" line {l}"))
                        .unwrap_or_default()
                ));
            }
            let who = comment["user"]["login"].as_str().unwrap_or("someone");
            prompt.push_str(&format!("Request from @{who}:\n{ask}"));
            Plan::Task {
                number,
                on_pr,
                comment_id: comment["id"].as_u64(),
                prompt,
            }
        }
        "issues" => {
            if !matches!(action, "opened" | "edited") {
                return Plan::Ignore(format!("issues {action}"));
            }
            let issue = &event["issue"];
            let Some(ask) = request_in(issue, trigger, allow) else {
                return Plan::Ignore(format!("no {trigger} request from an allowed author"));
            };
            let Some(number) = issue["number"].as_u64() else {
                return Plan::Ignore("no issue number".into());
            };
            let title = issue["title"].as_str().unwrap_or("");
            Plan::Task {
                number,
                on_pr: false,
                comment_id: None,
                prompt: format!("Issue #{number}: {title}\n\n{ask}"),
            }
        }
        other => Plan::Ignore(format!("event {other} isn't handled")),
    }
}

/// The text after the trigger in a comment or issue body, when a person
/// allowed to ask wrote it.
fn request_in(item: &Value, trigger: &str, allow: &[String]) -> Option<String> {
    if item["user"]["type"].as_str() == Some("Bot") {
        return None;
    }
    let assoc = item["author_association"].as_str().unwrap_or("NONE");
    if !allow.iter().any(|a| a == assoc) {
        return None;
    }
    let body = item["body"].as_str()?;
    let at = find_trigger(body, trigger)?;
    let ask = body[at + trigger.len()..].trim();
    Some(if ask.is_empty() {
        "Please take care of this.".to_owned()
    } else {
        ask.to_owned()
    })
}

/// Where `trigger` appears as its own word (so `@mira` doesn't match
/// `@miranda`), case-insensitively.
fn find_trigger(body: &str, trigger: &str) -> Option<usize> {
    let lower = body.to_lowercase();
    let t = trigger.to_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(&t) {
        let at = from + i;
        let before_ok = at == 0 || !is_word(lower[..at].chars().next_back().unwrap());
        let after = lower[at + t.len()..].chars().next();
        if before_ok && !after.is_some_and(is_word) {
            return Some(at);
        }
        from = at + t.len();
    }
    None
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-'
}

fn is_review_request(ask: &str) -> bool {
    let first = ask.split_whitespace().next().unwrap_or("").to_lowercase();
    first.trim_end_matches(['.', '!', ':']) == "review"
}

/* ---------- review ---------- */

#[allow(clippy::too_many_arguments)]
async fn review(
    gh: &GitHub,
    owner: &str,
    repo: &str,
    pr: u64,
    provider: &dyn mira_ai::ChatProvider,
    model: &str,
    workspace: &std::path::Path,
    args: &GithubArgs,
) -> Result<()> {
    let info = gh
        .get(&format!("/repos/{owner}/{repo}/pulls/{pr}"))
        .await
        .map_err(|e| anyhow!("{e}"))?;
    let head_sha = info["head"]["sha"].as_str().unwrap_or_default().to_owned();
    let diff = gh
        .pr_diff(owner, repo, pr)
        .await
        .map_err(|e| anyhow!("{e}"))?;
    if diff.trim().is_empty() {
        println!("mira: empty diff, nothing to review");
        return Ok(());
    }
    println!(
        "mira: reviewing #{pr} ({} diff lines)",
        diff.lines().count()
    );
    let findings = mira_review::review(
        provider,
        model,
        &diff,
        workspace,
        !args.no_verify,
        &LogProgress,
    )
    .await?;
    println!("mira: {} finding(s)", findings.len());

    let commentable = commentable_lines(&diff);
    let (inline, general): (Vec<&Finding>, Vec<&Finding>) = findings.iter().partition(|f| {
        f.line.is_some_and(|l| {
            commentable
                .get(f.file.as_str())
                .is_some_and(|s| s.contains(&l))
        })
    });
    let comments: Vec<Value> = inline
        .iter()
        .map(|f| {
            json!({
                "path": f.file,
                "line": f.line,
                "side": "RIGHT",
                "body": finding_body(f),
            })
        })
        .collect();
    let mut body = if findings.is_empty() {
        "**Mira review:** no issues found.".to_owned()
    } else {
        format!(
            "**Mira review:** {} finding(s){}.",
            findings.len(),
            if inline.is_empty() {
                String::new()
            } else {
                format!(", {} inline", inline.len())
            }
        )
    };
    for f in &general {
        let at = match f.line {
            Some(l) => format!("`{}:{l}`", f.file),
            None => format!("`{}`", f.file),
        };
        body.push_str(&format!("\n\n---\n{at}\n\n{}", finding_body(f)));
    }
    let mut review = json!({"event": "COMMENT", "body": body, "comments": comments});
    if !head_sha.is_empty() {
        review["commit_id"] = json!(head_sha);
    }
    gh.post(
        &format!("/repos/{owner}/{repo}/pulls/{pr}/reviews"),
        &review,
    )
    .await
    .map_err(|e| anyhow!("posting the review: {e}"))?;
    Ok(())
}

fn finding_body(f: &Finding) -> String {
    let mut s = format!(
        "**{}: {}**\n\n{}",
        f.severity.label(),
        f.title,
        f.explanation
    );
    if let Some(fix) = f.suggested_fix.as_deref().filter(|x| !x.trim().is_empty()) {
        s.push_str(&format!("\n\n**Suggested fix:** {fix}"));
    }
    s
}

/// For each file, the new-side line numbers a review comment can attach
/// to: added and context lines inside the diff's hunks.
fn commentable_lines(diff: &str) -> BTreeMap<&str, BTreeSet<u32>> {
    let mut out: BTreeMap<&str, BTreeSet<u32>> = BTreeMap::new();
    let mut file: Option<&str> = None;
    let mut line = 0u32;
    let mut in_hunk = false;
    for l in diff.lines() {
        if let Some(path) = l.strip_prefix("+++ ") {
            file = path
                .strip_prefix("b/")
                .or(Some(path))
                .filter(|p| *p != "/dev/null");
            in_hunk = false;
        } else if l.starts_with("diff --git ") {
            file = None;
            in_hunk = false;
        } else if let Some(rest) = l.strip_prefix("@@ ") {
            // `@@ -a,b +c,d @@`
            line = rest
                .split_whitespace()
                .find_map(|p| p.strip_prefix('+'))
                .and_then(|p| p.split(',').next())
                .and_then(|n| n.parse().ok())
                .unwrap_or(0);
            in_hunk = true;
        } else if in_hunk {
            let Some(f) = file else { continue };
            match l.chars().next() {
                Some('+') | Some(' ') => {
                    out.entry(f).or_default().insert(line);
                    line += 1;
                }
                Some('-') | Some('\\') => {}
                _ => in_hunk = false,
            }
        }
    }
    out
}

struct LogProgress;

#[async_trait::async_trait]
impl ProgressSink for LogProgress {
    async fn emit(&self, event: Progress) {
        if let Ok(s) = serde_json::to_string(&event) {
            println!("mira: {s}");
        }
    }
}

/* ---------- tasks ---------- */

struct TaskCtx<'a> {
    gh: &'a GitHub,
    owner: &'a str,
    repo: &'a str,
    api: &'a str,
    token: &'a str,
    settings: &'a crate::ResolvedSettings,
    provider: Arc<dyn mira_ai::ChatProvider>,
    args: &'a GithubArgs,
    event: &'a Value,
}

async fn task(ctx: &TaskCtx<'_>, number: u64, on_pr: bool, prompt: &str) -> Result<()> {
    let (owner, repo) = (ctx.owner, ctx.repo);
    let base_branch = if on_pr {
        let pr = ctx
            .gh
            .get(&format!("/repos/{owner}/{repo}/pulls/{number}"))
            .await
            .map_err(|e| anyhow!("{e}"))?;
        if pr["head"]["repo"]["full_name"] != pr["base"]["repo"]["full_name"] {
            bail!("this pull request comes from a fork, and Mira can only push to this repository");
        }
        pr["head"]["ref"]
            .as_str()
            .context("the pull request has no head branch")?
            .to_owned()
    } else {
        ctx.event["repository"]["default_branch"]
            .as_str()
            .unwrap_or("main")
            .to_owned()
    };

    let id = mira_cloud::spec::new_task_id();
    let server = std::env::var("GITHUB_SERVER_URL").unwrap_or_else(|_| "https://github.com".into());
    let temp = std::env::var_os("RUNNER_TEMP")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let spec = TaskSpec {
        id: id.clone(),
        prompt: prompt.to_owned(),
        repo: RepoSpec {
            owner: owner.to_owned(),
            name: repo.to_owned(),
            clone_url: format!("{server}/{owner}/{repo}.git"),
            api_url: ctx.api.to_owned(),
        },
        base_branch: base_branch.clone(),
        branch: mira_cloud::spec::branch_name(prompt.lines().last().unwrap_or(prompt), &id),
        workdir: temp.join(format!("mira-{id}")),
        git_identity: GitIdentity {
            name: "github-actions[bot]".into(),
            email: "41898282+github-actions[bot]@users.noreply.github.com".into(),
        },
        model: ModelSpec {
            provider: ctx.settings.provider_name.clone(),
            base_url: ctx.settings.base_url.clone(),
            model: ctx.settings.model.clone(),
            evaluator_model: None,
            prompt_caching: ctx.settings.prompt_caching,
        },
        limits: Limits {
            max_runtime_secs: ctx.args.max_runtime_minutes.max(2) * 60,
            max_iterations: ctx.args.max_iterations.max(1),
            budget_usd: None,
        },
        setup: ctx.args.setup.clone(),
        sandbox_id: None,
        e2b_api_url: None,
    };
    println!("mira: task {id} on {owner}/{repo} from {base_branch}");
    let outcome = mira_cloud::worker::Worker {
        spec,
        secrets: Secrets {
            model_api_key: ctx.settings.api_key.clone(),
            github_token: ctx.token.to_owned(),
            e2b_api_key: None,
        },
        provider: ctx.provider.clone(),
        log: mira_cloud::worker::stdout_log(),
    }
    .run()
    .await
    .map_err(|e| anyhow!("{e}"))?;

    let reply = match &outcome.pr {
        Some(pr) => format!("Mira opened {} ({}).", pr.html_url, outcome.status.label()),
        None => format!("Mira finished: {}.", outcome.status.label()),
    };
    let _ = ctx.gh.comment(owner, repo, number, &reply).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allow() -> Vec<String> {
        vec!["OWNER".into(), "MEMBER".into(), "COLLABORATOR".into()]
    }

    fn comment(body: &str, assoc: &str, on_pr: bool) -> Value {
        let mut issue = json!({"number": 7, "title": "Crash on empty input", "body": "It panics."});
        if on_pr {
            issue["pull_request"] = json!({"url": "…"});
        }
        json!({
            "action": "created",
            "issue": issue,
            "comment": {"id": 99, "body": body, "author_association": assoc,
                        "user": {"login": "dami", "type": "User"}},
            "repository": {"full_name": "o/r", "default_branch": "main"},
        })
    }

    #[test]
    fn comments_become_tasks_or_reviews() {
        match plan(
            "issue_comment",
            &comment("@mira fix it please", "OWNER", false),
            "@mira",
            &allow(),
        ) {
            Plan::Task {
                number,
                on_pr,
                comment_id,
                prompt,
            } => {
                assert_eq!((number, on_pr, comment_id), (7, false, Some(99)));
                assert!(prompt.contains("Issue #7: Crash on empty input"));
                assert!(prompt.contains("It panics."));
                assert!(prompt.ends_with("Request from @dami:\nfix it please"));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            plan(
                "issue_comment",
                &comment("@Mira review", "MEMBER", true),
                "@mira",
                &allow()
            ),
            Plan::Review { pr: 7 }
        );
        assert!(matches!(
            plan(
                "issue_comment",
                &comment("@mira add tests", "COLLABORATOR", true),
                "@mira",
                &allow()
            ),
            Plan::Task { on_pr: true, .. }
        ));
    }

    #[test]
    fn strangers_bots_and_lookalikes_are_ignored() {
        let ignored =
            |v: &Value| matches!(plan("issue_comment", v, "@mira", &allow()), Plan::Ignore(_));
        assert!(ignored(&comment("@mira delete everything", "NONE", false)));
        assert!(ignored(&comment("hey @miranda", "OWNER", false)));
        assert!(ignored(&comment("no mention here", "OWNER", false)));
        let mut bot = comment("@mira loop", "OWNER", false);
        bot["comment"]["user"]["type"] = json!("Bot");
        assert!(ignored(&bot));
    }

    #[test]
    fn pull_requests_are_reviewed_unless_draft() {
        let pr = |draft: bool| json!({"action": "opened", "pull_request": {"number": 3, "draft": draft}});
        assert_eq!(
            plan("pull_request", &pr(false), "@mira", &allow()),
            Plan::Review { pr: 3 }
        );
        assert!(matches!(
            plan("pull_request", &pr(true), "@mira", &allow()),
            Plan::Ignore(_)
        ));
    }

    #[test]
    fn finds_lines_a_review_can_comment_on() {
        let diff = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,3 +1,4 @@\n fn a() {\n-    old();\n+    new();\n+    more();\n }\n@@ -20,2 +21,2 @@\n x\n-y\n+z\ndiff --git a/gone.rs b/gone.rs\n--- a/gone.rs\n+++ /dev/null\n@@ -1 +0,0 @@\n-bye\n";
        let lines = commentable_lines(diff);
        assert_eq!(
            lines["src/a.rs"].iter().copied().collect::<Vec<_>>(),
            vec![1, 2, 3, 4, 21, 22]
        );
        assert!(!lines.contains_key("gone.rs"));
    }
}
