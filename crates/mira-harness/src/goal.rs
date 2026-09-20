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

use std::path::Path;
use std::time::Duration;

use mira_ai::{ChatEvent, ChatProvider, ChatRequest, ResponseFormat};
use mira_core::{Message, Role};
use mira_sandbox::Sandbox;
use regex::Regex;
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
    /// Optional external verifier — a shell command whose exit-code
    /// (and optionally stdout regex) gates the "Met" verdict.
    ///
    /// Runs before the LLM evaluator on every iteration. When the
    /// verify command *fails*, the harness short-circuits to `NotMet`
    /// with the command output as the reason and skips the LLM call
    /// entirely — this is the fix for the "gaming the verifier"
    /// failure mode where the main model claims to have finished
    /// without any observable evidence. When the verify command
    /// passes, the LLM still runs (it may catch semantic problems the
    /// script can't).
    ///
    /// Absent (`None`) preserves the transcript-only behaviour that
    /// shipped in v0.1 of `/goal`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify: Option<VerifyCommand>,
    /// Optional hard token budget (input + output, summed across
    /// every provider round in this session). Checked after each
    /// iteration; on breach the goal transitions to `Exhausted` with
    /// a "budget exceeded" reason. `None` disables the token check.
    ///
    /// Complements `max_iterations`: an iteration cap bounds *count*,
    /// while `budget_tokens` bounds *spend* — a 20-iteration loop
    /// with 200K-token turns can burn a lot before the count cap
    /// fires, and this closes that hole.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_tokens: Option<u64>,
    /// Optional hard USD budget, computed from the session's
    /// accumulated token usage and the active model's list price via
    /// [`mira_ai::cost_usd`]. Checked after each iteration; on breach
    /// the goal transitions to `Exhausted` with a "budget exceeded"
    /// reason.
    ///
    /// Unpriced models (no entry in `MODEL_PRICES`) skip the USD
    /// check entirely — token cost can't be computed, so the check
    /// no-ops rather than reporting a false pass. Use
    /// `budget_tokens` when working with a self-hosted or otherwise
    /// unpriced model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_usd: Option<f64>,
}

/// External verifier for a goal. A shell command whose exit-code
/// (and optionally a regex against stdout) tells the harness whether
/// the goal condition is observably met.
///
/// The command runs inside the sandbox, which is offline by default,
/// so it must not need the network.
///
/// Example — "cargo test until every test passes":
/// ```yaml
/// verify:
///   command: cargo test --all --offline
///   # expected_exit defaults to 0
///   timeout_secs: 300
/// ```
///
/// Example — "no `v1` call sites remain":
/// ```yaml
/// verify:
///   command: grep -rn "api.v1" src/
///   expected_exit: 1   # grep returns 1 when there are no matches
/// ```
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VerifyCommand {
    /// Shell fragment run as `bash -c <command>` via the session's
    /// sandbox. Runs in the session's cwd. The sandbox execs binaries
    /// directly (no shell parsing), so the harness wraps this fragment
    /// in `bash -c` itself.
    pub command: String,
    /// Exit code that indicates "goal condition met". Defaults to 0
    /// via [`VerifyCommand::expected_exit`], but a rule like
    /// `grep -c … src/` naturally wants 1 (no match).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_exit: Option<i32>,
    /// Optional regex the command's output must match for the check
    /// to pass. The sandbox captures stdout and stderr separately;
    /// they are joined (stderr appended after stdout) before the
    /// regex runs. Applied on top of the exit-code check: both must
    /// hold.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expect_stdout: Option<String>,
    /// Wall-clock cap. Defaults to 300s. Kept generous so a real
    /// `cargo test` on a fresh checkout has room, but capped so a
    /// broken command can't wedge the goal loop forever.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
}

impl VerifyCommand {
    /// Exit code that indicates the goal condition holds. Defaults to
    /// `0` — matches the standard "run this and see if it succeeded"
    /// intuition; override only for shape-of-output checks (grep,
    /// diff, etc.).
    pub fn expected_exit(&self) -> i32 {
        self.expected_exit.unwrap_or(0)
    }

    /// Timeout for a single run. 300 seconds by default — enough for
    /// a small-to-medium test suite; a big monorepo should raise this.
    pub fn timeout_secs(&self) -> u64 {
        self.timeout_secs.unwrap_or(300)
    }
}

/// Outcome of one verify run — returned in a shape the harness can
/// translate directly into a `NotMet` reason without re-formatting.
#[derive(Clone, Debug)]
pub struct VerifyOutcome {
    /// True when the command's exit code matches
    /// `VerifyCommand::expected_exit` AND (when set) `expect_stdout`
    /// matches the combined stdout/stderr.
    pub passed: bool,
    /// Actual exit code from the command. `-1` means timeout or
    /// termination by signal (the sandbox reports `None` for both).
    pub exit_code: i32,
    /// Combined stdout+stderr transcript, truncated to
    /// [`VERIFY_OUTPUT_CAP`] bytes so a chatty script can't bloat the
    /// evaluator's context downstream.
    pub output: String,
    /// True when the command hit `timeout_secs` before completing.
    pub timed_out: bool,
}

impl VerifyOutcome {
    /// A short one-liner suitable for `Goal::last_reason` and the
    /// synthetic "keep working" continuation message when the check
    /// fails. Prefers the useful signal (exit code + tail of output)
    /// over a wall of transcript.
    pub fn short_reason(&self) -> String {
        let tail = tail_lines(&self.output, 20);
        if self.timed_out {
            format!("verify timed out. Last output:\n{tail}")
        } else if !self.passed {
            format!(
                "verify failed (exit {}). Last output:\n{tail}",
                self.exit_code
            )
        } else {
            "verify passed".to_owned()
        }
    }
}

/// Cap on the combined stdout/stderr we keep after a verify run — big
/// enough to be diagnostic, small enough that a `cargo test` failure
/// dump doesn't dominate the evaluator's context.
pub const VERIFY_OUTPUT_CAP: usize = 32 * 1024;

/// Return the last `n` lines of `s` with the leading elided marker
/// when we truncated.
fn tail_lines(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    if lines.len() <= n {
        return s.to_owned();
    }
    let mut out = String::from("[… earlier output elided]\n");
    for l in &lines[lines.len() - n..] {
        out.push_str(l);
        out.push('\n');
    }
    out
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
            verify: None,
            budget_tokens: None,
            budget_usd: None,
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

    /// Attach an external verifier. See [`VerifyCommand`] for the
    /// contract — a shell command whose exit code (and optional
    /// stdout regex) gates the "Met" verdict on every iteration.
    pub fn with_verify(mut self, verify: Option<VerifyCommand>) -> Self {
        self.verify = verify;
        self
    }

    /// Hard token budget (input + output). `None` disables the check.
    pub fn with_budget_tokens(mut self, tokens: Option<u64>) -> Self {
        self.budget_tokens = tokens;
        self
    }

    /// Hard USD budget. Requires the active model to be priced (see
    /// `mira-ai/src/pricing.rs`); unpriced models skip the check.
    pub fn with_budget_usd(mut self, usd: Option<f64>) -> Self {
        self.budget_usd = usd;
        self
    }
}

/// Outcome of a budget check. `None` means the check didn't apply
/// (no budget set, or the model isn't priced for the USD variant).
/// `Some(Ok)` means under budget; `Some(Err(reason))` means the cap
/// was breached and the caller should transition the goal to
/// `Exhausted` with `reason`.
#[derive(Clone, Debug, PartialEq)]
pub enum BudgetCheck {
    /// No budget configured, or the USD budget couldn't be computed
    /// because the model isn't priced. Skip the check without
    /// affecting the goal.
    Skipped,
    /// Under both budgets.
    Under,
    /// One of the budgets was exceeded. String is a short user-facing
    /// reason for `last_reason`.
    Exceeded(String),
}

/// Evaluate any configured budgets against a running usage total.
/// Pure — no I/O, safe to call after each iteration.
///
/// Order: token budget first (cheap, always computable), then USD
/// (skipped for unpriced models). First breach wins the reason.
pub fn check_budget(
    budget_tokens: Option<u64>,
    budget_usd: Option<f64>,
    model: &str,
    usage: crate::persist::UsageTotals,
) -> BudgetCheck {
    if budget_tokens.is_none() && budget_usd.is_none() {
        return BudgetCheck::Skipped;
    }

    if let Some(cap) = budget_tokens {
        let total = usage.total_tokens();
        if total > cap {
            return BudgetCheck::Exceeded(format!(
                "token budget exceeded: {total} used (cap {cap})"
            ));
        }
    }

    if let Some(cap) = budget_usd {
        // Fold the running total into a synthetic TokenUsage for the
        // pricing helper — its inputs are per-round but the math is
        // linear in each field, so summing across rounds gives the
        // right dollar total (modulo the u32 → u64 conversion, which
        // saturates safely).
        let synth = mira_ai::TokenUsage {
            prompt_tokens: clip_u64_to_u32(usage.prompt_tokens),
            completion_tokens: clip_u64_to_u32(usage.completion_tokens),
            cached_input_tokens: clip_u64_to_u32(usage.cached_input_tokens),
        };
        match mira_ai::cost_usd(model, synth) {
            Some(actual) if actual > cap => {
                return BudgetCheck::Exceeded(format!(
                    "USD budget exceeded: ${actual:.2} used (cap ${cap:.2}, model `{model}`)"
                ));
            }
            Some(_) => {}
            None => {
                // Unpriced model — token budget was our only hope. If
                // it was set and we got here, it passed. If it wasn't,
                // we skip.
                if budget_tokens.is_none() {
                    return BudgetCheck::Skipped;
                }
            }
        }
    }

    BudgetCheck::Under
}

/// Saturating u64 → u32 for the pricing hop. Sessions that actually
/// hit u32::MAX (4B) tokens have bigger problems than a rounding
/// error in the cost estimate.
fn clip_u64_to_u32(n: u64) -> u32 {
    if n > u32::MAX as u64 {
        u32::MAX
    } else {
        n as u32
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

    // JsonObject rather than JsonSchema — wider provider support
    // (Ollama, LM Studio, older OpenAI-compat servers routinely ignore
    // strict schema mode and return partial/empty streams). Combined
    // with the tolerant `parse_evaluation` below this survives most
    // providers without a per-vendor branch.
    let req = ChatRequest {
        model: model.to_owned(),
        messages: vec![Message::system(system), Message::user(user)],
        tools: Vec::new(),
        temperature: Some(0.0),
        max_tokens: Some(EVAL_MAX_TOKENS),
        reasoning_effort: None,
        response_format: Some(ResponseFormat::JsonObject),
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
    let json_str = extract_json_object(raw)
        .ok_or_else(|| format!("evaluator reply did not contain a JSON object: {raw:?}"))?;
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
            warn!(
                ?other,
                "evaluator returned unknown verdict, treating as not_met"
            );
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

/// Run one verification pass against the goal's `verify` command.
///
/// Returns:
///   - `Ok(VerifyOutcome { passed: true, .. })` when exit code (and
///     optional stdout regex) matched — the LLM evaluator should
///     still run on top of this to catch semantic issues the script
///     can't observe.
///   - `Ok(VerifyOutcome { passed: false, .. })` when the check
///     failed — the caller short-circuits to `NotMet` with
///     `VerifyOutcome::short_reason()` as the reason and skips the
///     LLM call entirely.
///   - `Err(_)` when the sandbox itself couldn't run the command
///     (spawn error, invalid regex, etc.). Callers treat this as a
///     `NotMet` too so a broken verify command doesn't wedge the
///     goal loop — a warning gets emitted alongside.
pub async fn run_verify(
    sandbox: &Sandbox,
    cwd: &Path,
    verify: &VerifyCommand,
) -> anyhow::Result<VerifyOutcome> {
    // Compile the stdout regex up-front — a bad regex is a config
    // error we should surface once, not on every iteration.
    let stdout_re = match verify.expect_stdout.as_deref() {
        Some(pat) => Some(
            Regex::new(pat)
                .map_err(|e| anyhow::anyhow!("invalid `expect_stdout` regex `{pat}`: {e}"))?,
        ),
        None => None,
    };

    // The sandbox execs a binary directly (no shell parsing), so wrap
    // the user's shell fragment in `bash -c` ourselves. Deliberately not
    // `-l`: a login shell re-sources profile files that may not be
    // visible inside the sandbox, and PATH is already passed through.
    let args = vec!["-c".to_owned(), verify.command.clone()];

    let outcome = sandbox
        .run_with_timeout("bash", &args, cwd, verify.timeout_secs())
        .await
        .map_err(|e| anyhow::anyhow!("verify command failed to run: {e:#}"))?;

    // Truncate before we hand it back — chatty commands would
    // otherwise dominate the evaluator's context window on the next
    // iteration.
    let output = truncate_output(&outcome.combined_output(), VERIFY_OUTPUT_CAP);

    let exit_ok = !outcome.timed_out && outcome.exit_code == Some(verify.expected_exit());
    let stdout_ok = match &stdout_re {
        Some(re) => re.is_match(&output),
        None => true,
    };
    let passed = exit_ok && stdout_ok;

    Ok(VerifyOutcome {
        passed,
        // `None` means timeout or killed by a signal; keep the -1
        // convention so downstream formatting is unchanged.
        exit_code: outcome.exit_code.unwrap_or(-1),
        output,
        timed_out: outcome.timed_out,
    })
}

/// Cap `s` at `max_bytes`, keeping the *tail* — the interesting bits
/// of a `cargo test` run are the failed assertions at the end, not
/// the compilation preamble. Char-boundary-safe.
fn truncate_output(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_owned();
    }
    let start = s.len() - max_bytes;
    // Walk forward to the next char boundary — a naive slice can
    // panic on multi-byte UTF-8 (unlikely in tool output but cheap
    // to guard).
    let mut safe_start = start;
    while safe_start < s.len() && !s.is_char_boundary(safe_start) {
        safe_start += 1;
    }
    let mut out = String::from("[… earlier output elided]\n");
    out.push_str(&s[safe_start..]);
    out
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

    /// Sandbox rooted at the current directory, plus that directory
    /// for use as the command's cwd.
    fn test_sandbox() -> (Sandbox, std::path::PathBuf) {
        let cwd = std::env::current_dir().unwrap();
        (Sandbox::new(cwd.clone()), cwd)
    }

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

    #[test]
    fn verify_expected_exit_defaults_to_zero() {
        let v = VerifyCommand {
            command: "true".into(),
            expected_exit: None,
            expect_stdout: None,
            timeout_secs: None,
        };
        assert_eq!(v.expected_exit(), 0);
        assert_eq!(v.timeout_secs(), 300);
    }

    #[test]
    fn verify_outcome_reason_shapes() {
        let passing = VerifyOutcome {
            passed: true,
            exit_code: 0,
            output: "ok\n".into(),
            timed_out: false,
        };
        assert!(passing.short_reason().contains("passed"));

        let failing = VerifyOutcome {
            passed: false,
            exit_code: 1,
            output: (0..30).map(|i| format!("line {i}\n")).collect(),
            timed_out: false,
        };
        let r = failing.short_reason();
        assert!(r.contains("failed"));
        assert!(r.contains("exit 1"));
        // Tail includes recent lines, elides earlier ones.
        assert!(r.contains("line 29"));
        assert!(!r.contains("line 0\n"));

        let tmo = VerifyOutcome {
            passed: false,
            exit_code: -1,
            output: "hung\n".into(),
            timed_out: true,
        };
        assert!(tmo.short_reason().contains("timed out"));
    }

    #[tokio::test]
    async fn run_verify_passes_on_zero_exit() {
        let (sandbox, cwd) = test_sandbox();
        let v = VerifyCommand {
            command: "true".into(),
            expected_exit: None,
            expect_stdout: None,
            timeout_secs: Some(5),
        };
        let out = run_verify(&sandbox, &cwd, &v).await.unwrap();
        assert!(out.passed);
        assert_eq!(out.exit_code, 0);
    }

    #[tokio::test]
    async fn run_verify_fails_on_nonzero_exit() {
        let (sandbox, cwd) = test_sandbox();
        let v = VerifyCommand {
            command: "false".into(),
            expected_exit: None,
            expect_stdout: None,
            timeout_secs: Some(5),
        };
        let out = run_verify(&sandbox, &cwd, &v).await.unwrap();
        assert!(!out.passed);
        assert_ne!(out.exit_code, 0);
    }

    #[tokio::test]
    async fn run_verify_honors_custom_expected_exit() {
        // `grep` returns 1 when no match — that's a common "goal
        // condition met" shape ("no v1 call sites remain"). The
        // verify should count that as pass when expected_exit=1.
        let (sandbox, cwd) = test_sandbox();
        let v = VerifyCommand {
            command: "echo hello | grep xyz".into(),
            expected_exit: Some(1),
            expect_stdout: None,
            timeout_secs: Some(5),
        };
        let out = run_verify(&sandbox, &cwd, &v).await.unwrap();
        assert!(
            out.passed,
            "grep-no-match should pass with expected_exit=1, got {out:?}"
        );
    }

    #[tokio::test]
    async fn run_verify_stdout_regex_gates_pass() {
        let (sandbox, cwd) = test_sandbox();
        // Exit 0 but stdout regex doesn't match → fail.
        let v = VerifyCommand {
            command: "echo greeting".into(),
            expected_exit: Some(0),
            expect_stdout: Some(r"^bye".into()),
            timeout_secs: Some(5),
        };
        let out = run_verify(&sandbox, &cwd, &v).await.unwrap();
        assert!(!out.passed);

        // Exit 0 and regex matches → pass.
        let v2 = VerifyCommand {
            command: "echo greeting".into(),
            expected_exit: Some(0),
            expect_stdout: Some(r"greeting".into()),
            timeout_secs: Some(5),
        };
        let out2 = run_verify(&sandbox, &cwd, &v2).await.unwrap();
        assert!(out2.passed);
    }

    #[tokio::test]
    async fn run_verify_timeout_reports_minus_one() {
        let (sandbox, cwd) = test_sandbox();
        let v = VerifyCommand {
            command: "sleep 10".into(),
            expected_exit: None,
            expect_stdout: None,
            timeout_secs: Some(1),
        };
        let out = run_verify(&sandbox, &cwd, &v).await.unwrap();
        assert!(!out.passed);
        assert!(out.timed_out);
        assert_eq!(out.exit_code, -1);
    }

    /* ---- budget check ---- */

    fn usage_with(prompt: u64, completion: u64) -> crate::persist::UsageTotals {
        crate::persist::UsageTotals {
            prompt_tokens: prompt,
            completion_tokens: completion,
            cached_input_tokens: 0,
            rounds: 1,
        }
    }

    #[test]
    fn budget_check_skips_when_no_budgets_configured() {
        let out = check_budget(None, None, "gpt-4o-mini", usage_with(100_000, 50_000));
        assert!(matches!(out, BudgetCheck::Skipped));
    }

    #[test]
    fn budget_check_token_cap_breach() {
        let out = check_budget(Some(1_000), None, "gpt-4o-mini", usage_with(800, 500));
        match out {
            BudgetCheck::Exceeded(msg) => {
                assert!(msg.contains("token budget"));
                assert!(msg.contains("1300"));
                assert!(msg.contains("1000"));
            }
            other => panic!("expected Exceeded, got {other:?}"),
        }
    }

    #[test]
    fn budget_check_under_token_cap() {
        let out = check_budget(Some(10_000), None, "gpt-4o-mini", usage_with(800, 500));
        assert!(matches!(out, BudgetCheck::Under));
    }

    #[test]
    fn budget_check_usd_cap_breach() {
        // gpt-4o-mini: $0.15/M input, $0.60/M output. 1M input + 1M
        // output = $0.15 + $0.60 = $0.75. Cap at $0.10 → breach.
        let out = check_budget(
            None,
            Some(0.10),
            "gpt-4o-mini",
            usage_with(1_000_000, 1_000_000),
        );
        match out {
            BudgetCheck::Exceeded(msg) => {
                assert!(msg.contains("USD budget"));
                assert!(msg.contains("$0.75"));
                assert!(msg.contains("$0.10"));
            }
            other => panic!("expected Exceeded, got {other:?}"),
        }
    }

    #[test]
    fn budget_check_usd_under_cap() {
        let out = check_budget(
            None,
            Some(10.0),
            "gpt-4o-mini",
            usage_with(1_000_000, 1_000_000),
        );
        assert!(matches!(out, BudgetCheck::Under));
    }

    #[test]
    fn budget_check_unpriced_model_skips_usd_but_honors_tokens() {
        // Model not in the pricing table → USD check can't compute.
        // With a token cap set and under, that's Under (token check
        // passed; USD skipped silently).
        let out = check_budget(
            Some(10_000),
            Some(1.0),
            "some-local-model",
            usage_with(100, 50),
        );
        assert!(matches!(out, BudgetCheck::Under));

        // With only a USD cap and no token cap on an unpriced model,
        // Skipped — nothing to check.
        let out2 = check_budget(None, Some(1.0), "some-local-model", usage_with(100, 50));
        assert!(matches!(out2, BudgetCheck::Skipped));
    }

    #[test]
    fn budget_check_token_cap_breach_wins_over_usd() {
        // Both set, token cap fires first (order in the check
        // function). Verifies the reason mentions tokens.
        let out = check_budget(Some(100), Some(10.0), "gpt-4o-mini", usage_with(500, 500));
        match out {
            BudgetCheck::Exceeded(msg) => assert!(msg.contains("token")),
            other => panic!("expected token-cap Exceeded, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn run_verify_bad_regex_errors() {
        let (sandbox, cwd) = test_sandbox();
        let v = VerifyCommand {
            command: "true".into(),
            expected_exit: None,
            // Unclosed bracket — real bad regex.
            expect_stdout: Some("[unclosed".into()),
            timeout_secs: Some(5),
        };
        let res = run_verify(&sandbox, &cwd, &v).await;
        assert!(res.is_err(), "bad regex should surface as Err");
    }
}