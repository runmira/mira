//! Session-scoped goal loop.
//!
//! A `/goal` turns Mira from turn-based ("model stops, we wait") into
//! goal-directed ("keep going until an external evaluator agrees the
//! condition is met"). The mechanism is intentionally boring:
//!
//! 1. The user sets a goal contract (`Goal::condition`) — usually the
//!    acceptance criteria + a verifiable exit signal.
//! 2. The harness runs the normal per-turn loop.
//! 3. On clean stop it calls a **separate** LLM evaluator ([`evaluate`])
//!    that reads the recent transcript and returns one of
//!    `met | not_met | impossible | needs_user`.
//! 4. On `not_met` the harness synthesizes a "continue toward the goal"
//!    user message and re-enters the round loop.
//! 5. Any other verdict terminates the goal (and the send stream).
//!
//! The evaluator is a **different provider call** than the main model —
//! same provider handle but with `response_format: json_schema` and a
//! tight system prompt. That keeps the "did we finish?" question outside
//! the main model's own judgement, which the industry consensus flags as
//! the biggest way agents lie their way out of a goal.

use std::time::Duration;

use mira_ai::{ChatEvent, ChatProvider, ChatRequest, ResponseFormat};
use mira_core::{Message, Role};
use serde::{Deserialize, Serialize};
use tokio_stream::StreamExt;
use tracing::warn;

/// The user's standing objective. Persisted with the session so a resume
/// picks up where autonomy left off. Once `status` moves off `Active`
/// the goal is considered terminal — the harness won't loop again on
/// this record; the user has to set a new one.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Goal {
    /// The full contract the user wrote. Feeds both the "continue toward"
    /// synthetic message and the evaluator's prompt.
    pub condition: String,
    /// Terminal-or-active status. See [`GoalStatus`].
    pub status: GoalStatus,
    /// How many `not_met → continue` iterations have been consumed.
    /// Bumped by the harness right before the synthetic continuation
    /// message is injected.
    pub iterations: usize,
    /// Hard cap on iterations to prevent runaway spend. Configurable
    /// per goal at set-time; defaults to [`DEFAULT_MAX_ITERATIONS`].
    pub max_iterations: usize,
    /// UNIX seconds. Recorded once at goal set-time; not updated on
    /// resume.
    pub created_at: u64,
    /// The evaluator's most recent free-text reason. Rendered in the UI
    /// so the user can see "why the loop is still going" without
    /// scrolling the transcript. `None` before the first evaluation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_reason: Option<String>,
    /// Optional evaluator model override. Falls back to the session's
    /// active model when `None`. Set this to a cheap tier
    /// (`gpt-4o-mini`, `claude-haiku`, …) to keep long autonomous runs
    /// affordable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluator_model: Option<String>,
}

/// Terminal-or-active status. Once a goal leaves `Active` the harness
/// treats the record as done — it neither loops nor evaluates again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    /// Loop still running. Set on goal-create and after every `not_met`.
    Active,
    /// Evaluator judged the acceptance criteria satisfied.
    Met,
    /// Evaluator declared the goal fundamentally unachievable (bad
    /// contract, hostile environment, model incapacity).
    Impossible,
    /// Evaluator says only the user can move the needle (missing
    /// credential, human approval, decision).
    NeedsUser,
    /// User cleared the goal manually via `/goal clear` or the UI.
    Cleared,
    /// Ran out of `max_iterations` before meeting the condition. The
    /// last transcript is left intact; the user can raise the cap and
    /// re-set the goal to continue.
    Exhausted,
}

impl GoalStatus {
    /// True when the goal is still driving the loop. False for any
    /// terminal state.
    pub fn is_active(self) -> bool {
        matches!(self, Self::Active)
    }
}

/// Cap on autonomous iterations before the harness gives up and hands
/// control back with `Exhausted`. Chosen conservatively — most useful
/// goals should terminate well under this in practice, and a runaway
/// with an unmet condition is the failure mode we're guarding against.
pub const DEFAULT_MAX_ITERATIONS: usize = 20;

impl Goal {
    /// Fresh goal at `Active`, iteration 0, `created_at = now`.
    pub fn new(condition: impl Into<String>) -> Self {
        Self {
            condition: condition.into(),
            status: GoalStatus::Active,
            iterations: 0,
            max_iterations: DEFAULT_MAX_ITERATIONS,
            created_at: crate::persist::now_secs(),
            last_reason: None,
            evaluator_model: None,
        }
    }

    /// Override the iteration cap. Kept as a builder so `Session::set_goal`
    /// can accept a plain string in the common case.
    pub fn with_max_iterations(mut self, max: usize) -> Self {
        self.max_iterations = max.max(1);
        self
    }

    /// Pin the evaluator to a specific (usually cheaper) model. Falls
    /// back to the session's model when unset.
    pub fn with_evaluator_model(mut self, model: Option<String>) -> Self {
        self.evaluator_model = model;
        self
    }
}

/// The evaluator's judgement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Evaluation {
    pub verdict: GoalVerdict,
    /// Short free-text explanation the UI shows and the harness feeds
    /// back to the main model on `not_met`.
    pub reason: String,
}

/// Four-outcome verdict — mirrors the industry consensus (Claude Code,
/// Codex) rather than a binary done/not-done so failure modes can be
/// routed to the user distinctly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GoalVerdict {
    /// Acceptance criteria clearly satisfied.
    Met,
    /// Work progressed but the criteria aren't met yet — keep going.
    NotMet,
    /// The evaluator judges the goal fundamentally unachievable.
    Impossible,
    /// Only the user can provide what's blocking progress.
    NeedsUser,
}

/// Wall-clock cap on a single evaluator call. Kept short — the evaluator
/// is meant to be fast; a hanging small-model call shouldn't stall the
/// whole goal.
const EVAL_TIMEOUT: Duration = Duration::from_secs(45);

/// Cap on how much recent transcript we feed the evaluator. Enough to
/// see the model's most-recent actions + a couple of tool results;
/// small enough that evaluator cost stays a rounding error next to the
/// main-model round.
const EVAL_TRANSCRIPT_CAP: usize = 8000;

/// Cap on the evaluator's own reply length. It only needs to emit
/// `{verdict, reason}` — 512 tokens is generous.
const EVAL_MAX_TOKENS: u32 = 512;

/// Run one evaluation. Returns `Err(reason)` when the provider call
/// or the parse fails — callers should treat that as `not_met` with a
/// warning to the user rather than aborting the goal, so a transient
/// provider blip doesn't kill an in-progress autonomous run.
pub async fn evaluate(
    provider: &dyn ChatProvider,
    model: &str,
    condition: &str,
    transcript: &[Message],
) -> Result<Evaluation, String> {
    let transcript_str = format_transcript(transcript, EVAL_TRANSCRIPT_CAP);

    let system = format!(
        "You are the goal evaluator for an autonomous coding agent. You are given a goal contract and the recent transcript. Your job is to decide whether the goal is satisfied — not whether the model did a lot of work, not whether the plan looks good, but whether the acceptance criteria in the contract are observably met.\n\n\
         Reply with a single JSON object of the form:\n\
         {{\"verdict\": \"met\"|\"not_met\"|\"impossible\"|\"needs_user\", \"reason\": \"one sentence\"}}\n\n\
         Verdicts:\n\
         - `met`: the acceptance criteria are clearly satisfied by evidence in the transcript (tests passing, file contents changed as specified, count reaches zero, etc.).\n\
         - `not_met`: work is ongoing or partial. The model should keep going.\n\
         - `impossible`: the goal is fundamentally unachievable (missing dependency that cannot be installed, contradiction in the contract, out-of-scope environment).\n\
         - `needs_user`: only the user can move the needle (missing credential, approval, ambiguous decision).\n\n\
         Never say `met` unless there is direct evidence. When in doubt, `not_met`.\n\n\
         GOAL CONTRACT:\n{condition}"
    );

    let user = format!("RECENT TRANSCRIPT:\n\n{transcript_str}\n\nEvaluate the goal now. Reply with the JSON object only.");

    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "verdict": {
                "type": "string",
                "enum": ["met", "not_met", "impossible", "needs_user"],
            },
            "reason": { "type": "string" }
        },
        "required": ["verdict", "reason"],
        "additionalProperties": false,
    });

    let req = ChatRequest {
        model: model.to_owned(),
        messages: vec![Message::system(system), Message::user(user)],
        tools: Vec::new(),
        temperature: Some(0.0),
        max_tokens: Some(EVAL_MAX_TOKENS),
        reasoning_effort: None,
        response_format: Some(ResponseFormat::JsonSchema {
            name: "goal_evaluation".into(),
            schema,
            strict: true,
        }),
    };

    let stream_res = tokio::time::timeout(EVAL_TIMEOUT, provider.stream(req)).await;
    let mut stream = match stream_res {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => return Err(format!("provider error: {e}")),
        Err(_) => return Err("evaluator timed out".into()),
    };

    let mut buf = String::new();
    let read_res = tokio::time::timeout(EVAL_TIMEOUT, async {
        while let Some(evt) = stream.next().await {
            match evt {
                Ok(ChatEvent::TextDelta(t)) => buf.push_str(&t),
                Ok(ChatEvent::Done(_)) => break,
                Ok(_) => {}
                Err(e) => return Err(format!("stream error: {e}")),
            }
        }
        Ok::<(), String>(())
    })
    .await;
    match read_res {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(e),
        Err(_) => return Err("evaluator stream timed out".into()),
    }

    parse_evaluation(&buf)
}

/// Parse the evaluator's JSON reply. Tolerant of leading/trailing prose
/// (some providers wrap `json_schema` mode in a code fence anyway) —
/// we grab the outermost `{ … }` and try that.
fn parse_evaluation(raw: &str) -> Result<Evaluation, String> {
    let json_str = extract_json_object(raw).ok_or_else(|| {
        format!("evaluator reply did not contain a JSON object: {raw:?}")
    })?;
    #[derive(Deserialize)]
    struct Reply {
        verdict: String,
        reason: String,
    }
    let parsed: Reply = serde_json::from_str(json_str)
        .map_err(|e| format!("evaluator JSON parse failed: {e} — raw: {raw:?}"))?;
    let verdict = match parsed.verdict.as_str() {
        "met" => GoalVerdict::Met,
        "not_met" => GoalVerdict::NotMet,
        "impossible" => GoalVerdict::Impossible,
        "needs_user" => GoalVerdict::NeedsUser,
        other => {
            warn!(?other, "evaluator returned unknown verdict, treating as not_met");
            GoalVerdict::NotMet
        }
    };
    Ok(Evaluation {
        verdict,
        reason: parsed.reason,
    })
}

/// Grab the first `{…}` block in `s` (balanced-brace scan). Handles the
/// "provider added a ```json fence" case without a full markdown parser.
fn extract_json_object(s: &str) -> Option<&str> {
    let start = s.find('{')?;
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;
    for i in start..bytes.len() {
        let c = bytes[i];
        if in_string {
            if escape {
                escape = false;
            } else if c == b'\\' {
                escape = true;
            } else if c == b'"' {
                in_string = false;
            }
            continue;
        }
        match c {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&s[start..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Format a transcript slice for the evaluator prompt. Each message
/// becomes `[role] text` on its own block, truncated so the whole
/// section stays under `cap` bytes.
fn format_transcript(messages: &[Message], cap: usize) -> String {
    // Walk backwards until we've either used every message or hit the
    // budget, then reverse so the evaluator sees oldest-first (the
    // natural reading order for "here's what happened").
    let mut pieces: Vec<String> = Vec::new();
    let mut used = 0usize;
    for m in messages.iter().rev() {
        let tag = match m.role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool_result",
        };
        let body = m.content.clone().unwrap_or_default();
        let piece = format!("[{tag}]\n{body}\n");
        let piece_len = piece.len();
        if used + piece_len > cap && !pieces.is_empty() {
            pieces.push("[… earlier context elided]\n".into());
            break;
        }
        pieces.push(piece);
        used += piece_len;
    }
    pieces.reverse();
    pieces.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_bare_json() {
        let e = parse_evaluation(r#"{"verdict":"met","reason":"tests pass"}"#).unwrap();
        assert_eq!(e.verdict, GoalVerdict::Met);
        assert_eq!(e.reason, "tests pass");
    }

    #[test]
    fn parse_json_in_fence() {
        let e = parse_evaluation(
            "```json\n{\"verdict\":\"not_met\",\"reason\":\"still failing\"}\n```",
        )
        .unwrap();
        assert_eq!(e.verdict, GoalVerdict::NotMet);
    }

    #[test]
    fn parse_unknown_verdict_falls_back_to_not_met() {
        let e = parse_evaluation(r#"{"verdict":"maybe","reason":"idk"}"#).unwrap();
        assert_eq!(e.verdict, GoalVerdict::NotMet);
    }

    #[test]
    fn extract_json_handles_nested_braces() {
        let s = r#"prefix {"a":{"b":1},"c":"}"} suffix"#;
        let got = extract_json_object(s).unwrap();
        assert_eq!(got, r#"{"a":{"b":1},"c":"}"}"#);
    }

    #[test]
    fn format_transcript_trims_to_budget() {
        let msgs = vec![
            Message::user("a".repeat(1000)),
            Message::assistant("b".repeat(1000)),
            Message::user("c".repeat(1000)),
        ];
        let out = format_transcript(&msgs, 1500);
        assert!(out.contains("elided"));
        assert!(out.len() < 2500);
    }
}
