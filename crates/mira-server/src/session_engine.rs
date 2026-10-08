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
    pub capabilities: mira_acp::runtime::RuntimeCapabilities,
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
    /// Set when the session moved from one agent to another: what the
    /// previous agent did, for the new one's next prompt.
    pub previous_agent: Option<String>,
    /// Facts the agent must hear before its next prompt, e.g. background
    /// work a restart cancelled. Delivered once, then dropped.
    pub notes: Vec<String>,
}

/// Per-slot engine runtime. Lives on [`SessionSlot`].
#[derive(Default)]
pub struct EngineRuntime {
    pub generation: std::sync::atomic::AtomicU64,
    pub retired: AtomicBool,
    pub activity: std::sync::Mutex<mira_acp::runtime::RuntimeActivity>,
    pub outbox_running: AtomicBool,
    pub dispatch_lock: tokio::sync::Mutex<()>,
    pub input_dispatch: tokio::sync::Mutex<()>,
    pub activity_changed: tokio::sync::Notify,
    pub provider_in_turn: AtomicBool,
    pub provider_generation: std::sync::atomic::AtomicU64,
    pub activity_publish: std::sync::Mutex<()>,
    /// Wake permissions already waiting when the chat posture changes.
    pub approval_mode_changed: tokio::sync::Notify,

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
    pub approval_scopes:
        std::sync::Mutex<std::collections::HashMap<String, crate::protocol::ApprovalScope>>,
    /// Standing "allow for this session" grants for agent tool calls.
    /// Mira's own rule, kept here — never a persistent grant handed to
    /// the agent (see `standing_target`): the agent is always answered
    /// with its narrowest one-shot option, and repeat calls match here
    /// first so it stops asking without ever gaining standing access.
    pub standing_approvals: std::sync::Mutex<Vec<StandingApproval>>,
    /// Unix seconds of the agent's last sign of life (a prompt or a frame),
    /// for the idle reaper.
    pub last_active: std::sync::atomic::AtomicU64,
    /// An agent turn is in flight: set when a prompt goes out, cleared when
    /// its turn ends. A window that opens the chat mid-turn reads it (via
    /// `Ready.running`) to show Stop rather than an idle composer.
    pub agent_in_turn: AtomicBool,
}

impl EngineRuntime {
    pub fn touch(&self) {
        self.last_active
            .store(now_secs(), std::sync::atomic::Ordering::Relaxed);
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A standing session grant for one agent tool call shape.
///
/// The target is what the user actually approved: the command for `bash`,
/// the path for file tools, the full arguments otherwise. Matching is
/// deliberately narrow — an identical call, not a category — because this
/// auto-answers without showing a card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StandingApproval {
    pub tool: String,
    pub target: String,
}

impl StandingApproval {
    pub fn allows(&self, tool: &str, target: &str) -> bool {
        self.tool == tool && self.target == target
    }
}

/// The stable target of an agent tool call view: command for `bash`,
/// path for file tools, the whole arguments object otherwise.
pub fn standing_target(tool: &str, args: &serde_json::Value) -> String {
    if tool == "bash" {
        if let Some(cmd) = args.get("command").and_then(|c| c.as_str()) {
            return cmd.to_string();
        }
    }
    if let Some(path) = args.get("path").and_then(|p| p.as_str()) {
        return path.to_string();
    }
    args.to_string()
}

/// Whether the slot already lets this agent call through without asking.
pub fn standing_allows(
    standing: &[StandingApproval],
    tool: &str,
    args: &serde_json::Value,
) -> bool {
    let target = standing_target(tool, args);
    standing.iter().any(|s| s.allows(tool, &target))
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
                if slot.is_running().await || slot.acp_agent.read().await.is_none() {
                    continue;
                }
                let idle = now.saturating_sub(
                    slot.engine
                        .last_active
                        .load(std::sync::atomic::Ordering::Relaxed),
                );
                if idle < REAP_AFTER.as_secs() {
                    continue;
                }
                let waiting = !slot.pending.lock().await.is_empty()
                    || !slot.prompt_pending.lock().await.is_empty();
                if waiting {
                    continue;
                }
                let Ok(_dispatch) = slot.engine.dispatch_lock.try_lock() else {
                    continue;
                };
                let Ok(_start) = slot.engine.start_lock.try_lock() else {
                    continue;
                };
                if slot.is_running().await
                    || !slot.pending.lock().await.is_empty()
                    || !slot.prompt_pending.lock().await.is_empty()
                {
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
        let (instance, _model, _small) = slot.selection.snapshot();
        let model = slot.session.read().await.config().await.model;
        let display_name = instance
            .as_deref()
            .and_then(|id| state.engines.current().get(id).and_then(|i| i.display_name.clone()))
            .or_else(|| instance.clone())
            .unwrap_or_else(|| "Mira".to_string());
        return SessionEngine {
            kind: EngineKind::Provider,
            capabilities: mira_acp::runtime::RuntimeCapabilities {
                steering: mira_acp::runtime::SteeringCapability::SafeBoundary,
                image_input: true, cancellation: true, live_model_switch: true, live_mode_switch: true,
                ..Default::default()
            },
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
    let reported = slot
        .engine
        .reported_model
        .lock()
        .ok()
        .and_then(|m| m.clone());
    let capabilities = match slot.acp_agent.read().await.clone() {
        Some(handle) => {
            let mut caps = handle.agent().await.map(|a| a.runtime_capabilities()).unwrap_or_default();
            if handle.opencode_control.is_some() { caps.steering = mira_acp::runtime::SteeringCapability::Native; }
            if handle.driver_kind == "grok" { caps.stop_behavior = mira_acp::runtime::StopBehavior::Runtime; }
            caps
        }
        None => Default::default(),
    };
    SessionEngine {
        kind: EngineKind::Agent,
        capabilities,
        instance: Some(params.instance.clone()),
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
    let _dispatch = slot.engine.dispatch_lock.lock().await;
    let _start = slot.engine.start_lock.lock().await;
    let previous = slot.acp_launch.lock().await.clone();
    let from_provider = previous.is_none();
    let same_setup = previous.as_ref().is_some_and(|p| p.same_agent(&params));

    // A different agent (or the same one reconfigured) replaces the old
    // process: two agents must never share one session's permissions.
    if !same_setup {
        crate::acp_session::stop_agent(slot).await;
    }
    if let Some(prev) = previous.as_ref().filter(|p| {
        p.driver_kind != params.driver_kind || p.instance != params.instance
    }) {
        hand_agent_turns_to_agent(state, slot, prev, &params).await;
    }
    if from_provider {
        slot.engine.handoff.lock().await.agent_needs_context = true;
        // Mark where the provider's turns stop, so a reload replays the two
        // histories interleaved in the order they happened rather than all
        // provider turns followed by all agent turns.
        if let Some(path) = state
            .store
            .as_ref()
            .and_then(|s| s.agent_log_path(&slot.id))
        {
            let harness_len = slot.session.read().await.transcript().await.len();
            mira_acp::agent_sessions::append_line_to(
                &path,
                &serde_json::json!({
                    "t": mira_harness::persist::now_ms(),
                    "driver": params.driver_kind,
                    "instance": params.instance,
                    "switch": {
                        "harness_len": harness_len,
                        "to": "agent",
                        "from_engine": provider_ref(state, slot, None).await,
                        "to_engine": agent_ref(&params),
                    },
                }),
            );
            slot.engine.handoff.lock().await.harness_seen_agent += 1;
        }
    }
    {
        let mut launch = slot.acp_launch.lock().await;
        if !same_setup { clear_reported_model(&slot.engine); }
        *launch = Some(params.clone());
    }
    *slot
        .selection
        .instance
        .write()
        .expect("selection lock poisoned") = Some(params.instance.clone());
    *slot
        .selection
        .model
        .write()
        .expect("selection lock poisoned") = params.model.clone();
    remember_agent(slot, &params, true).await;
    *slot.engine.phase.lock().await = AgentPhase::Starting;
    publish(state, slot).await;

    let state = state.clone();
    let slot = slot.clone();
    tokio::spawn(async move {
        let _ = ensure_agent(&state, &slot).await;
        crate::runtime_requests::wake_outbox(&state, &slot);
    });
}

/// Make a provider this session's engine, leaving any agent behind. The
/// agent's turns since the last switch are handed to the harness first, so
/// the provider continues the same conversation.
pub async fn select_provider(state: &AppState, slot: &Arc<SessionSlot>) {
    let _dispatch = slot.engine.dispatch_lock.lock().await;
    let _start = slot.engine.start_lock.lock().await;
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
        return Err(
            "this session is not set up for an agent — pick one in the model picker".into(),
        );
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
            instance: Some(params.instance.clone()),
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
async fn hand_agent_turns_to_harness(
    state: &AppState,
    slot: &SessionSlot,
    params: &AcpLaunchParams,
) {
    let Some(path) = state
        .store
        .as_ref()
        .and_then(|s| s.agent_log_path(&slot.id))
    else {
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
            "instance": params.instance,
            "switch": {
                "harness_len": harness_len,
                "to": "provider",
                "from_engine": agent_ref(params),
                "to_engine": provider_ref(state, slot, Some(params)).await,
            },
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

/// Agent → agent: what the previous agent did since the last switch, for
/// the new agent's next prompt. Provider turns from before either agent are
/// handed over again too, since the new agent never saw them.
async fn hand_agent_turns_to_agent(
    state: &AppState,
    slot: &SessionSlot,
    from: &AcpLaunchParams,
    to: &AcpLaunchParams,
) {
    let Some(path) = state
        .store
        .as_ref()
        .and_then(|s| s.agent_log_path(&slot.id))
    else {
        return;
    };
    let lines = mira_acp::agent_sessions::read_lines(&path);
    // The previous agent's stretch starts after the last engine switch.
    let start = lines
        .iter()
        .rposition(|l| l.get("switch").is_some())
        .map_or(0, |i| i + 1);
    let text = agent_lines_to_text(&lines[start..]);
    let harness_len = slot.session.read().await.transcript().await.len();
    mira_acp::agent_sessions::append_line_to(
        &path,
        &serde_json::json!({
            "t": mira_harness::persist::now_ms(),
            "driver": to.driver_kind,
            "instance": to.instance,
            "switch": {
                "harness_len": harness_len,
                "to": "agent",
                "from_engine": agent_ref(from),
                "to_engine": agent_ref(to),
            },
        }),
    );
    let mut handoff = slot.engine.handoff.lock().await;
    handoff.harness_seen_agent = lines.len() + 1;
    handoff.agent_seen_harness = 0;
    handoff.agent_needs_context = harness_len > 0;
    handoff.previous_agent = (!text.trim().is_empty()).then(|| {
        format!(
            "<previous_agent>\nThe user just switched this conversation to you from {}. \
             What happened while it was driving (its tool calls ran on this same machine \
             and project):\n\n{}</previous_agent>\n\n",
            from.display_name(),
            tail_chars(&text, HANDOFF_MAX_CHARS),
        )
    });
}

/// An agent, as a switch marker names it, so the transcript can label the
/// handoff with both engines after a reload.
fn agent_ref(params: &AcpLaunchParams) -> serde_json::Value {
    serde_json::json!({
        "kind": "agent",
        "driver": params.driver_kind,
        "instance": params.instance,
        "display_name": params.display_name(),
        "model": params.model,
    })
}

/// The session's provider, as a switch marker names it. `leaving` is the
/// agent being switched away from, whose instance the selection may still
/// hold.
async fn provider_ref(
    state: &AppState,
    slot: &SessionSlot,
    leaving: Option<&AcpLaunchParams>,
) -> serde_json::Value {
    let (instance, _model, _small) = slot.selection.snapshot();
    let instance = instance.filter(|i| leaving.is_none_or(|p| &p.instance != i));
    let model = slot.session.read().await.config().await.model;
    let display_name = instance
        .as_deref()
        .and_then(|id| state.engines.current().get(id).and_then(|i| i.display_name.clone()))
        .or_else(|| instance.clone())
        .unwrap_or_else(|| "Mira".to_string());
    serde_json::json!({
        "kind": "provider",
        "instance": instance,
        "display_name": display_name,
        "model": model,
    })
}

/// Queue a note for the agent's next prompt.
pub async fn note_for_agent(slot: &SessionSlot, note: String) {
    slot.engine.handoff.lock().await.notes.push(note);
}

/// Everything the agent missed, formatted to sit in front of its next
/// prompt: provider turns (provider → agent), the previous agent's turns
/// (agent → agent), and queued notes. `None` when there is nothing to hand
/// over, or the agent already has it.
pub async fn take_context_for_agent(slot: &SessionSlot) -> Option<String> {
    let mut handoff = slot.engine.handoff.lock().await;
    let mut out = String::new();
    if handoff.agent_needs_context {
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
        if !text.trim().is_empty() {
            out.push_str(&format!(
                "<conversation_so_far>\nThis conversation started with another assistant; you are \
                 taking over. What was said so far:\n\n{}</conversation_so_far>\n\n",
                tail_chars(&text, HANDOFF_MAX_CHARS),
            ));
        }
    }
    if let Some(previous) = handoff.previous_agent.take() {
        out.push_str(&previous);
    }
    if !handoff.notes.is_empty() {
        out.push_str(&format!(
            "<mira_note>\n{}\n</mira_note>\n\n",
            handoff.notes.drain(..).collect::<Vec<_>>().join("\n")
        ));
    }
    (!out.is_empty()).then_some(out)
}

/// What a restart cancelled that the agent may still expect to hear back
/// from. Bounded so it can't crowd out the prompt.
pub fn restart_note(cancelled: &[mira_acp::runtime::RuntimeWork]) -> Option<String> {
    const MAX: usize = 8;
    let labels: Vec<String> = cancelled
        .iter()
        .map(|w| {
            let kind = match w.kind {
                mira_acp::runtime::WorkKind::Goal => "goal",
                mira_acp::runtime::WorkKind::Task => "background task",
                mira_acp::runtime::WorkKind::Subagent => "subagent",
            };
            match w.title.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
                Some(t) => format!("- {kind}: {}", t.chars().take(80).collect::<String>()),
                None => format!("- {kind}"),
            }
        })
        .collect();
    if labels.is_empty() {
        return None;
    }
    let extra = labels.len().saturating_sub(MAX);
    let mut list = labels.into_iter().take(MAX).collect::<Vec<_>>().join("\n");
    if extra > 0 {
        list.push_str(&format!("\n- …and {extra} more"));
    }
    Some(format!(
        "Your process was restarted, which cancelled background work you started. It will \
         not report back:\n{list}\nRestart anything still needed. Commands started with \
         Mira's run_background were not affected."
    ))
}

/// Prompts and replies from sidecar lines, in order, as plain text.
pub(crate) fn agent_lines_to_text(lines: &[serde_json::Value]) -> String {
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
        if let Some(text) = line
            .get("user")
            .and_then(|u| u.get("text"))
            .and_then(|t| t.as_str())
        {
            flush(&mut out, &mut reply, &mut tools);
            out.push_str(&format!("User: {}\n\n", text.trim()));
            continue;
        }
        let Some(frame) = line.get("frame") else {
            continue;
        };
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

/// A model report belongs to the process that supplied it. Replacing that
/// process must clear the report before publishing the next agent selection.
fn clear_reported_model(runtime: &EngineRuntime) {
    *runtime.reported_model.lock().unwrap_or_else(|error| error.into_inner()) = None;
}

/// Track the model an agent reports running, from its config-option frames.
/// Returns true when it changed, so the caller can republish the engine.
pub fn observe_frame(rt: &EngineRuntime, msg: &ServerMsg, active_driver: Option<&str>) -> bool {
    let ServerMsg::AcpConfigOptions { options, driver } = msg else {
        return false;
    };
    if active_driver.is_none() || driver.as_deref().is_some_and(|driver| Some(driver) != active_driver) { return false; }
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
    fn a_restart_note_lists_cancelled_work_and_stays_bounded() {
        use mira_acp::runtime::{RuntimeWork, WorkKind, WorkStatus};
        let work = |i: usize| RuntimeWork {
            id: format!("w{i}"),
            native_thread_id: None,
            kind: WorkKind::Subagent,
            status: WorkStatus::Cancelled,
            title: Some(format!("review pass {i}")),
        };
        assert!(restart_note(&[]).is_none());
        let note = restart_note(&(0..11).map(work).collect::<Vec<_>>()).unwrap();
        assert!(note.contains("subagent: review pass 0"));
        assert!(!note.contains("review pass 9"));
        assert!(note.contains("and 3 more"));
        assert!(note.contains("run_background"));
    }

    #[test]
    fn standing_rules_match_commands_paths_and_exact_calls() {
        let bash = StandingApproval {
            tool: "bash".into(),
            target: "npm test".into(),
        };
        assert!(bash.allows("bash", "npm test"));
        assert!(!bash.allows("bash", "npm run lint"));
        assert!(!bash.allows("edit_file", "npm test"));

        // File tools key on path, not content: approving an edit to a
        // file covers that file, the way Mira's own policy rules do.
        let args = json!({"path": "src/a.rs", "old_string": "x", "new_string": "y"});
        assert_eq!(standing_target("edit_file", &args), "src/a.rs");
        let same_path = json!({"path": "src/a.rs", "old_string": "DIFFERENT"});
        assert!(standing_allows(
            &[StandingApproval {
                tool: "edit_file".into(),
                target: "src/a.rs".into()
            }],
            "edit_file",
            &same_path,
        ));
        // ...but not another file, and not another tool on the same path.
        assert!(!standing_allows(
            &[StandingApproval {
                tool: "edit_file".into(),
                target: "src/a.rs".into()
            }],
            "edit_file",
            &json!({"path": "src/b.rs"}),
        ));
        assert!(!standing_allows(
            &[StandingApproval {
                tool: "edit_file".into(),
                target: "src/a.rs".into()
            }],
            "write_file",
            &same_path,
        ));
        // Unknown shapes fall back to the whole arguments object.
        let odd = json!({"frobnicate": true});
        assert_eq!(standing_target("weird_tool", &odd), odd.to_string());
    }

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
    fn replacing_an_agent_drops_its_reported_model() {
        let runtime = EngineRuntime::default();
        *runtime.reported_model.lock().unwrap() = Some("gpt-6.1-sol".into());
        clear_reported_model(&runtime);
        assert_eq!(*runtime.reported_model.lock().unwrap(), None);
        let old = ServerMsg::AcpConfigOptions { driver:Some("codex".into()), options:vec![mira_acp::events::SessionConfigView { id:"model".into(), name:"Model".into(), description:None, category:Some("model".into()), current:Some("gpt-6.1-sol".into()), values:vec![] }] };
        assert!(!observe_frame(&runtime, &old, Some("opencode")), "queued old-agent reports must not restore its model");
        assert_eq!(*runtime.reported_model.lock().unwrap(), None);
        let frame = ServerMsg::AcpConfigOptions { driver: Some("opencode".into()), options: vec![mira_acp::events::SessionConfigView { id:"model".into(), name:"Model".into(), description:None, category:Some("model".into()), current:Some("opencode/default".into()), values:vec![] }] };
        assert!(observe_frame(&runtime, &frame, Some("opencode")));
        assert_eq!(runtime.reported_model.lock().unwrap().as_deref(), Some("opencode/default"));
    }

    #[test]
    fn the_reported_model_is_tracked_from_config_frames() {
        let rt = EngineRuntime::default();
        let frame = ServerMsg::AcpConfigOptions {
            driver: None,
            options: vec![mira_acp::events::SessionConfigView {
                id: "model".into(),
                name: "Model".into(),
                description: None,
                category: Some("model".into()),
                current: Some("opus".into()),
                values: Vec::new(),
            }],
        };
        assert!(observe_frame(&rt, &frame, Some("claude-code")), "first sighting is a change");
        assert!(!observe_frame(&rt, &frame, Some("claude-code")), "the same model again is not");
        assert_eq!(rt.reported_model.lock().unwrap().as_deref(), Some("opus"));
    }
}
