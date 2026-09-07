use std::sync::Arc;

use futures::{stream::BoxStream, StreamExt};
use mira_ai::{ChatEvent, ChatProvider, ChatRequest, FinishReason};
use mira_core::{Message, SessionId, ToolCall, ToolResult};
use mira_policy::{Decision, Policy, Request as PolicyRequest};
use mira_tools::{FileGuard, Registry, ToolContext};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, Mutex};
use tokio::task::AbortHandle;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{error, info, warn};

/// How much of a tool result actually goes back into the model's context.
/// A single `ls -R` on a real repo can be 20k+ tokens which blows past
/// every provider's per-request cap — cap in the harness so the frontend
/// still sees the full result but the model only sees a preview.
const TOOL_RESULT_HISTORY_CAP: usize = 4000;

use crate::approver::Approver;
use crate::event::HarnessEvent;
use crate::persist::{now_ms, now_secs, SessionRecord, SessionStore, TurnMeta, UsageTotals};

/// Runtime configuration for a session. Everything the loop needs besides
/// mutable state.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionConfig {
    pub model: String,
    /// Number of tool-call rounds allowed within one user turn. Guards
    /// against runaway loops (model calling itself forever).
    #[serde(default = "default_max_rounds")]
    pub max_rounds: usize,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// Reasoning effort for models that expose it (OpenAI `reasoning_effort`,
    /// Anthropic thinking budget, …). `None` = don't send the field. Values
    /// mirror OpenAI's vocabulary: `"minimal" | "low" | "medium" | "high"`.
    /// Providers that don't recognise the field ignore it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
}

/// Ceiling on tool-call rounds within a single user turn. Guards against
/// runaway loops (a model calling tools forever) without cutting real
/// refactors short. Raise via `max_rounds` in `mira.yaml` when 60 isn't
/// enough — long feature builds legitimately exceed it.
fn default_max_rounds() -> usize {
    60
}

impl SessionConfig {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            max_rounds: default_max_rounds(),
            temperature: None,
            max_tokens: None,
            reasoning_effort: None,
        }
    }
}

/// A single conversation.
///
/// All mutable state (history, policy) is behind `Arc<Mutex<_>>` so [`send`]
/// can spawn its loop on a background task and stream events out without
/// holding a mutable borrow on `Session`. Cheap to `Clone` — every field is
/// an `Arc` or a small `Clone` value.
///
/// [`send`]: Session::send
#[derive(Clone)]
pub struct Session {
    pub id: SessionId,
    cfg: Arc<Mutex<SessionConfig>>,
    history: Arc<Mutex<Vec<Message>>>,
    /// Human-readable nickname. Generated post-hoc by the server after the
    /// first assistant reply; the harness itself only reads + persists it.
    title: Arc<Mutex<Option<String>>>,
    /// Per-turn wall-clock timing. Appended when [`Session::send`] pushes a
    /// user message; the last entry's `ended_at` is stamped when the loop
    /// emits its final `Done` event.
    turns: Arc<Mutex<Vec<TurnMeta>>>,
    /// Aggregate token usage. Folded in whenever the provider emits a usage
    /// trailer; persisted alongside the session.
    usage: Arc<Mutex<UsageTotals>>,
    created_at: u64,

    provider: Arc<dyn ChatProvider>,
    registry: Arc<Registry>,
    policy: Arc<Mutex<Policy>>,
    approver: Arc<dyn Approver>,
    tool_ctx: ToolContext,
    store: Option<Arc<dyn SessionStore>>,
    /// Handle to the currently-running turn task, if any. `cancel()` aborts
    /// it; the loop's `tx.send` calls then fail as the channel closes and
    /// the frontend stops seeing new events.
    current_turn: Arc<Mutex<Option<AbortHandle>>>,
}

impl Session {
    pub fn new(
        cfg: SessionConfig,
        system_prompt: impl Into<String>,
        provider: Arc<dyn ChatProvider>,
        registry: Arc<Registry>,
        policy: Arc<Mutex<Policy>>,
        approver: Arc<dyn Approver>,
        mut tool_ctx: ToolContext,
    ) -> Self {
        let id = SessionId::new();
        // Attach a session-scoped FileGuard for conflict detection + undo.
        // Failure is logged and swallowed — a missing guard just means those
        // features are disabled for this session (files still get read /
        // written normally).
        if let Ok(g) = FileGuard::open(&id.to_string(), tool_ctx.cwd.clone()) {
            tool_ctx = tool_ctx.with_guard(Arc::new(g));
        } else {
            warn!(session = %id, "file guard init failed; undo + conflict detection disabled");
        }
        Self {
            id,
            cfg: Arc::new(Mutex::new(cfg)),
            history: Arc::new(Mutex::new(vec![Message::system(system_prompt)])),
            title: Arc::new(Mutex::new(None)),
            turns: Arc::new(Mutex::new(Vec::new())),
            usage: Arc::new(Mutex::new(UsageTotals::default())),
            created_at: now_secs(),
            provider,
            registry,
            policy,
            approver,
            tool_ctx,
            store: None,
            current_turn: Arc::new(Mutex::new(None)),
        }
    }

    /// Rehydrate from a persisted record — same wiring as [`Session::new`]
    /// but the id, history, config, and creation timestamp come from disk.
    /// Attach a store afterwards with [`Session::with_store`] to keep
    /// autosaving.
    #[allow(clippy::too_many_arguments)]
    pub fn resume_from(
        record: SessionRecord,
        provider: Arc<dyn ChatProvider>,
        registry: Arc<Registry>,
        policy: Arc<Mutex<Policy>>,
        approver: Arc<dyn Approver>,
        mut tool_ctx: ToolContext,
    ) -> Self {
        // Same guard wiring as `new` — seq counter picks up where the
        // previous run left off (see FileGuard::open).
        if let Ok(g) = FileGuard::open(&record.id.to_string(), tool_ctx.cwd.clone()) {
            tool_ctx = tool_ctx.with_guard(Arc::new(g));
        } else {
            warn!(session = %record.id, "file guard init failed on resume");
        }
        Self {
            id: record.id,
            cfg: Arc::new(Mutex::new(record.cfg)),
            history: Arc::new(Mutex::new(record.messages)),
            title: Arc::new(Mutex::new(record.title)),
            turns: Arc::new(Mutex::new(record.turns)),
            usage: Arc::new(Mutex::new(record.usage)),
            created_at: record.created_at,
            provider,
            registry,
            policy,
            approver,
            tool_ctx,
            store: None,
            current_turn: Arc::new(Mutex::new(None)),
        }
    }

    /// Attach a store so the session autosaves after each round.
    pub fn with_store(mut self, store: Arc<dyn SessionStore>) -> Self {
        self.store = Some(store);
        self
    }

    /// Snapshot the current history.
    pub async fn history(&self) -> Vec<Message> {
        self.history.lock().await.clone()
    }

    pub async fn config(&self) -> SessionConfig {
        self.cfg.lock().await.clone()
    }

    /// Read the current nickname, if one has been generated.
    pub async fn title(&self) -> Option<String> {
        self.title.lock().await.clone()
    }

    /// Snapshot of per-turn timing. Same ordering as user messages in
    /// `history()`.
    pub async fn turns(&self) -> Vec<TurnMeta> {
        self.turns.lock().await.clone()
    }

    /// Snapshot the aggregate token usage across every provider round in
    /// this session so far.
    pub async fn usage(&self) -> UsageTotals {
        *self.usage.lock().await
    }

    /// Expose the session's undo/conflict guard. `None` when the FileGuard
    /// failed to initialise (see the warn! in `new` / `resume_from`).
    pub fn file_guard(&self) -> Option<Arc<FileGuard>> {
        self.tool_ctx.guard.clone()
    }

    /// Stamp `ended_at` on the most recent open turn, if any. Idempotent —
    /// calling twice on the same turn keeps the first end time. Used by
    /// callers that emit `HarnessEvent::Done` outside the harness's own
    /// loop (e.g. `Interrupt` in the server WS handler).
    pub async fn end_current_turn(&self) {
        let mut guard = self.turns.lock().await;
        if let Some(last) = guard.last_mut() {
            if last.ended_at.is_none() {
                last.ended_at = Some(now_ms());
            }
        }
    }

    /// Set the nickname and persist immediately. Callers are expected to
    /// have generated a sensible short title; the harness doesn't validate
    /// content beyond trimming whitespace and enforcing a hard cap so a
    /// runaway model can't stuff the sidebar with a paragraph.
    pub async fn set_title(&self, title: impl Into<String>) {
        let mut t = title.into().trim().to_string();
        if t.is_empty() {
            return;
        }
        const MAX: usize = 80;
        if t.chars().count() > MAX {
            t = t.chars().take(MAX).collect();
        }
        *self.title.lock().await = Some(t);
        // Flush a checkpoint so a crash/reload after title generation still
        // shows the nickname. Failures are logged inside `checkpoint`.
        checkpoint(self).await;
    }

    /// Hot-swap the model. Applies to the next `send()` — an in-flight turn
    /// finishes on the model it started with, since `run_loop` snapshots the
    /// config at spawn time.
    pub async fn set_model(&self, model: impl Into<String>) {
        self.cfg.lock().await.model = model.into();
    }

    /// Set the reasoning-effort field for future turns. `None` clears it so
    /// non-reasoning models aren't hit with an ignored parameter. Same
    /// snapshot-at-spawn caveat as `set_model` — in-flight turn keeps the
    /// prior value.
    pub async fn set_reasoning_effort(&self, effort: Option<String>) {
        self.cfg.lock().await.reasoning_effort = effort;
    }

    /// Run one user turn to completion.
    ///
    /// The returned stream ends with [`HarnessEvent::Done`]. Callers may
    /// drop it early to cancel — the task keeps mutating history until it
    /// hits a checkpoint, then exits when the send channel closes.
    pub async fn send(&self, user_input: impl Into<String>) -> BoxStream<'static, HarnessEvent> {
        self.history.lock().await.push(Message::user(user_input));
        // Open a new turn timer; `run_loop` stamps `ended_at` on the way out.
        self.turns.lock().await.push(TurnMeta {
            started_at: now_ms(),
            ended_at: None,
        });

        let cfg = self.cfg.lock().await.clone();
        let this = self.clone();
        let (tx, rx) = mpsc::channel::<HarnessEvent>(64);

        // If a prior turn is still running (shouldn't happen with a
        // well-behaved UI but easy to hit while debugging), abort it —
        // otherwise two tasks race to mutate history.
        let handle = tokio::spawn(async move { run_loop(this, cfg, tx).await });
        let mut slot = self.current_turn.lock().await;
        if let Some(prev) = slot.take() {
            prev.abort();
        }
        *slot = Some(handle.abort_handle());

        ReceiverStream::new(rx).boxed()
    }

    /// Cancel the currently-running turn, if any. Any in-flight tool call
    /// finishes on its own thread (we don't kill child processes), but the
    /// model stream stops pumping events and the next round never starts.
    pub async fn cancel(&self) -> bool {
        let mut slot = self.current_turn.lock().await;
        match slot.take() {
            Some(h) => {
                h.abort();
                true
            }
            None => false,
        }
    }
}

// ---- the loop ----

/// Cap on how many times a single turn will inject verify failures back
/// into the model before giving up. Prevents an unfixable error from
/// looping the turn forever; the user still sees a warning frame and can
/// intervene manually.
const MAX_VERIFY_ATTEMPTS: usize = 3;

async fn run_loop(sess: Session, cfg: SessionConfig, tx: mpsc::Sender<HarnessEvent>) {
    // Snapshot the set of paths already written to at turn start so a
    // later diff tells us what *this* turn touched. When there's no
    // FileGuard, apply-verify is disabled entirely (empty set → no
    // detected writes → verify skipped).
    let writes_at_turn_start: std::collections::HashSet<std::path::PathBuf> =
        match sess.tool_ctx.guard.as_ref() {
            Some(g) => g.written_snapshot().await,
            None => std::collections::HashSet::new(),
        };
    let mut verify_attempts = 0usize;

    for round in 0..cfg.max_rounds {
        info!(round, "harness: model turn");

        let req = ChatRequest {
            model: cfg.model.clone(),
            messages: sess.history.lock().await.clone(),
            tools: sess.registry.specs(),
            temperature: cfg.temperature,
            max_tokens: cfg.max_tokens,
            reasoning_effort: cfg.reasoning_effort.clone(),
        };

        let mut stream = match sess.provider.stream(req).await {
            Ok(s) => s,
            Err(e) => {
                let _ = tx
                    .send(HarnessEvent::Warning(format!("provider error: {e}")))
                    .await;
                sess.end_current_turn().await;
                checkpoint(&sess).await;
                let _ = tx.send(HarnessEvent::Done).await;
                return;
            }
        };

        let mut assistant_text = String::new();
        let mut pending_calls: Vec<ToolCall> = Vec::new();
        let mut finish: FinishReason = FinishReason::Other;

        while let Some(evt) = stream.next().await {
            match evt {
                Ok(ChatEvent::TextDelta(t)) => {
                    assistant_text.push_str(&t);
                    if tx.send(HarnessEvent::Token(t)).await.is_err() {
                        return;
                    }
                }
                Ok(ChatEvent::ToolCalls(calls)) => {
                    pending_calls = calls;
                }
                Ok(ChatEvent::Usage(round)) => {
                    let totals = {
                        let mut u = sess.usage.lock().await;
                        u.add_round(round);
                        *u
                    };
                    // Ignore send errors — a dropped receiver just means the
                    // UI stopped listening; the totals are still recorded.
                    let _ = tx.send(HarnessEvent::Usage { round, totals }).await;
                }
                Ok(ChatEvent::Done(reason)) => {
                    finish = reason;
                    break;
                }
                Err(e) => {
                    let _ = tx
                        .send(HarnessEvent::Warning(format!("stream error: {e}")))
                        .await;
                    break;
                }
            }
        }

        // Record the assistant turn — may carry text, tool calls, or both.
        let assistant_msg = if pending_calls.is_empty() {
            Message::assistant(assistant_text.clone())
        } else if assistant_text.is_empty() {
            Message::assistant_calls(pending_calls.clone())
        } else {
            let mut m = Message::assistant(assistant_text.clone());
            m.tool_calls = pending_calls.clone();
            m
        };
        sess.history.lock().await.push(assistant_msg);
        checkpoint(&sess).await;
        let _ = tx.send(HarnessEvent::TurnComplete).await;

        if pending_calls.is_empty() || finish == FinishReason::Stop {
            // Apply-verify: if the model wrote source files this turn, run
            // the project's natural safety check (cargo check / tsc / …).
            // On failure, feed the errors back and let the model take one
            // more crack at it — up to MAX_VERIFY_ATTEMPTS total.
            if verify_attempts < MAX_VERIFY_ATTEMPTS {
                if let Some((check, output)) =
                    run_verify(&sess, &writes_at_turn_start, &tx).await
                {
                    verify_attempts += 1;
                    // Inject the failure as a user message so the next
                    // model round sees it as fresh feedback (rather than
                    // as a tool_result which requires a matching call).
                    let synthetic = format!(
                        "The `{}` check just failed after your last edits:\n\n\
                         ```\n{}\n```\n\n\
                         Fix the errors and continue. You have {} more automatic \
                         verify retries before I stop.",
                        check.name,
                        truncate_for_history(&output),
                        MAX_VERIFY_ATTEMPTS - verify_attempts,
                    );
                    sess.history.lock().await.push(Message::user(synthetic));
                    continue;
                }
            } else {
                // We hit the retry cap. Emit a warning so the user knows
                // and doesn't wonder why the errors are still there.
                let _ = tx
                    .send(HarnessEvent::Warning(format!(
                        "[verify] still failing after {MAX_VERIFY_ATTEMPTS} attempts — stopping"
                    )))
                    .await;
            }

            sess.end_current_turn().await;
            checkpoint(&sess).await;
            let _ = tx.send(HarnessEvent::Done).await;
            return;
        }

        // Dispatch calls. Denials become tool-result messages so the model
        // sees WHY it didn't get a result and can adapt.
        for call in pending_calls {
            let Some(tool) = sess.registry.get(&call.function.name) else {
                let msg = format!("no such tool: {}", call.function.name);
                warn!(tool = %call.function.name, "unknown tool call");
                let result = ToolResult::err(call.id.clone(), msg);
                sess.history.lock().await.push(Message::tool(
                    result.call_id.clone(),
                    truncate_for_history(&result.content),
                ));
                let _ = tx.send(HarnessEvent::ToolEnd(result)).await;
                continue;
            };

            let target = tool.policy_target(&call);
            let decision = sess.policy.lock().await.evaluate(&PolicyRequest {
                action: tool.action(),
                target: &target,
            });

            let allowed = match decision {
                Decision::Allow => true,
                Decision::Deny => false,
                Decision::Ask => sess.approver.approve(&call, decision).await,
            };

            let _ = tx.send(HarnessEvent::ToolStart(call.clone())).await;

            let result = if !allowed {
                ToolResult::err(
                    call.id.clone(),
                    format!(
                        "denied by policy: {} on `{}`",
                        format_action(tool.action()),
                        target
                    ),
                )
            } else {
                match tool.invoke(&call, &sess.tool_ctx).await {
                    Ok(r) => r,
                    Err(e) => {
                        error!(tool = %call.function.name, %e, "tool invocation failed");
                        ToolResult::err(call.id.clone(), e.to_string())
                    }
                }
            };

            sess.history.lock().await.push(Message::tool(
                result.call_id.clone(),
                truncate_for_history(&result.content),
            ));
            let _ = tx.send(HarnessEvent::ToolEnd(result)).await;
        }
        checkpoint(&sess).await;
    }

    let _ = tx
        .send(HarnessEvent::Warning(format!(
            "hit max_rounds ({}) — send `continue` to resume, or raise `max_rounds` in mira.yaml",
            cfg.max_rounds
        )))
        .await;
    sess.end_current_turn().await;
    checkpoint(&sess).await;
    let _ = tx.send(HarnessEvent::Done).await;
}

/// Snapshot the session and save through the attached store, if any.
/// Failures are logged and swallowed — losing a checkpoint shouldn't kill
/// the running conversation.
async fn checkpoint(sess: &Session) {
    let Some(store) = &sess.store else { return };
    let record = SessionRecord {
        id: sess.id.clone(),
        cwd: sess.tool_ctx.cwd.clone(),
        cfg: sess.cfg.lock().await.clone(),
        messages: sess.history.lock().await.clone(),
        created_at: sess.created_at,
        updated_at: now_secs(),
        title: sess.title.lock().await.clone(),
        turns: sess.turns.lock().await.clone(),
        usage: *sess.usage.lock().await,
    };
    if let Err(e) = store.save(&record).await {
        warn!(session = %sess.id, %e, "session checkpoint failed");
    }
}

/// Truncate a tool result to a size the model can safely re-ingest. We keep
/// the head (usually the most relevant part — first lines of a diff, the
/// start of a file listing) and add a marker line telling the model how
/// much was elided.
fn truncate_for_history(content: &str) -> String {
    if content.len() <= TOOL_RESULT_HISTORY_CAP {
        return content.to_owned();
    }
    let cut = content
        .char_indices()
        .take_while(|(i, _)| *i < TOOL_RESULT_HISTORY_CAP)
        .map(|(i, _)| i)
        .last()
        .unwrap_or(0);
    let head = &content[..cut];
    let omitted = content.len() - cut;
    format!("{head}\n\n… [{omitted} bytes truncated; ask again with a narrower query to see more]")
}

fn format_action(a: mira_tools::Action) -> &'static str {
    match a {
        mira_tools::Action::Read => "Read",
        mira_tools::Action::Edit => "Edit",
        mira_tools::Action::Write => "Write",
        mira_tools::Action::Bash => "Bash",
        mira_tools::Action::Pure => "Pure",
    }
}

/// Compute this turn's new writes vs the pre-turn snapshot, pick an
/// appropriate check, and run it. Returns `Some((check, output))` only
/// on failure — success + "no writes" + "no matching check" all return
/// `None` so the caller emits `Done` unchanged.
async fn run_verify(
    sess: &Session,
    writes_at_turn_start: &std::collections::HashSet<std::path::PathBuf>,
    tx: &mpsc::Sender<HarnessEvent>,
) -> Option<(crate::verify::VerifyCheck, String)> {
    let guard = sess.tool_ctx.guard.as_ref()?;
    let now = guard.written_snapshot().await;
    let new_writes: Vec<std::path::PathBuf> = now
        .difference(writes_at_turn_start)
        .cloned()
        .collect();
    if new_writes.is_empty() {
        return None;
    }
    let check = crate::verify::detect(&sess.tool_ctx.cwd, &new_writes)?;
    let _ = tx
        .send(HarnessEvent::Warning(format!(
            "[verify] running `{}`…",
            check.name
        )))
        .await;
    let outcome = crate::verify::run(&sess.tool_ctx.sandbox, &check, &sess.tool_ctx.cwd).await;
    if outcome.ok {
        let _ = tx
            .send(HarnessEvent::Warning(format!(
                "[verify] `{}` passed",
                check.name
            )))
            .await;
        None
    } else {
        let _ = tx
            .send(HarnessEvent::Warning(format!(
                "[verify] `{}` failed — asking model to fix",
                check.name
            )))
            .await;
        Some((check, outcome.output))
    }
}
