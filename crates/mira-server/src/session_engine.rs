//! What drives a session: a provider or an external agent.
//!
//! Every session runs on exactly one *engine* at a time — one of Mira's
//! own providers, or an external agent (Claude Code, Codex, …). This module
//! owns that choice and the transitions between the two, so the rest of the
//! server (and the client) never has to reconstruct it from scattered state.
//!
//! # One view, pushed on every transition
//!
//! [`SessionEngine`] is the whole answer to "what am I talking to", sent in
//! `Ready` and as a `session_engine` frame whenever it changes: an engine
//! picked, an agent starting, ready, failed or exited. The composer renders
//! from it directly — picking an agent updates the UI at once, not when the
//! first reply arrives.
//!
//! # Switching keeps the conversation
//!
//! A provider and an agent keep separate histories (the harness must not
//! read agent words as its own, and an agent's context lives in its own
//! process). Switching mid-session therefore hands the other side what it
//! missed: the agent's turns become a note in the harness history, and the
//! provider's turns ride in front of the agent's next prompt. Each side
//! only receives what it has not already seen, tracked by [`Handoff`].
//!
//! # Starts are serialized
//!
//! Agent processes are started from several places — a pick, a first
//! prompt, a mode or model change — and a double click or a prompt sent
//! while a pick is still starting used to spawn two processes for one
//! session. Every start goes through [`EngineRuntime::start_lock`].

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::acp_session::{AcpLaunchParams, SlotAgent};
use crate::protocol::ServerMsg;
use crate::slot::SessionSlot;
use crate::state::AppState;

/// Which kind of engine a session runs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineKind {
    Provider,
    Agent,
}

/// Lifecycle of the session's engine. A provider is always `Ready`; an
/// agent moves `Idle → Starting → Ready`, and back to `Idle` when its
/// process exits (the next prompt restarts it, resuming its session).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineStatus {
    /// Selected, not running. The first prompt starts it.
    Idle,
    Starting,
    Ready,
    /// The last start failed; `error` says why.
    Error,
}

/// The session's engine, as the client renders it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionEngine {
    pub kind: EngineKind,
    /// Provider: the engine instance (`anthropic`, `openrouter`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    /// Agent: the driver kind (`claude-code`, `codex`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver: Option<String>,
    pub display_name: String,
    /// The model this engine runs, when known. For an agent this is the
    /// model picked for it, or the one it reported running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub status: EngineStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// How far each side has read the other's turns. See the module docs.
#[derive(Debug, Default)]
pub struct Handoff {
    /// Harness transcript length the agent has been given.
    pub agent_seen_harness: usize,
    /// Sidecar line count the harness has been given.
    pub harness_seen_agent: usize,
    /// Set when the session moved provider → agent: the agent's next
    /// prompt carries the provider turns it missed.
    pub agent_needs_context: bool,
}

/// Per-slot engine runtime. Lives on [`SessionSlot`].
#[derive(Default)]
pub struct EngineRuntime {
    /// Held for the whole of any agent start or restart.
    pub start_lock: Mutex<()>,
    /// Where the agent is in its lifecycle when it is not simply running.
    pub phase: Mutex<AgentPhase>,
    /// The agent's running model as it last reported it.
    pub reported_model: std::sync::Mutex<Option<String>>,
    /// Set once the transcript logger is subscribed, so restarts do not
    /// stack a second logger that writes every frame twice.
    pub logger_started: AtomicBool,
    pub handoff: Mutex<Handoff>,
    /// The scope the user picked for each pending approval, by Mira call
    /// id — recorded before the approval resolves, so the agent's gate can
    /// turn "allow for this session" into rules the agent keeps.
    pub approval_scopes: std::sync::Mutex<std::collections::HashMap<String, crate::protocol::ApprovalScope>>,
    /// Unix seconds of the agent's last sign of life (a prompt or a frame),
    /// for the idle reaper.
    pub last_active: std::sync::atomic::AtomicU64,
}

impl EngineRuntime {
    pub fn touch(&self) {
        self.last_active.store(now_secs(), std::sync::atomic::Ordering::Relaxed);
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// An idle agent is stopped after this long. Its session stays set up for
/// the agent: the next prompt restarts it, resuming the same conversation,
/// so all this reclaims is a process nobody is using.
const REAP_AFTER: std::time::Duration = std::time::Duration::from_secs(30 * 60);
const REAP_EVERY: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// Stop agent processes that have sat idle past [`REAP_AFTER`].
///
/// Every chat that picks an agent owns a CLI process, and before this they
/// lived until the server exited — a day of chats was a day of idle Claude
/// Code and Codex processes holding memory and terminals. A process is only
/// reaped between turns and with no approval or question pending.
pub fn spawn_reaper(state: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(REAP_EVERY);
        tick.tick().await;
        loop {
            tick.tick().await;
            let now = now_secs();
            for slot in state.list_slots().await {
                if slot.acp_agent.read().await.is_none() {
                    continue;
                }
                let idle = now.saturating_sub(slot.engine.last_active.load(std::sync::atomic::Ordering::Relaxed));
                if idle < REAP_AFTER.as_secs() {
                    continue;
                }
                let waiting = !slot.pending.lock().await.is_empty()
                    || !slot.prompt_pending.lock().await.is_empty();
                if waiting {
                    continue;
                }
                tracing::info!(session = %slot.id, idle_secs = idle, "reaping idle agent");
                crate::acp_session::stop_agent(&slot).await;
                *slot.engine.phase.lock().await = AgentPhase::Idle;
                publish(&state, &slot).await;
            }
        }
    });
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum AgentPhase {
    #[default]
    Idle,
    Starting,
    Failed(String),
}

impl EngineRuntime {
    /// A runtime for a slot rebuilt from disk: both sides are assumed to
    /// have seen everything recorded so far, so a switch after reload hands
    /// over only what happens next rather than replaying old turns.
    pub fn resumed(harness_len: usize, agent_lines: usize) -> Self {
        let rt = EngineRuntime::default();
        if let Ok(mut h) = rt.handoff.try_lock() {
            h.agent_seen_harness = harness_len;
            h.harness_seen_agent = agent_lines;
        }
        rt
    }
}

/// The session's engine right now.
pub async fn current(state: &AppState, slot: &SessionSlot) -> SessionEngine {
    let launch = slot.acp_launch.lock().await.clone();
    let Some(params) = launch else {
        let (instance, _model, _small) = state.selection.snapshot();
        let model = slot.session.read().await.config().await.model;
        let display_name = instance
            .as_deref()
            .and_then(|id| state.engines.get(id))
            .and_then(|i| i.display_name.clone())
            .or_else(|| instance.clone())
            .unwrap_or_else(|| "Mira".to_string());
        return SessionEngine {
            kind: EngineKind::Provider,
            instance,
            driver: None,
            display_name,
            model: Some(model),
            status: EngineStatus::Ready,
            error: None,
        };
    };

    let live = match slot.acp_agent.read().await.clone() {
        Some(h) => h.agent().await.is_some(),
        None => false,
    };
    let phase = slot.engine.phase.lock().await.clone();
    let (status, error) = if live {
        (EngineStatus::Ready, None)
    } else {
        match phase {
            AgentPhase::Starting => (EngineStatus::Starting, None),
            AgentPhase::Failed(e) => (EngineStatus::Error, Some(e)),
            AgentPhase::Idle => (EngineStatus::Idle, None),
        }
    };
    let reported = slot.engine.reported_model.lock().ok().and_then(|m| m.clone());
    SessionEngine {
        kind: EngineKind::Agent,
        instance: None,
        display_name: params.display_name(),
        driver: Some(params.driver_kind.clone()),
        model: reported.or(params.model.clone()),
        status,
        error,
    }
}

/// Send the session's engine to everyone watching it.
pub async fn publish(state: &AppState, slot: &SessionSlot) {
    let engine = current(state, slot).await;
    let _ = slot.events_tx.send(ServerMsg::SessionEngine { engine });
}

/// Make `params` this session's engine and bring the agent up in the
/// background, so the composer can show it (and any startup failure)
/// straight away instead of on the first prompt.
pub async fn select_agent(state: &AppState, slot: &Arc<SessionSlot>, params: AcpLaunchParams) {
    let previous = slot.acp_launch.lock().await.clone();
    let from_provider = previous.is_none();
    let same_setup = previous.as_ref().is_some_and(|p| p.same_agent(&params));

    // A different agent (or the same one reconfigured) replaces the old
    // process: two agents must never share one session's permissions.
    if !same_setup {
        crate::acp_session::stop_agent(slot).await;
    }
    if from_provider {
        slot.engine.handoff.lock().await.agent_needs_context = true;
        // Mark where the provider's turns stop, so a reload replays the two
        // histories interleaved in the order they happened rather than all
        // provider turns followed by all agent turns.
        if let Some(path) = state.store.as_ref().and_then(|s| s.agent_log_path(&slot.id)) {
            let harness_len = slot.session.read().await.transcript().await.len();
            mira_acp::agent_sessions::append_line_to(
                &path,
                &serde_json::json!({
                    "t": mira_harness::persist::now_ms(),
                    "driver": params.driver_kind,
                    "switch": { "harness_len": harness_len, "to": "agent" },
                }),
            );
            slot.engine.handoff.lock().await.harness_seen_agent += 1;
        }
    }
    *slot.acp_launch.lock().await = Some(params.clone());
    remember_agent(slot, &params, true).await;
    *slot.engine.phase.lock().await = AgentPhase::Starting;
    publish(state, slot).await;

    let state = state.clone();
    let slot = slot.clone();
    tokio::spawn(async move {
        let _ = ensure_agent(&state, &slot).await;
    });
}

/// Make a provider this session's engine, leaving any agent behind. The
/// agent's turns since the last switch are handed to the harness first, so
/// the provider continues the same conversation.
pub async fn select_provider(state: &AppState, slot: &Arc<SessionSlot>) {
    let Some(params) = slot.acp_launch.lock().await.take() else {
        return;
    };
    crate::acp_session::stop_agent(slot).await;
    *slot.engine.phase.lock().await = AgentPhase::Idle;
    if let Ok(mut m) = slot.engine.reported_model.lock() {
        *m = None;
    }
    remember_agent(slot, &params, false).await;
    hand_agent_turns_to_harness(state, slot, &params).await;
    publish(state, slot).await;
}

/// The running agent, starting it first when the session is set up for one
/// and nothing is running. Every start path funnels through here, under the
/// slot's start lock, so concurrent callers share one process.
pub async fn ensure_agent(
    state: &AppState,
    slot: &Arc<SessionSlot>,
) -> Result<Arc<SlotAgent>, String> {
    let _guard = slot.engine.start_lock.lock().await;
    slot.engine.touch();
    if let Some(h) = slot.acp_agent.read().await.clone() {
        if h.agent().await.is_some() {
            return Ok(h);
        }
    }
    let Some(params) = slot.acp_launch.lock().await.clone() else {
        return Err("this session is not set up for an agent — pick one in the model picker".into());
    };
    *slot.engine.phase.lock().await = AgentPhase::Starting;
    publish(state, slot).await;
    let result = crate::acp_session::start_agent(state, slot, &params, None).await;
    match &result {
        Ok(h) => {
            *slot.engine.phase.lock().await = AgentPhase::Idle;
            let _ = slot.events_tx.send(ServerMsg::AcpAgentStarted {
                kind: h.driver_kind.clone(),
                display_name: h.display_name.clone(),
                launch: h.launch.clone(),
                error: None,
            });
        }
        Err(e) => {
            *slot.engine.phase.lock().await = AgentPhase::Failed(e.clone());
            let _ = slot.events_tx.send(ServerMsg::AcpAgentStarted {
                kind: params.driver_kind.clone(),
                display_name: params.display_name(),
                launch: String::new(),
                error: Some(e.clone()),
            });
        }
    }
    publish(state, slot).await;
    result
}

/// Record the agent on the session, so the sidebar badges it and a reload
/// routes back to it with the same settings.
async fn remember_agent(slot: &SessionSlot, params: &AcpLaunchParams, active: bool) {
    let session = slot.session.read().await;
    session
        .set_agent(mira_harness::persist::AgentSessionMeta {
            driver_kind: params.driver_kind.clone(),
            model: params.model.clone(),
            active,
            launch: Some(params.to_persisted()),
        })
        .await;
}

/// Bound on what one handoff carries. Enough for the gist of a long
/// exchange; the tail wins because recent turns matter most.
const HANDOFF_MAX_CHARS: usize = 12_000;

/// Agent → provider: the agent's turns the harness has not seen, as a note.
async fn hand_agent_turns_to_harness(state: &AppState, slot: &SessionSlot, params: &AcpLaunchParams) {
    let Some(path) = state.store.as_ref().and_then(|s| s.agent_log_path(&slot.id)) else {
        return;
    };
    let lines = mira_acp::agent_sessions::read_lines(&path);
    // The replay marker for this switch, written before the handoff note
    // so replay places provider turns after it.
    let harness_len = slot.session.read().await.transcript().await.len();
    mira_acp::agent_sessions::append_line_to(
        &path,
        &serde_json::json!({
            "t": mira_harness::persist::now_ms(),
            "driver": params.driver_kind,
            "switch": { "harness_len": harness_len, "to": "provider" },
        }),
    );
    let mut handoff = slot.engine.handoff.lock().await;
    let from = handoff.harness_seen_agent.min(lines.len());
    handoff.harness_seen_agent = lines.len();
    let text = agent_lines_to_text(&lines[from..]);
    if text.trim().is_empty() {
        return;
    }
    let note = format!(
        "The user just switched this conversation from the external agent {} back to you. \
         What happened while it was driving (its tool calls ran on this same machine and \
         project):\n\n{}\n\nContinue from here.",
        params.display_name(),
        tail_chars(&text, HANDOFF_MAX_CHARS),
    );
    slot.session.read().await.push_note(note).await;
}

/// Provider → agent: the provider turns the agent has not seen, formatted to
/// sit in front of its next prompt. `None` when there is nothing to hand
/// over, or the agent already has it.
pub async fn take_context_for_agent(slot: &SessionSlot) -> Option<String> {
    let mut handoff = slot.engine.handoff.lock().await;
    if !handoff.agent_needs_context {
        return None;
    }
    handoff.agent_needs_context = false;
    let transcript = slot.session.read().await.transcript().await;
    let from = handoff.agent_seen_harness.min(transcript.len());
    handoff.agent_seen_harness = transcript.len();
    let mut text = String::new();
    for m in &transcript[from..] {
        let who = match m.role {
            mira_core::Role::User => "User",
            mira_core::Role::Assistant => "Assistant",
            _ => continue,
        };
        let body = m.content.as_deref().unwrap_or("").trim();
        if body.is_empty() {
            continue;
        }
        text.push_str(&format!("{who}: {body}\n\n"));
    }
    if text.trim().is_empty() {
        return None;
    }
    Some(format!(
        "<conversation_so_far>\nThis conversation started with another assistant; you are \
         taking over. What was said so far:\n\n{}</conversation_so_far>\n\n",
        tail_chars(&text, HANDOFF_MAX_CHARS),
    ))
}

/// Prompts and replies from sidecar lines, in order, as plain text.
fn agent_lines_to_text(lines: &[serde_json::Value]) -> String {
    let mut out = String::new();
    let mut reply = String::new();
    let mut tools: Vec<String> = Vec::new();
    let flush = |out: &mut String, reply: &mut String, tools: &mut Vec<String>| {
        if !tools.is_empty() {
            out.push_str(&format!("Agent used tools: {}\n\n", tools.join(", ")));
            tools.clear();
        }
        if !reply.trim().is_empty() {
            out.push_str(&format!("Agent: {}\n\n", reply.trim()));
        }
        reply.clear();
    };
    for line in lines {
        if let Some(text) = line.get("user").and_then(|u| u.get("text")).and_then(|t| t.as_str()) {
            flush(&mut out, &mut reply, &mut tools);
            out.push_str(&format!("User: {}\n\n", text.trim()));
            continue;
        }
        let Some(frame) = line.get("frame") else { continue };
        match frame.get("type").and_then(|t| t.as_str()) {
            Some("acp_text") => {
                if let Some(t) = frame.get("text").and_then(|t| t.as_str()) {
                    reply.push_str(t);
                }
            }
            Some("acp_tool_call") => {
                if let Some(title) = frame
                    .get("call")
                    .and_then(|c| c.get("title"))
                    .and_then(|t| t.as_str())
                    .filter(|t| !t.is_empty())
                {
                    if tools.len() < 12 {
                        tools.push(title.chars().take(80).collect());
                    }
                }
            }
            Some("acp_turn_end") => flush(&mut out, &mut reply, &mut tools),
            _ => {}
        }
    }
    flush(&mut out, &mut reply, &mut tools);
    out
}

/// The last `max` characters of `s`, on a char boundary, marked when cut.
fn tail_chars(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        return s.to_string();
    }
    let tail: String = s.chars().skip(count - max).collect();
    format!("[…earlier turns omitted…]\n{tail}")
}

/// Track the model an agent reports running, from its config-option frames.
/// Returns true when it changed, so the caller can republish the engine.
pub fn observe_frame(rt: &EngineRuntime, msg: &ServerMsg) -> bool {
    let ServerMsg::AcpConfigOptions { options } = msg else {
        return false;
    };
    let Some(current) = options
        .iter()
        .find(|o| o.category.as_deref() == Some("model"))
        .and_then(|o| o.current.clone())
    else {
        return false;
    };
    let Ok(mut m) = rt.reported_model.lock() else {
        return false;
    };
    if m.as_deref() == Some(current.as_str()) {
        return false;
    }
    *m = Some(current);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn agent_turns_read_as_a_plain_exchange() {
        let lines = vec![
            json!({"user": {"text": "what am i working on?"}}),
            json!({"frame": {"type": "acp_tool_call", "call": {"title": "ls ~"}}}),
            json!({"frame": {"type": "acp_text", "text": "Mostly "}}),
            json!({"frame": {"type": "acp_text", "text": "Mira."}}),
            json!({"frame": {"type": "acp_turn_end", "stop_reason": "end_turn"}}),
        ];
        let text = agent_lines_to_text(&lines);
        assert!(text.contains("User: what am i working on?"));
        assert!(text.contains("Agent used tools: ls ~"));
        assert!(text.contains("Agent: Mostly Mira."));
        // Order is kept: the question comes before its answer.
        assert!(text.find("User:").unwrap() < text.find("Agent: Mostly").unwrap());
    }

    #[test]
    fn long_handoffs_keep_the_most_recent_turns() {
        let s = format!("{}END", "x".repeat(50));
        let t = tail_chars(&s, 10);
        assert!(t.ends_with("END"));
        assert!(t.starts_with("[…earlier turns omitted…]"));
        assert_eq!(tail_chars("short", 10), "short");
    }

    #[test]
    fn the_reported_model_is_tracked_from_config_frames() {
        let rt = EngineRuntime::default();
        let frame = ServerMsg::AcpConfigOptions {
            options: vec![mira_acp::events::SessionConfigView {
                id: "model".into(),
                name: "Model".into(),
                description: None,
                category: Some("model".into()),
                current: Some("opus".into()),
                values: Vec::new(),
            }],
        };
        assert!(observe_frame(&rt, &frame), "first sighting is a change");
        assert!(!observe_frame(&rt, &frame), "the same model again is not");
        assert_eq!(rt.reported_model.lock().unwrap().as_deref(), Some("opus"));
    }
}
