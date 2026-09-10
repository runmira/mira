//! Two-stage diff review.
//!
//! Stage 1 asks the model to produce findings from a diff. Stage 2 re-reads
//! the actual code at each finding's location and forces the model to try to
//! *disprove* the claim before we accept it. Cheap noise findings die here.
//!
//! This crate is transport-agnostic: callers pass in a [`mira_ai::ChatProvider`]
//! and a model id, and receive [`Finding`]s. `mira-cli` renders them to the
//! terminal; `mira-server` streams them to the browser.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use futures::StreamExt;
use mira_ai::{ChatEvent, ChatProvider, ChatRequest};
use mira_core::Message;
use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Self::Critical => "CRITICAL",
            Self::High => "HIGH",
            Self::Medium => "MEDIUM",
            Self::Low => "LOW",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Finding {
    pub severity: Severity,
    pub file: String,
    #[serde(default)]
    pub line: Option<u32>,
    pub title: String,
    pub explanation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_fix: Option<String>,
    /// Filled in during stage 2 with the verifier's confirming rationale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify_note: Option<String>,
}

/// Live progress the caller can surface as a spinner / progress bar.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Progress {
    Stage1Started {
        diff_lines: usize,
    },
    Stage1Completed {
        total_findings: usize,
    },
    Stage2Started {
        total: usize,
    },
    Stage2Item {
        /// 1-based index of the finding currently under re-verify.
        index: usize,
        total: usize,
        title: String,
        kept: Option<bool>,
    },
    Completed {
        kept: usize,
        dropped: usize,
    },
}

/// Sink for streaming [`Progress`] events. Callers implement this to fan
/// updates out over WS (server) or stderr (CLI).
#[async_trait::async_trait]
pub trait ProgressSink: Send + Sync {
    async fn emit(&self, event: Progress);
}

/// No-op sink for callers that don't care about progress.
pub struct NullProgress;

#[async_trait::async_trait]
impl ProgressSink for NullProgress {
    async fn emit(&self, _event: Progress) {}
}

/* ---------- top-level entry ---------- */

/// Run stage 1 (+ optional stage 2) on a diff. Returns the confirmed findings.
pub async fn review(
    provider: &dyn ChatProvider,
    model: &str,
    diff: &str,
    cwd: &Path,
    verify: bool,
    progress: &dyn ProgressSink,
) -> Result<Vec<Finding>> {
    let diff_lines = diff.lines().count();
    progress.emit(Progress::Stage1Started { diff_lines }).await;

    let mut findings = stage1_generate(provider, model, diff).await?;
    progress
        .emit(Progress::Stage1Completed {
            total_findings: findings.len(),
        })
        .await;

    if verify && !findings.is_empty() {
        let total = findings.len();
        progress.emit(Progress::Stage2Started { total }).await;

        let mut kept = Vec::with_capacity(total);
        for (i, f) in findings.into_iter().enumerate() {
            progress
                .emit(Progress::Stage2Item {
                    index: i + 1,
                    total,
                    title: f.title.clone(),
                    kept: None,
                })
                .await;

            // 30 lines of context each side — the 12-line default was too narrow
            // to disprove most "missing check" / "null deref" claims because the
            // guard often lives 15+ lines away (top of the function, or after
            // an early return). Wider snippet = better rejection precision.
            let snippet = read_snippet(cwd, &f.file, f.line, 30);
            let verdict = verify_one(provider, model, &f, snippet.as_deref()).await?;
            let was_kept = matches!(verdict, Verdict::Confirm(_));
            progress
                .emit(Progress::Stage2Item {
                    index: i + 1,
                    total,
                    title: f.title.clone(),
                    kept: Some(was_kept),
                })
                .await;

            if let Verdict::Confirm(reason) = verdict {
                let mut kf = f;
                kf.verify_note = Some(reason);
                kept.push(kf);
            }
        }
        let dropped = total - kept.len();
        progress
            .emit(Progress::Completed {
                kept: kept.len(),
                dropped,
            })
            .await;
        findings = kept;
    } else {
        progress
            .emit(Progress::Completed {
                kept: findings.len(),
                dropped: 0,
            })
            .await;
    }

    Ok(findings)
}

/* ---------- stage 1 ---------- */

pub async fn stage1_generate(
    provider: &dyn ChatProvider,
    model: &str,
    diff: &str,
) -> Result<Vec<Finding>> {
    let system = "You are a senior code reviewer doing a THOROUGH review of a diff. \
                  Your goal is HIGH RECALL — surface every plausible correctness, \
                  security, or reliability concern. A separate hostile-verification \
                  pass will drop the weak ones, so it is better to over-report than \
                  to miss a real bug.\n\n\
                  Never flag: style, formatting, naming preferences, or subjective \
                  taste. Focus only on bugs, security, and correctness.";

    let user = format!(
        "Review the unified diff below. Read every hunk carefully — do not skim.\n\n\
         Walk through this checklist explicitly. For EACH category, note whether \
         the diff introduces a risk of that class. Any 'yes' or 'maybe' becomes a \
         finding — you can lower severity if you're not sure, but do not silently \
         drop it.\n\n\
         1.  **Correctness / logic**: inverted conditions, wrong operator, off-by-one, \
             missing branch, wrong loop bound, swapped arguments, wrong return value.\n\
         2.  **Null / None / undefined / zero-value dereference**: any access that \
             assumes a value is present without checking.\n\
         3.  **Error handling**: swallowed errors, `unwrap`/`panic` on fallible ops, \
             lost error context, wrong recovery (retry when should fail-fast, etc).\n\
         4.  **Concurrency**: data races, missing locks, TOCTOU, deadlock, incorrect \
             use of async/await, sending non-Send data, cancellation safety.\n\
         5.  **Resource leaks**: unclosed files/sockets/handles, unbounded caches, \
             tasks spawned without join, subscriptions never dropped.\n\
         6.  **Security**: injection (SQL / command / path / template), auth or \
             authz bypass, unsafe deserialization, secret in log/response, missing \
             rate limit, weak crypto, TOCTOU on permission checks.\n\
         7.  **API contracts**: broke a caller invariant, changed a signature's \
             semantics, altered idempotency, changed nullability without callers \
             updated.\n\
         8.  **Edge cases**: empty input, single-element input, very large input, \
             negative numbers, integer overflow, unicode / surrogate pairs, \
             timezone / DST, leap seconds, path traversal, `..`.\n\
         9.  **State / lifecycle**: use-after-free / use-after-move, uninitialised \
             read, bad state-machine transition, forgotten cleanup on error paths.\n\
         10. **Regressions**: removed a guard, removed a test's precondition, \
             quietly changed default behaviour.\n\
         11. **Performance cliffs**: quadratic where linear expected, N+1 queries, \
             unbounded fan-out, sync work on the hot path.\n\
         12. **Missing tests for the risky part**: a subtle change with no new test \
             is a finding (severity: low or medium).\n\n\
         Response format:\n\
         First, write a short reasoning block (2-6 sentences) explaining what the \
         diff does and what you looked at. Then output a JSON array in a ```json \
         fence with one object per finding:\n\n\
         - severity: \"critical\" | \"high\" | \"medium\" | \"low\"\n\
         - file: path from the diff header (post-rename `b/…` without the `b/`)\n\
         - line: integer line number in the NEW file, when applicable\n\
         - title: <80 chars, one line, no period at the end\n\
         - explanation: 2-4 sentences, quote the specific lines you're worried about\n\
         - suggested_fix: optional, one line\n\n\
         Return `[]` inside the ```json fence ONLY if the diff is genuinely trivial \
         (rename, doc typo, dependency bump with no code shape change). Otherwise \
         you should almost always find something worth calling out — even a low-\
         severity one about a missing test or an edge case.\n\n\
         Diff:\n```diff\n{diff}\n```"
    );

    let reply = complete(provider, model, system, &user).await?;
    parse_findings(&reply)
}

/* ---------- stage 2 ---------- */

pub enum Verdict {
    Confirm(String),
    Reject(String),
}

pub async fn verify_one(
    provider: &dyn ChatProvider,
    model: &str,
    f: &Finding,
    snippet: Option<&str>,
) -> Result<Verdict> {
    let system = "You are a careful code-review verifier. Another reviewer flagged \
                  a potential issue. Your job is to look at the actual code and \
                  decide whether the finding is real.\n\n\
                  DEFAULT TO CONFIRM. Reject only when the code clearly does NOT \
                  have the issue described (e.g. the check the reviewer says is \
                  missing is right there, the variable they say is unchecked is \
                  provably non-null, the race they describe cannot happen with the \
                  visible synchronization).\n\n\
                  If you'd need more context to be sure — CONFIRM. If the finding \
                  is directionally right but slightly wrong on details — CONFIRM. \
                  If the code you can see is ambiguous — CONFIRM. A confirmed \
                  finding still gets human review; a rejected one is silently lost.";

    let finding_json = serde_json::to_string_pretty(f)?;
    let snippet_block = match snippet {
        Some(s) => format!(
            "Actual current code around the flagged location:\n```\n{s}\n```\n"
        ),
        None => "(No source snippet available — the file could not be opened. \
                 With no way to disprove, you should CONFIRM.)\n"
            .to_string(),
    };

    let user = format!(
        "Finding to verify:\n\n{finding_json}\n\n\
         {snippet_block}\n\
         Decide. Respond in exactly ONE line:\n\
         `CONFIRM: <one-sentence reason grounded in the code>` — the code has the \
             described issue, OR you can't rule it out from what you can see.\n\
         `REJECT: <one-sentence reason grounded in the code>` — the code clearly \
             does NOT have the issue (cite the specific line/check that disproves it)."
    );

    let reply = complete(provider, model, system, &user).await?;
    // Some models still ramble a sentence before the verdict, or use bold
    // markdown. Scan every line for the first CONFIRM: / REJECT: prefix
    // (case-insensitive) rather than only checking line 1 — that made
    // preamble-heavy replies silently fall through to the "unclear" branch.
    for raw in reply.lines() {
        let line = raw.trim().trim_start_matches(['*', '#', '>', '-', ' ']);
        let upper = line.to_uppercase();
        if let Some(rest) = upper.strip_prefix("CONFIRM:") {
            let reason = line[line.len() - rest.len()..].trim().to_owned();
            return Ok(Verdict::Confirm(reason));
        }
        if let Some(rest) = upper.strip_prefix("REJECT:") {
            let reason = line[line.len() - rest.len()..].trim().to_owned();
            return Ok(Verdict::Reject(reason));
        }
    }
    // Malformed — safer to keep the finding than silently drop it.
    let first_line = reply.trim().lines().next().unwrap_or("").trim();
    Ok(Verdict::Confirm(format!(
        "verifier unclear: {}",
        short(first_line, 80)
    )))
}

pub fn read_snippet(cwd: &Path, file: &str, line: Option<u32>, ctx: u32) -> Option<String> {
    let path: PathBuf = cwd.join(file);
    let content = std::fs::read_to_string(&path).ok()?;
    let lines: Vec<&str> = content.lines().collect();
    let (start, end) = match line {
        Some(n) => {
            let n = (n as usize).saturating_sub(1); // 0-indexed
            let start = n.saturating_sub(ctx as usize);
            let end = (n + ctx as usize + 1).min(lines.len());
            (start, end)
        }
        None => (0, lines.len().min(40)),
    };
    let mut out = String::new();
    for (i, l) in lines[start..end].iter().enumerate() {
        out.push_str(&format!("{:>5} | {}\n", start + i + 1, l));
    }
    Some(out)
}

/* ---------- provider glue ---------- */

/// One-shot non-streaming completion — consumes the stream, joins text deltas.
async fn complete(
    provider: &dyn ChatProvider,
    model: &str,
    system: &str,
    user: &str,
) -> Result<String> {
    let req = ChatRequest {
        model: model.to_owned(),
        messages: vec![Message::system(system), Message::user(user)],
        tools: Vec::new(),
        // Bumped from 0.1: near-deterministic sampling made the model default
        // to "return []" too often. 0.3 keeps replies grounded but lets it
        // consider more finding candidates. Verify pass drops the noise.
        temperature: Some(0.3),
        // Bumped from 4096 to fit the wider stage-1 prompt (CoT + JSON) on
        // larger diffs without truncating findings.
        max_tokens: Some(8192),
        reasoning_effort: None,
        response_format: None,
    };
    let mut stream = provider.stream(req).await.context("provider stream")?;
    let mut out = String::new();
    while let Some(ev) = stream.next().await {
        match ev.context("stream error")? {
            ChatEvent::TextDelta(t) => out.push_str(&t),
            ChatEvent::ToolCalls(_) => { /* review prompt has no tools */ }
            ChatEvent::Usage(_) => { /* review has its own accounting; ignore */ }
            ChatEvent::Done(_) => break,
        }
    }
    Ok(out)
}

/* ---------- parse ---------- */

pub fn parse_findings(reply: &str) -> Result<Vec<Finding>> {
    let json = extract_json_block(reply).unwrap_or(reply);
    let trimmed = json.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let parsed: Vec<Finding> = serde_json::from_str(trimmed)
        .with_context(|| format!("model returned unparseable JSON: {}", short(trimmed, 200)))?;
    Ok(parsed)
}

fn extract_json_block(s: &str) -> Option<&str> {
    if let Some(start) = s.find("```json") {
        let after = &s[start + "```json".len()..];
        if let Some(end) = after.find("```") {
            return Some(after[..end].trim());
        }
    }
    let start = s.find('[')?;
    let end = s.rfind(']')?;
    if end > start {
        Some(s[start..=end].trim())
    } else {
        None
    }
}

fn short(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("{}…", &s[..n])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fenced_json_findings() {
        let reply = r#"Here you go:

```json
[
  {
    "severity": "high",
    "file": "src/foo.rs",
    "line": 42,
    "title": "unbounded loop",
    "explanation": "The while-loop lacks a termination condition."
  }
]
```
"#;
        let out = parse_findings(reply).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].severity, Severity::High);
        assert_eq!(out[0].line, Some(42));
    }

    #[test]
    fn parses_bare_json_array() {
        let reply = r#"[{"severity":"low","file":"a.rs","title":"x","explanation":"y"}]"#;
        let out = parse_findings(reply).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].severity, Severity::Low);
    }

    #[test]
    fn empty_reply_is_no_findings() {
        assert!(parse_findings("").unwrap().is_empty());
        assert!(parse_findings("```json\n[]\n```").unwrap().is_empty());
    }
}
