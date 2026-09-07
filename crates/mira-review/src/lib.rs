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

            let snippet = read_snippet(cwd, &f.file, f.line, 12);
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
    let system = "You are a senior code reviewer. You review diffs for correctness bugs, \
                  security issues, and serious design flaws. You do NOT report style nits, \
                  formatting, or subjective preferences. You return concrete, verifiable findings.";

    let user = format!(
        "Review the following unified diff. For each real issue you find, produce a JSON object with:\n\
         - severity: one of \"critical\", \"high\", \"medium\", \"low\"\n\
         - file:     path from the diff header (post-rename `b/…` path, without the `b/` prefix)\n\
         - line:     line number in the NEW file (integer), if applicable\n\
         - title:    one-line summary (<80 chars)\n\
         - explanation: 2-4 sentence rationale grounded in the diff\n\
         - suggested_fix: (optional) a concrete fix\n\n\
         Return ONLY a JSON array wrapped in a ```json code fence. Empty array if no findings.\n\
         Prefer FEWER, higher-confidence findings over a long list of maybes.\n\n\
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
    let system = "You are a hostile code-review verifier. Your job is to try HARD to disprove \
                  findings other reviewers have made. Prefer REJECT unless the code clearly \
                  has the exact issue described. Do not confirm speculation.";

    let finding_json = serde_json::to_string_pretty(f)?;
    let snippet_block = match snippet {
        Some(s) => format!("Actual current code at that location:\n```\n{s}\n```\n"),
        None => "(No source snippet available — could not open the referenced file.)\n".to_string(),
    };

    let user = format!(
        "A previous reviewer flagged this finding:\n\n\
         {finding_json}\n\n\
         {snippet_block}\n\
         Try to disprove it. Respond in exactly ONE line, in this format:\n\
         `REJECT: <reason>` — if the actual code does not have the issue described.\n\
         `CONFIRM: <reason>` — if you cannot disprove it after honest scrutiny."
    );

    let reply = complete(provider, model, system, &user).await?;
    let line = reply.trim().lines().next().unwrap_or("").trim();
    if let Some(rest) = line
        .strip_prefix("CONFIRM:")
        .or_else(|| line.strip_prefix("Confirm:"))
    {
        Ok(Verdict::Confirm(rest.trim().to_owned()))
    } else if let Some(rest) = line
        .strip_prefix("REJECT:")
        .or_else(|| line.strip_prefix("Reject:"))
    {
        Ok(Verdict::Reject(rest.trim().to_owned()))
    } else {
        // Malformed — safer to keep the finding than silently drop it.
        Ok(Verdict::Confirm(format!(
            "verifier unclear: {}",
            short(line, 80)
        )))
    }
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
        temperature: Some(0.1),
        max_tokens: Some(4096),
        // Review calls are structured/JSON-mode-ish — reasoning effort would
        // just add latency without materially improving finding quality.
        reasoning_effort: None,
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
