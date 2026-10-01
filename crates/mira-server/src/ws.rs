//! WebSocket handler.
//!
//! One task per connection reads client frames; another forwards frames
//! from the *currently-attached* slot's broadcast bus to the same socket.
//! The two halves close together when either side hangs up.
//!
//! ## Attach model
//!
//! Every connection starts out attached to the server's `active` session.
//! A client can move to a different session by sending
//! `ClientMsg::Attach { session_id }`; the reader validates the id,
//! updates its per-connection attached slot, hands the new subscription
//! to the forwarder via a small channel, and pushes a fresh Ready.
//!
//! Every dispatch action (Send, Approve, SetModel, Interrupt, …)
//! resolves the CURRENTLY attached slot, so a WS attached to session A
//! never mutates session B — even if the process-wide `state.active`
//! points somewhere else because a different tab attached to a
//! different session first.

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use futures::{SinkExt, StreamExt};
use mira_config::RuntimeState;
use tokio::sync::{broadcast, mpsc, RwLock};
use tracing::{debug, info, warn};

use crate::approver;
use crate::protocol::{ClientMsg, ServerMsg};
use crate::slot::{AttachGuard, SessionSlot};
use crate::state::AppState;
use crate::title;

pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

/// Per-connection attach state. Shared between the reader (which mutates
/// on `Attach`) and the forwarder (which re-subscribes when the slot
/// changes). Held as an Arc<RwLock<...>> so both halves see the current
/// slot without message-passing every frame.
struct AttachState {
    slot: Arc<SessionSlot>,
    _guard: AttachGuard,
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    info!("ws: client connected");
    let (mut sink, mut stream) = socket.split();

    // Attach to the server's currently-active slot at connect time.
    let initial_slot = state.active_slot().await;
    let attach = Arc::new(RwLock::new(AttachState {
        _guard: AttachGuard::new(&initial_slot),
        slot: initial_slot.clone(),
    }));

    // Send the initial Ready before the forwarder loop starts so no
    // between-snapshot frames get lost.
    let ready = build_ready(&initial_slot, &state).await;
    if send_json(&mut sink, &ready).await.is_err() {
        return;
    }

    // Channel the reader uses to hand new broadcast receivers to the
    // forwarder when the client attaches to a different slot.
    let (rx_swap_tx, mut rx_swap_rx) = mpsc::channel::<broadcast::Receiver<ServerMsg>>(4);

    // Forwarder task: broadcast → socket, with hot-swappable subscription.
    let attach_forwarder = attach.clone();
    let mut forwarder = tokio::spawn(async move {
        let mut rx = attach_forwarder.read().await.slot.events_tx.subscribe();
        loop {
            tokio::select! {
                Some(new_rx) = rx_swap_rx.recv() => {
                    rx = new_rx;
                }
                res = rx.recv() => {
                    match res {
                        Ok(frame) => {
                            if send_json(&mut sink, &frame).await.is_err() {
                                break;
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            warn!(dropped = n, "ws: broadcast lagged");
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
                else => break,
            }
        }
    });

    // Reader: socket → server actions on the currently-attached slot.
    let reader_state = state.clone();
    let reader_attach = attach.clone();
    let reader_swap = rx_swap_tx.clone();
    let mut reader = tokio::spawn(async move {
        while let Some(msg) = stream.next().await {
            let text = match msg {
                Ok(Message::Text(t)) => t,
                Ok(Message::Close(_)) => break,
                Ok(_) => continue,
                Err(e) => {
                    warn!(%e, "ws: read error");
                    break;
                }
            };
            match serde_json::from_str::<ClientMsg>(&text) {
                Ok(cmd) => {
                    dispatch(cmd, &reader_state, &reader_attach, &reader_swap).await;
                }
                Err(e) => {
                    // Route error to whichever slot the client is watching;
                    // otherwise it appears in nobody's transcript.
                    let slot = reader_attach.read().await.slot.clone();
                    let _ = slot.events_tx.send(ServerMsg::Error {
                        text: format!("bad frame: {e}"),
                    });
                }
            }
        }
    });

    tokio::select! {
        _ = &mut forwarder => { reader.abort(); }
        _ = &mut reader    => { forwarder.abort(); }
    }
    info!("ws: client disconnected");
}

pub(crate) async fn build_ready(slot: &SessionSlot, state: &AppState) -> ServerMsg {
    let sess = slot.session.read().await.clone();
    let cfg = sess.config().await;
    let mode = state.policy.lock().await.mode();
    // Everything, including what compaction summarized (shown behind a
    // divider); the model itself only sees `history()`.
    let history = sess.transcript().await;
    let turns = sess.turns().await;
    let usage = sess.usage().await;
    let tasks = sess.tasks().await;
    let goal = sess.goal().await;
    let previews = sess.previews().await;
    let cwd = slot.cwd.read().await.clone();
    let (agent_transcript, agent_driver) =
        crate::sessions::read_agent_state(state.store.as_ref(), &sess).await;
    let agent_kind = slot
        .acp_agent
        .read()
        .await
        .clone()
        .map(|h| h.driver_kind.clone());
    let (instance, _model, _small) = state.selection.snapshot();
    // The session's *configured* agent — inherited from the previous
    // chat or retained while running — when none is live. The client
    // shows the setup and the first prompt boots it.
    let agent_configured = if slot.acp_agent.read().await.is_none() {
        slot.acp_launch.lock().await.as_ref().map(|p| p.driver_kind.clone())
    } else {
        None
    };
    ServerMsg::Ready {
        session_id: sess.id.to_string(),
        model: cfg.model,
        mode,
        cwd: cwd.display().to_string(),
        history,
        turns,
        usage,
        tasks,
        goal,
        previews,
        agent_transcript,
        agent_driver,
        agent_kind,
        instance,
        agent_configured,
        title: sess.title().await,
        engine: crate::session_engine::current(state, slot).await,
    }
}

async fn dispatch(
    cmd: ClientMsg,
    state: &AppState,
    attach: &Arc<RwLock<AttachState>>,
    rx_swap: &mpsc::Sender<broadcast::Receiver<ServerMsg>>,
) {
    // Common: whenever we're about to act on the "current" slot, take a
    // cheap Arc clone once so we don't re-lock for every field.
    let slot = attach.read().await.slot.clone();

    match cmd {
        ClientMsg::Send { text, images } => {
            debug!(len = text.len(), images = images.len(), "ws: send");
            // Engine routing, the same rule the model selection follows:
            // a session configured for an external agent (started here,
            // or inherited from the previous chat) gets the prompt, and
            // the agent boots on first use. Everything else runs the
            // native harness. Which engine serves a session is *its*
            // property — two sessions can run two agents at once.
            let configured_for_agent = slot.acp_launch.lock().await.is_some();
            if configured_for_agent {
                prompt_agent(state, &slot, text, images).await;
            } else {
                spawn_turn(state.clone(), slot, text, images).await;
            }
        }
        ClientMsg::Resend {
            original,
            occurrence,
            text,
        } => {
            // Same engine routing as `Send`: on an agent-configured
            // session the resend is a fresh prompt to that agent — the
            // native rewind doesn't apply to a transcript the agent
            // owns. (The original images were staged into the agent's
            // files dir and can't be re-staged, so text only.)
            let configured_for_agent = slot.acp_launch.lock().await.is_some();
            if configured_for_agent {
                prompt_agent(state, &slot, text, Vec::new()).await;
                return;
            }
            let sess = slot.session.read().await.clone();
            if let Some(images) = sess.rewind_to_user(&original, occurrence).await {
                // Edits keep the original message's images.
                spawn_turn(state.clone(), slot, text, images).await;
            } else {
                let _ = slot.events_tx.send(ServerMsg::Warning {
                    text: "can't edit that message — a turn is running, or it was \
                           summarized away by compaction"
                        .into(),
                });
                let _ = slot.events_tx.send(ServerMsg::Done);
            }
        }
        ClientMsg::Approve {
            call_id,
            allow,
            scope,
        } => {
            // Recorded before the approval resolves: an agent's gate wakes
            // on resolve and reads the scope to decide what rules to adopt.
            if allow {
                if let Ok(mut m) = slot.engine.approval_scopes.lock() {
                    m.insert(call_id.clone(), scope);
                }
            }
            match approver::resolve(&slot.pending, &call_id, allow).await {
            Some(call) => {
                if allow && !matches!(scope, crate::protocol::ApprovalScope::Once) {
                    apply_scope_widening(state, &slot, &call, scope).await;
                }
            }
            None => {
                if let Ok(mut m) = slot.engine.approval_scopes.lock() {
                    m.remove(&call_id);
                }
                warn!(call_id, "approval for unknown call")
            }
        }
        }
        ClientMsg::PromptResponse {
            prompt_id,
            response,
        } => {
            if !crate::interactive::resolve(&slot.prompt_pending, &prompt_id, response).await {
                warn!(prompt_id, "prompt response for unknown id");
            }
        }
        ClientMsg::Environment { target: None } => {
            let _ = slot.events_tx.send(ServerMsg::EnvironmentStatus {
                status: slot.environments.status().await,
                environments: slot.environments.list(),
            });
        }
        ClientMsg::Environment {
            target: Some(target),
        } => {
            let busy = slot.session.read().await.is_busy().await;
            if busy || slot.environments.is_switching() {
                // Answer with a failed switch (not a bare warning) so the
                // client's "switching…" state always resolves.
                let status = slot.environments.status().await;
                let _ = slot.events_tx.send(ServerMsg::EnvironmentSwitched {
                    from: status.current.clone(),
                    to: target,
                    lines: Vec::new(),
                    conflicts: Vec::new(),
                    error: Some(if busy {
                        "wait for the current turn to finish before switching environments".into()
                    } else {
                        "an environment switch is already in progress".into()
                    }),
                    status,
                });
                return;
            }
            crate::slot::spawn_environment_switch(slot.clone(), target);
        }
        // -------- external ACP agents --------
        ClientMsg::AcpStart {
            instance,
            driver,
            binary_path,
            display_name,
            launch_args,
            env,
            api_key,
            home_path,
            effort,
            setting_sources,
            resume,
            model,
        } => {
            // Picking an agent makes it this session's engine at once — the
            // composer switches before anything is spawned — and the agent
            // comes up in the background, so a slow Node adapter never holds
            // up the socket and a missing binary is reported immediately.
            //
            // Resolution order (see `resolve_start_params`): an engine
            // instance wins; then the launch settings this session
            // inherited from the previous chat — used when the client
            // names the same driver or none at all, which is how a new
            // chat starts the same agent with the same config; then the
            // legacy bare-driver-with-defaults path. The per-field
            // overrides the client sends layer on top of whatever base
            // resolved.
            let recorded = slot.acp_launch.lock().await.clone();
            let instance_cfg = instance.as_deref().and_then(|id| {
                state.engines.external_driver_config(id).map(|c| {
                    let kind = state
                        .engines
                        .get(id)
                        .map(|i| i.driver.to_string())
                        .unwrap_or_else(|| id.to_string());
                    (kind, c)
                })
            });
            let same_driver_as_recorded = recorded
                .as_ref()
                .is_some_and(|r| driver.as_deref().is_none_or(|d| d == r.driver_kind));
            let (base_kind, mut cfg) = match crate::acp_session::resolve_start_params(
                recorded.clone(),
                instance_cfg,
                driver.as_deref(),
            ) {
                Ok(resolved) => resolved,
                Err(e) => {
                    let _ = slot.events_tx.send(ServerMsg::AcpAgentStarted {
                        kind: driver.clone().unwrap_or_default(),
                        display_name: driver.clone().unwrap_or_default(),
                        launch: String::new(),
                        error: Some(e),
                    });
                    return;
                }
            };
            if display_name.is_some() {
                cfg.display_name = display_name;
            }
            if binary_path.is_some() {
                cfg.binary_path = binary_path.map(std::path::PathBuf::from);
            }
            if !launch_args.is_empty() {
                cfg.launch_args = launch_args;
            }
            if !env.is_empty() {
                cfg.env.extend(env);
            }
            if api_key.is_some() {
                cfg.api_key = api_key;
            }
            if home_path.is_some() {
                cfg.home_path = home_path.map(std::path::PathBuf::from);
            }
            if effort.is_some() {
                cfg.effort = effort;
            }
            if setting_sources.is_some() {
                cfg.setting_sources = setting_sources;
            }
            cfg.enabled = true;
            if mira_acp::drivers::by_kind(&base_kind).is_none() {
                let _ = slot.events_tx.send(ServerMsg::AcpAgentStarted {
                    kind: base_kind.clone(),
                    display_name: base_kind.clone(),
                    launch: String::new(),
                    error: Some(format!("unknown agent: {base_kind}")),
                });
                return;
            }
            let mut params = crate::acp_session::AcpLaunchParams::new(base_kind.clone(), cfg);
            // Re-picking the same agent keeps the standing choices made for
            // it in this session (its model and mode) unless a new model
            // was picked with it.
            if same_driver_as_recorded {
                if let Some(r) = &recorded {
                    params.model = r.model.clone();
                    params.mode_id = r.mode_id.clone();
                }
            }
            if model.is_some() {
                params.model = model;
            }
            // An explicit resume (history import) is recorded as this
            // session's cursor, so the start picks it up like any other
            // resume; the running process (if any) must make way for it.
            if let Some(sid) = resume {
                mira_acp::agent_sessions::record(&slot.id.to_string(), &base_kind, &sid);
                crate::acp_session::stop_agent(&slot).await;
            }
            crate::session_engine::select_agent(state, &slot, params).await;
        }

        ClientMsg::AcpSetMode {
            mode_id,
            acknowledge_privileged,
        } => {
            let Some(handle) = slot.acp_agent.read().await.clone() else {
                // Set up but not running yet: the mode is remembered and
                // applied when the agent starts, like a model picked early.
                let mut launch = slot.acp_launch.lock().await;
                if let Some(p) = launch.as_mut() {
                    p.mode_id = Some(mode_id);
                    drop(launch);
                    crate::session_engine::publish(state, &slot).await;
                } else {
                    let _ = slot.events_tx.send(ServerMsg::Error {
                        text: "this session is not using an agent".into(),
                    });
                }
                return;
            };
            let Some(agent) = handle.agent().await else { return };

            // A mode that grants more than Mira's own approval pipeline
            // would is a standing change to the agent's authority, so it
            // needs the user to have been told what it does. The client can
            // set the flag, which is why the gate is "was this explained and
            // agreed" rather than "is this a privileged mode" — but it
            // cannot grant the authority without a round trip that showed
            // the consequence.
            let privileged = handle
                .privileged_mode_reason(&mode_id)
                .is_some();
            if privileged && !acknowledge_privileged {
                let _ = slot.events_tx.send(ServerMsg::AcpPrivilegedModeConfirmation {
                    kind: handle.driver_kind.clone(),
                    display_name: handle.display_name.clone(),
                    mode_id: mode_id.clone(),
                    mode_name: mode_id.clone(),
                    reason: handle
                        .privileged_mode_reason(&mode_id)
                        .unwrap_or_default()
                        .to_string(),
                });
                return;
            }

            // The choice is the session's, not just this process's: a later
            // restart for any reason keeps it.
            if let Some(p) = slot.acp_launch.lock().await.as_mut() {
                p.mode_id = Some(mode_id.clone());
            }
            // Claude Code takes the change live over its control protocol;
            // only a transport without one (Codex app-server) relaunches,
            // resuming the session, rather than erroring.
            if !agent.changes_live() {
                let events = slot.events_tx.clone();
                let slot_c = slot.clone();
                let state_c = state.clone();
                tokio::spawn(async move {
                    match crate::acp_session::restart_native_agent(
                        &state_c,
                        &slot_c,
                        mira_acp::native::NativeOverrides {
                            permission_mode: Some(mode_id.clone()),
                            ..Default::default()
                        },
                    )
                    .await
                    {
                        Ok(h) => {
                            let _ = events.send(ServerMsg::AcpAgentStarted {
                                kind: h.driver_kind.clone(),
                                display_name: h.display_name.clone(),
                                launch: h.launch.clone(),
                                error: None,
                            });
                            let _ = events.send(ServerMsg::AcpModeChanged {
                                kind: h.driver_kind.clone(),
                                display_name: h.display_name.clone(),
                                mode_id: mode_id.clone(),
                                mode_name: mode_id.clone(),
                                privileged,
                            });
                        }
                        Err(e) => {
                            let _ = events.send(ServerMsg::Error {
                                text: format!("could not change agent mode: {e}"),
                            });
                        }
                    }
                });
                return;
            }

            let native = matches!(agent, mira_acp::native::AgentHandle::Native(_));
            if native || agent.session_id().await.is_some() {
                if let Err(e) = agent.set_mode(&mode_id).await {
                    let _ = slot.events_tx.send(ServerMsg::Error {
                        text: format!("could not set agent mode: {e}"),
                    });
                    return;
                }
                // Recorded regardless of privilege: a standing change to what
                // the agent may do belongs in the transcript, not only in a
                // dropdown the user may not have open.
                let _ = slot.events_tx.send(ServerMsg::AcpModeChanged {
                    kind: handle.driver_kind.clone(),
                    display_name: handle.display_name.clone(),
                    mode_id: mode_id.clone(),
                    mode_name: mode_id.clone(),
                    privileged,
                });
            }
        }

        ClientMsg::AcpSetConfigOption { option_id, value } => {
            // The model is a standing choice for the session: remembered
            // first, so it survives restarts and applies to an agent that
            // has not started yet.
            let is_model = option_id == "model" || {
                let launch = slot.acp_launch.lock().await;
                launch.is_some() && option_id.to_ascii_lowercase().contains("model")
            };
            if is_model {
                if let Some(p) = slot.acp_launch.lock().await.as_mut() {
                    p.model = Some(value.clone());
                }
            }
            let Some(handle) = slot.acp_agent.read().await.clone() else {
                if slot.acp_launch.lock().await.is_some() {
                    crate::session_engine::publish(state, &slot).await;
                } else {
                    let _ = slot.events_tx.send(ServerMsg::Error {
                        text: "this session is not using an agent".into(),
                    });
                }
                return;
            };
            let Some(agent) = handle.agent().await else { return };
            // Same relaunch treatment for the model: a native agent takes
            // `--model` at spawn, and only the model option is relaunchable —
            // anything else keeps the honest error rather than pretending.
            if !agent.changes_live() && option_id == "model" {
                let events = slot.events_tx.clone();
                let slot_c = slot.clone();
                let state_c = state.clone();
                tokio::spawn(async move {
                    match crate::acp_session::restart_native_agent(
                        &state_c,
                        &slot_c,
                        mira_acp::native::NativeOverrides {
                            model: Some(value.clone()),
                            ..Default::default()
                        },
                    )
                    .await
                    {
                        Ok(h) => {
                            let _ = events.send(ServerMsg::AcpAgentStarted {
                                kind: h.driver_kind.clone(),
                                display_name: h.display_name.clone(),
                                launch: h.launch.clone(),
                                error: None,
                            });
                        }
                        Err(e) => {
                            let _ = events.send(ServerMsg::Error {
                                text: format!("could not set {option_id}: {e}"),
                            });
                        }
                    }
                });
                return;
            }
            let native = matches!(agent, mira_acp::native::AgentHandle::Native(_));
            if native || agent.session_id().await.is_some() {
                let v = serde_json::Value::String(value);
                match agent.set_config_option(&option_id, v).await {
                    Ok(outcome) => {
                        // The agent returns its refreshed option list, and a
                        // change usually moves `currentValue`. Forwarding it
                        // keeps the picker's selected value honest instead of
                        // leaving it showing the pre-change model.
                        if !outcome.is_empty() {
                            let _ = slot.events_tx.send(ServerMsg::AcpConfigOptions {
                                options: outcome,
                            });
                        }
                    }
                    Err(e) => {
                        let _ = slot.events_tx.send(ServerMsg::Error {
                            text: format!("could not set {option_id}: {e}"),
                        });
                    }
                }
            }
        }

        ClientMsg::AcpPrompt { text, images } => {
            prompt_agent(state, &slot, text, images).await;
        }

        ClientMsg::AcpStop => {
            // Leaving the agent is a decision about the session's engine,
            // not just the process: the provider takes over, with the
            // agent's turns handed to it so the conversation continues.
            crate::session_engine::select_provider(state, &slot).await;
        }

        ClientMsg::AcpFork => {
            // Fork continues the history under a new session id. Only Claude
            // Code has the flag; anything else is told so, not restarted
            // into amnesia.
            let Some(handle) = slot.acp_agent.read().await.clone() else {
                let _ = slot.events_tx.send(ServerMsg::Error {
                    text: "no external agent is running for this session".into(),
                });
                return;
            };
            if handle.driver_kind != "claude-code" {
                let _ = slot.events_tx.send(ServerMsg::Error {
                    text: format!(
                        "{} has no fork operation — fork is Claude Code only",
                        handle.display_name
                    ),
                });
                return;
            }
            let Some(agent) = handle.agent().await else {
                let _ = slot.events_tx.send(ServerMsg::Error {
                    text: "the external agent has stopped".into(),
                });
                return;
            };
            let sid = agent.session_id().await;
            let Some(sid) = sid else {
                let _ = slot.events_tx.send(ServerMsg::Error {
                    text: "the agent has no session to fork yet — send a prompt first".into(),
                });
                return;
            };
            let events = slot.events_tx.clone();
            let slot_c = slot.clone();
            let state_c = state.clone();
            tokio::spawn(async move {
                match crate::acp_session::restart_native_agent(
                    &state_c,
                    &slot_c,
                    mira_acp::native::NativeOverrides {
                        resume: Some(sid),
                        fork: true,
                        ..Default::default()
                    },
                )
                .await
                {
                    Ok(h) => {
                        let _ = events.send(ServerMsg::AcpAgentStarted {
                            kind: h.driver_kind.clone(),
                            display_name: h.display_name.clone(),
                            launch: h.launch.clone(),
                            error: None,
                        });
                    }
                    Err(e) => {
                        let _ = events.send(ServerMsg::Error {
                            text: format!("could not fork the agent session: {e}"),
                        });
                    }
                }
            });
        }

        ClientMsg::AcpCompact => {
            let Some(handle) = slot.acp_agent.read().await.clone() else {
                let _ = slot.events_tx.send(ServerMsg::Error {
                    text: "no external agent is running for this session".into(),
                });
                return;
            };
            let Some(agent) = handle.agent().await else {
                let _ = slot.events_tx.send(ServerMsg::Error {
                    text: "the external agent has stopped".into(),
                });
                return;
            };
            let events = slot.events_tx.clone();
            let turn_slot = slot.clone();
            tokio::spawn(async move {
                match agent {
                    mira_acp::native::AgentHandle::Native(n) => {
                        // Compaction is just a prompt: the transcript shows
                        // what happened instead of silence with side effects.
                        match n.prompt("/compact").await {
                            Ok(()) => {}
                            Err(e) => {
                                let _ = events.send(ServerMsg::Error {
                                    text: format!("compaction failed: {e}"),
                                });
                            }
                        }
                    }
                    mira_acp::native::AgentHandle::AppServer(a) => {
                        match a.compact().await {
                            Ok(()) => {
                                let _ = events.send(ServerMsg::Warning {
                                    text: "Codex is compacting its thread".into(),
                                });
                            }
                            Err(e) => {
                                let _ = events.send(ServerMsg::Error {
                                    text: format!("compaction failed: {e}"),
                                });
                            }
                        }
                    }
                    _ => {
                        let _ = events.send(ServerMsg::Error {
                            text: "this agent has no compaction operation".into(),
                        });
                    }
                }
                let _ = turn_slot;
            });
        }

        ClientMsg::AcpStatus => {
            // Probing spawns each agent, so it runs off the read loop.
            // Instances from the engine registry are probed with their
            // own config (binary path, env, home); driver kinds no
            // instance covers are probed with defaults so the list
            // still shows every agent this build knows.
            let events = slot.events_tx.clone();
            let engines = state.engines.clone();
            tokio::spawn(async move {
                let mut agents = Vec::new();
                let mut covered = std::collections::BTreeSet::new();
                for inst in engines.instances().filter(|i| !i.is_native()) {
                    let Some(d) = mira_acp::drivers::by_kind(inst.driver.as_str()) else {
                        continue;
                    };
                    covered.insert(inst.driver.as_str().to_string());
                    agents.push(
                        mira_acp::probe(
                            d.as_ref(),
                            &mira_engine::driver_config_for(inst),
                            Default::default(),
                        )
                        .await,
                    );
                }
                for d in mira_acp::drivers::all() {
                    if !covered.contains(d.kind()) {
                        agents.push(
                            mira_acp::probe(d.as_ref(), &Default::default(), Default::default())
                                .await,
                        );
                    }
                }
                let _ = events.send(ServerMsg::AcpAgentStatus { agents });
            });
        }

        ClientMsg::SetModelOption { id, value } => {
            // `off` / `auto` are the sentinels meaning "omit the field
            // entirely", which is what non-reasoning models and the
            // default service tier need.
            let normalize = |v: &str| -> Option<String> {
                match v {
                    "" | "off" | "auto" => None,
                    other => Some(other.to_string()),
                }
            };
            let session = slot.session.read().await;
            match id.as_str() {
                "reasoning_effort" => session.set_reasoning_effort(normalize(&value)).await,
                "service_tier" => session.set_service_tier(normalize(&value)).await,
                // Unknown id: log and drop. Accepting arbitrary ids here
                // would let a client write to any config field by name.
                other => debug!(
                    "ws: ignoring unknown model option id {other:?} (value {value:?})"
                ),
            }
        }
        ClientMsg::SetModel { model, instance } => {
            apply_model_selection(state.clone(), slot.clone(), instance, model).await;
        }
        ClientMsg::SetMode { mode } => {
            state.policy.lock().await.set_mode(mode);
            let repo_root = state.sandbox.profile().repo_root.clone();
            let profile = mira_harness::profile_for_mode(mode, &repo_root);
            // Propagate to EVERY slot's session so a background session's
            // next bash call also honors the new profile. Doing this per
            // slot is cheap (a handful of Arc<RwLock<..>> reads).
            for slot_arc in state.list_slots().await {
                slot_arc
                    .session
                    .read()
                    .await
                    .set_sandbox_profile(profile.clone())
                    .await;
                let _ = slot_arc.events_tx.send(ServerMsg::ModeChanged { mode });
            }
        }
        ClientMsg::SetEffort { effort } => {
            let normalized = effort
                .as_deref()
                .filter(|v| !v.is_empty() && *v != "off")
                .map(|s| s.to_owned());
            slot.session
                .read()
                .await
                .set_reasoning_effort(normalized)
                .await;
        }
        ClientMsg::Interrupt => {
            // Which engine is serving decides what "stop" means. The
            // agent announces its own interrupted turn end through its
            // pump, so the native Done/warning pair is skipped when an
            // agent is live — ending the turn twice reads as a hang.
            let agent = slot.acp_agent.read().await.clone();
            let agent_live = match &agent {
                Some(h) => h.agent().await.is_some(),
                None => false,
            };
            if agent_live {
                if let Some(h) = &agent {
                    h.cancel_current_turn().await;
                }
                let denied = approver::drain_pending_as_denied(&slot.pending).await;
                let text = if denied > 0 {
                    format!("turn interrupted (denied {denied} pending approval(s))")
                } else {
                    "turn interrupted".into()
                };
                let _ = slot.events_tx.send(ServerMsg::Warning { text });
                return;
            }
            let sess = slot.session.read().await.clone();
            let cancelled = sess.cancel().await;
            let denied = approver::drain_pending_as_denied(&slot.pending).await;
            let text = if cancelled {
                if denied > 0 {
                    format!("turn interrupted (denied {denied} pending approval(s))")
                } else {
                    "turn interrupted".into()
                }
            } else {
                "nothing to interrupt".into()
            };
            sess.end_current_turn().await;
            let _ = slot.events_tx.send(ServerMsg::Warning { text });
            let _ = slot.events_tx.send(ServerMsg::Done);
            // Abort the JoinHandle too so its tokio task doesn't keep
            // pumping tokens into a channel with no interested listener.
            if let Some(h) = slot.turn.lock().await.take() {
                h.abort();
            }
        }
        ClientMsg::SetGoal {
            condition,
            max_iterations,
            evaluator_model,
            verify,
            budget_tokens,
            budget_usd,
        } => {
            let condition = condition.trim().to_owned();
            if condition.is_empty() {
                let _ = slot.events_tx.send(ServerMsg::Warning {
                    text: "goal condition cannot be empty".into(),
                });
                return;
            }
            let mut goal = mira_harness::Goal::new(condition.clone());
            if let Some(n) = max_iterations {
                goal = goal.with_max_iterations(n);
            }
            goal = goal.with_evaluator_model(evaluator_model);
            goal = goal.with_verify(verify);
            goal = goal.with_budget_tokens(budget_tokens);
            goal = goal.with_budget_usd(budget_usd);
            slot.session.read().await.set_goal(goal.clone()).await;
            let _ = slot.events_tx.send(ServerMsg::GoalSet { goal });
            let kickoff = format!(
                "Start working toward this standing goal:\n\n{condition}\n\n\
                 Plan briefly if needed, then take the first concrete step. \
                 I'll keep looping automatically until an evaluator agrees \
                 the goal is met.",
            );
            spawn_turn(state.clone(), slot.clone(), kickoff, Vec::new()).await;
        }
        ClientMsg::ClearGoal => {
            slot.session.read().await.clear_goal().await;
            let _ = slot.events_tx.send(ServerMsg::GoalCleared);
        }
        ClientMsg::Compact { focus } => {
            let sess = slot.session.read().await.clone();
            let tx = slot.events_tx.clone();
            tokio::spawn(async move {
                let msg = match sess.compact_now(focus.as_deref()).await {
                    Ok(n) => ServerMsg::Compacted {
                        messages_removed: n,
                    },
                    Err(e) => ServerMsg::Error {
                        text: format!("couldn't compact: {e}"),
                    },
                };
                let _ = tx.send(msg);
            });
        }
        ClientMsg::Sync => {
            let ready = build_ready(&slot, state).await;
            let _ = slot.events_tx.send(ready);
        }
        ClientMsg::Attach { session_id } => {
            // Materialize the slot from disk if it isn't loaded yet — a
            // sidebar click on a persisted-but-not-open session should
            // Just Work rather than surfacing "unknown session id".
            let target = match state
                .ensure_slot(&mira_core::SessionId::from(session_id.as_str()))
                .await
            {
                Ok(s) => s,
                Err(e) => {
                    let _ = slot.events_tx.send(ServerMsg::Error {
                        text: format!("attach `{session_id}`: {e}"),
                    });
                    return;
                }
            };
            if target.id == slot.id {
                // No-op attach — still send a fresh Ready in case the
                // client just wants to re-sync state cheaply.
                let _ = slot.events_tx.send(build_ready(&slot, state).await);
                return;
            }
            // Swap the attach state: drop the old AttachGuard (decrements
            // old slot's attached count), install a fresh one on the
            // target slot (increments its count).
            {
                let mut guard = attach.write().await;
                *guard = AttachState {
                    _guard: AttachGuard::new(&target),
                    slot: target.clone(),
                };
            }
            // Hand the forwarder a fresh receiver on the target slot's
            // channel. Fresh subscription = we won't replay past frames;
            // the Ready below is the client's single source of state.
            let _ = rx_swap.send(target.events_tx.subscribe()).await;
            // Publish a Ready onto the TARGET slot's channel so this
            // client sees the new session's state on its next tick.
            let ready = build_ready(&target, state).await;
            let _ = target.events_tx.send(ready);
            // Update `active` so the next HTTP call (settings, memory, …)
            // targets what the user is watching.
            state.set_active(target.id.clone()).await;
            info!(from = %slot.id, to = %target.id, "ws: attached");
        }
        ClientMsg::Detach => {
            // Explicit detach: swap attach to the server's active session
            // (a no-op if that's what we were on). Detaching to *nothing*
            // would leave the WS with no receiver — better UX is to fall
            // back to the active pointer so the user can still send a
            // message from that connection.
            let target = state.active_slot().await;
            if target.id == slot.id {
                return;
            }
            {
                let mut guard = attach.write().await;
                *guard = AttachState {
                    _guard: AttachGuard::new(&target),
                    slot: target.clone(),
                };
            }
            let _ = rx_swap.send(target.events_tx.subscribe()).await;
            let ready = build_ready(&target, state).await;
            let _ = target.events_tx.send(ready);
        }
        ClientMsg::SetBackgroundMode { mode } => {
            {
                let mut guard = slot.background_mode.write().await;
                *guard = mode;
            }
            // Fan out — a tab watching a different session should still
            // see the sidebar's "•" indicator update.
            state
                .broadcast_all(ServerMsg::BackgroundModeChanged {
                    session_id: slot.id.to_string(),
                    mode,
                })
                .await;
        }
    }
}

/// Apply a `SetModel`: resolve the engine instance, route native
/// providers through the pool, adopt default models, and persist.
///
/// This is the whole seamless-switching flow in one place:
/// - `{model}` alone keeps the current instance (legacy clients).
/// - `{instance}` alone adopts that instance's default model.
/// - `{instance, model}` sets both.
/// - A native instance switch rebuilds/activates its provider and
///   kicks a background catalog fetch so the harness learns the real
///   context window; a model the new provider can't serve falls back
///   to the instance's default rather than erroring mid-turn.
async fn apply_model_selection(
    state: AppState,
    slot: Arc<SessionSlot>,
    instance: Option<String>,
    model: Option<String>,
) {
    if instance.is_none() && model.is_none() {
        return;
    }
    let engine_cfg = mira_config::MiraConfig::load_global().unwrap_or_default();
    let (cur_instance, cur_model, _small) = state.selection.snapshot();

    // Resolve the target instance: explicit, else keep the current one,
    // else the registry's default native instance.
    let target_instance = match instance {
        Some(id) => {
            if state.engines.get(&id).is_none() {
                let _ = slot.events_tx.send(ServerMsg::Warning {
                    text: format!("unknown engine `{id}` — see GET /api/engines"),
                });
                return;
            }
            Some(id)
        }
        None => cur_instance.or_else(|| {
            state
                .engines
                .default_native_instance(&engine_cfg)
                .map(|i| i.id.to_string())
        }),
    };
    let target_instance = match target_instance {
        Some(id) => id,
        None => {
            // No engine instance anywhere (empty config): the model-id
            // path still works against whatever delegate is active — and a
            // model picked there is still a pick of the provider.
            crate::session_engine::select_provider(&state, &slot).await;
            if let Some(m) = model {
                slot.session.read().await.set_model(&m).await;
                let _ = slot.events_tx.send(ServerMsg::ModelChanged {
                    model: m,
                    instance: None,
                });
            }
            crate::session_engine::publish(&state, &slot).await;
            return;
        }
    };

    // Native instance switch: swap the pool delegate before touching
    // the session, so an in-flight turn finishing on the old provider
    // and the next turn starting on the new one never interleave.
    let inst = state.engines.get(&target_instance);
    let is_native = inst.map(|i| i.is_native()).unwrap_or(false);
    if let (Some(i), false) = (inst, is_native) {
        // An external engine instance picked from the model list is an
        // agent pick: same path as the Agent section of the picker.
        let Some(cfg) = state.engines.external_driver_config(&target_instance) else {
            return;
        };
        let mut params = crate::acp_session::AcpLaunchParams::new(i.driver.to_string(), cfg);
        params.model = model.filter(|m| !m.trim().is_empty()).or_else(|| i.model.clone());
        crate::session_engine::select_agent(&state, &slot, params).await;
        return;
    }
    if is_native {
        // Picking a provider is also a decision to leave any external
        // agent behind — the provider takes over the session, and the
        // agent's turns are handed to it so the conversation continues.
        crate::session_engine::select_provider(&state, &slot).await;
        let rebuilt = mira_engine::native::build_native_provider(&engine_cfg, &target_instance);
        match rebuilt {
            Ok(p) => {
                state.provider.register(&target_instance, p);
                state.provider.activate(&target_instance);
                crate::models::invalidate();
            }
            Err(mira_engine::EngineState::NotConfigured { reason }) => {
                // Refuse the switch rather than pointing the session at
                // a provider that will 401 mid-turn.
                let _ = slot.events_tx.send(ServerMsg::Warning {
                    text: format!("can't switch to `{target_instance}`: {reason}"),
                });
                return;
            }
            Err(other) => {
                let _ = slot.events_tx.send(ServerMsg::Warning {
                    text: format!("can't switch to `{target_instance}`: {other:?}"),
                });
                return;
            }
        }
    }

    // Model: explicit → the instance's own default → keep the current
    // model. The fallback order is what makes cross-provider switches
    // work without the picker having to know each provider's catalog.
    let target_model = model
        .filter(|m| !m.trim().is_empty())
        .or_else(|| inst.and_then(|i| i.model.clone()))
        .or(cur_model)
        .unwrap_or_else(|| "default".to_string());

    // Persist + publish the selection first — AgentTool and the
    // engines API read the shared cell, and a subagent spawned right
    // after the user clicked should already see the new pick.
    {
        let mut inst_lock = state
            .selection
            .instance
            .write()
            .expect("selection lock poisoned");
        *inst_lock = Some(target_instance.clone());
    }
    {
        let mut model_lock = state
            .selection
            .model
            .write()
            .expect("selection lock poisoned");
        *model_lock = Some(target_model.clone());
    }

    slot.session.read().await.set_model(&target_model).await;
    let mut s = RuntimeState::load().unwrap_or_default();
    s.last_model = Some(target_model.clone());
    s.last_engine = Some(target_instance.clone());
    if let Err(e) = s.save() {
        warn!(%e, "state.yaml: save failed after model change");
    }
    let _ = slot.events_tx.send(ServerMsg::ModelChanged {
        model: target_model.clone(),
        instance: Some(target_instance.clone()),
    });
    crate::session_engine::publish(&state, &slot).await;

    if is_native {
        // Best-effort catalog lookup: if the provider reported a real
        // context length for this model, hand it to the harness so
        // compaction plans against the true window instead of the
        // prefix-match table.
        let provider = state.harness_provider.clone();
        let model_for_window = target_model.clone();
        let session = slot.session.read().await.clone();
        tokio::spawn(async move {
            let Ok(models) = provider.list_models().await else {
                return;
            };
            if let Some(info) = models.iter().find(|m| m.id == model_for_window) {
                session.set_context_window(info.context_length).await;
            } else {
                session.set_context_window(None).await;
            }
        });
    } else {
        // External agent: the agent owns its context window; clear any
        // stale native override.
        slot.session.read().await.set_context_window(None).await;
    }
}

/// Deliver a prompt to the session's external agent, starting it first
/// when the session is configured but nothing is running.
///
/// This is the agent half of engine routing: a session whose config says
/// "Claude Code" sends every prompt here — including the plain `Send`
/// frames a fresh chat produces — so the agent behaves like the provider
/// does: picked once, serving every turn, in as many concurrent sessions
/// as the user opens. The opt-out is `AcpStop` (or picking a native
/// engine in the model picker), which clears the config.
async fn prompt_agent(
    state: &AppState,
    slot: &Arc<SessionSlot>,
    text: String,
    images: Vec<mira_core::ImageData>,
) {
    // A session set up for an agent — picked here, inherited from the
    // previous chat, or restored from disk — starts it on demand. The
    // start is shared with any pick still in flight, so a prompt sent
    // while the agent boots waits for that process instead of spawning
    // a second one.
    let handle = match crate::session_engine::ensure_agent(state, slot).await {
        Ok(h) => h,
        Err(e) => {
            let _ = slot.events_tx.send(ServerMsg::Error { text: e });
            // The client marked the turn busy when it sent; end it.
            crate::acp_host::AcpEventPort::new(slot.events_tx.clone()).turn_ended("start_failed");
            return;
        }
    };
    let Some(agent) = handle.agent().await else {
        let _ = slot.events_tx.send(ServerMsg::Error {
            text: "the external agent has stopped".into(),
        });
        crate::acp_host::AcpEventPort::new(slot.events_tx.clone()).turn_ended("agent_exited");
        return;
    };

    // The prompt belongs in the transcript sidecar too, or replay shows
    // answers without their questions. Recorded here rather than in the
    // pump because prompts never cross the event bus.
    if let Some(path) = state
        .store
        .as_ref()
        .and_then(|store| store.agent_log_path(&slot.id))
    {
        let line = serde_json::json!({
            "t": mira_harness::persist::now_ms(),
            "driver": handle.driver_kind,
            "user": { "text": text, "images": images.len() },
        });
        mira_acp::agent_sessions::append_line_to(&path, &line);
    }
    // Untitled sessions take the first prompt as their title at once, so
    // the sidebar row reads as something; then the agent writes a proper
    // one from it (Claude Code's own title generation) and replaces it.
    {
        let session = slot.session.read().await;
        if session.title().await.is_none() {
            let first: String = text.chars().take(80).collect();
            if !first.trim().is_empty() {
                session.set_title(first).await;
                let session = session.clone();
                let agent = agent.clone();
                let state = state.clone();
                let description = text.clone();
                tokio::spawn(async move {
                    if let Some(title) = agent.generate_title(&description).await {
                        session.set_title(&title).await;
                        state
                            .broadcast_all(ServerMsg::SessionTitleUpdated {
                                session_id: session.id.to_string(),
                                title,
                            })
                            .await;
                    }
                });
            }
        }
    }

    // Coming from a provider, the agent gets what it missed in front of
    // this prompt, so switching engines mid-conversation keeps the thread.
    // The sidecar above records only what the user typed.
    let mut text = match crate::session_engine::take_context_for_agent(slot).await {
        Some(context) => format!("{context}{text}"),
        None => text,
    };

    // Images land in the agent's files dir and are named in the text:
    // the agent can only open what it is allowed to read, and the
    // prompt is what tells it the files exist.
    if !images.is_empty() {
        match handle.agent_files_dir.as_ref() {
            Some(dir) => match stage_agent_images(dir, &images) {
                Ok(names) => {
                    text.push_str("\n\nAttached images (open them to see):\n");
                    for n in &names {
                        text.push_str(&format!("- {n}\n"));
                    }
                }
                Err(e) => {
                    let _ = slot.events_tx.send(ServerMsg::Error {
                        text: format!("could not stage attachments: {e}"),
                    });
                    crate::acp_host::AcpEventPort::new(slot.events_tx.clone())
                        .turn_ended("attachments_failed");
                    return;
                }
            },
            None => {
                let _ = slot.events_tx.send(ServerMsg::Error {
                    text: "this agent was started without an attachments dir, so images were not sent".into(),
                });
                crate::acp_host::AcpEventPort::new(slot.events_tx.clone())
                    .turn_ended("attachments_failed");
                return;
            }
        }
    }

    // The turn blocks until the agent finishes, so it runs on its own
    // task: the WS reader must stay free to carry permission requests
    // and the cancel that a blocked turn would otherwise prevent.
    let events = slot.events_tx.clone();
    tokio::spawn(async move {
        match agent.prompt_text(&text).await {
            // `None` means a native agent, which announces its own
            // turn end when the CLI reports `result`. Announcing it
            // here as well would end the turn twice.
            Ok(Some(stop_reason)) => {
                crate::acp_host::AcpEventPort::new(events).turn_ended(&stop_reason);
            }
            Ok(None) => {}
            Err(e) => {
                let _ = events.send(ServerMsg::Error {
                    text: format!("agent turn failed: {e}"),
                });
                // Nothing else will end this turn: the prompt never landed.
                crate::acp_host::AcpEventPort::new(events).turn_ended("error");
            }
        }
    });
}

/// Widen the session's policy in response to an "Allow for session" /
/// "Allow always" approval. Builds one rule per policy target the call/// would touch (usually one; `apply_patch` returns every source + dest)
/// and appends it to the shared `Policy`'s allow list. On `Always`
/// scope, the same rules also get appended to `~/.mira/mira.yaml` so
/// they survive a restart.
///
/// Never fails the WS handler — a bad target or write error is logged
/// and surfaced as a Warning frame so the user can see something went
/// wrong without losing the one-shot approval that already resolved.
async fn apply_scope_widening(
    state: &AppState,
    slot: &Arc<SessionSlot>,
    call: &mira_core::ToolCall,
    scope: crate::protocol::ApprovalScope,
) {
    let Some(tool) = state.base_registry.get(&call.function.name) else {
        // Interactive/agent tools aren't in base_registry — they run at
        // `Action::Pure` so they wouldn't have triggered Ask anyway. Silent
        // no-op is the right thing.
        return;
    };
    // A compound shell command is approved part by part (see
    // `Policy::evaluate`), so "allow for this session" adds a rule for each
    // part that needed approval as well as the exact chain — the next
    // chain that reuses those commands runs without asking.
    let targets: Vec<String> = tool
        .policy_targets(call)
        .into_iter()
        .filter(|t| !t.is_empty())
        .collect();
    let mut expanded = targets.clone();
    if tool.action() == mira_tools::Action::Bash {
        let policy = state.policy.lock().await;
        for t in &targets {
            expanded.extend(policy.parts_needing_approval(t));
        }
    }
    let rule_strings: Vec<String> = expanded
        .into_iter()
        .map(|t| rule_string_for(tool.action(), &t))
        .filter(|r| !r.is_empty())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();

    if rule_strings.is_empty() {
        return;
    }

    // Session scope: add to the in-memory policy. Failure per rule is
    // logged and the loop continues so a single bad target doesn't
    // silently swallow the rest.
    {
        let mut policy = state.policy.lock().await;
        for r in &rule_strings {
            if let Err(e) = policy.add_allow_rule(r) {
                warn!(rule = %r, %e, "scope widen: parse failed; skipping");
            }
        }
    }

    let persisted = matches!(scope, crate::protocol::ApprovalScope::Always);
    if persisted {
        if let Err(e) = persist_allow_rules(&rule_strings).await {
            warn!(%e, "scope widen: persist to global config failed");
            let _ = slot.events_tx.send(ServerMsg::Warning {
                text: format!("saved for this session; couldn't persist to config: {e}"),
            });
        }
    }

    let _ = slot.events_tx.send(ServerMsg::Warning {
        text: format!(
            "policy: added {} allow rule{} ({}): {}",
            rule_strings.len(),
            if rule_strings.len() == 1 { "" } else { "s" },
            if persisted {
                "persistent"
            } else {
                "session-only"
            },
            rule_strings.join(", "),
        ),
    });
}

/// Render a `(Action, target)` pair as a policy DSL rule that
/// [`mira_policy::Policy::add_allow_rule`] can parse. Bash calls use
/// the raw command string as an exact-match; file actions wrap the
/// path in the corresponding `Read/Edit/Write(path)` form (glob-
/// exact — the path is literal, not a wildcard).
fn rule_string_for(action: mira_tools::Action, target: &str) -> String {
    match action {
        mira_tools::Action::Bash => format!("Bash({target})"),
        mira_tools::Action::Read => format!("Read({target})"),
        mira_tools::Action::Edit => format!("Edit({target})"),
        mira_tools::Action::Write => format!("Write({target})"),
        mira_tools::Action::Computer | mira_tools::Action::Browser | mira_tools::Action::Mcp => {
            mira_policy::session_rule_for(action, target).unwrap_or_default()
        }
        // Pure never gates; if we somehow got here just synthesize
        // something the parser will reject so the caller logs + skips.
        mira_tools::Action::Pure => String::new(),
    }
}

/// Append rules to `~/.mira/mira.yaml`'s `permissions.allow` and save.
/// Loads the on-disk file fresh (not the merged runtime view) so we
/// only ever write user-owned config, never a per-repo overlay.
async fn persist_allow_rules(rules: &[String]) -> anyhow::Result<()> {
    let rules = rules.to_vec();
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let mut cfg = mira_config::MiraConfig::load_global()?;
        for r in rules {
            if !cfg.permissions.allow.contains(&r) {
                cfg.permissions.allow.push(r);
            }
        }
        cfg.save_global()?;
        Ok(())
    })
    .await
    .map_err(|e| anyhow::anyhow!("blocking task join: {e}"))?
}

/// Spawn a turn task on `slot`, storing its JoinHandle on the slot so
/// Interrupt and delete_session can tear it down cleanly. Aborts any
/// existing turn handle before starting the new one.
async fn spawn_turn(
    state: AppState,
    slot: Arc<SessionSlot>,
    text: String,
    images: Vec<mira_core::ImageData>,
) {
    // Tools would run against a half-moved tree mid-switch.
    if slot.environments.is_switching() {
        let _ = slot.events_tx.send(ServerMsg::Warning {
            text: "switching environments — send again when it's done".into(),
        });
        let _ = slot.events_tx.send(ServerMsg::Done);
        return;
    }
    // Refresh the skill registry from disk before the turn starts.
    crate::skills::reload_registry(&state).await;

    // `/name args` for a custom command or MCP prompt expands here, so a
    // typed command and a palette pick behave the same.
    let text = match expand_slash(&state, &text).await {
        Ok(t) => t,
        Err(message) => {
            let _ = slot.events_tx.send(ServerMsg::Warning { text: message });
            let _ = slot.events_tx.send(ServerMsg::Done);
            return;
        }
    };

    // Publish a "running" marker — fanned out so a client watching a
    // different session still sees the sidebar spinner light up.
    state
        .broadcast_all(ServerMsg::SessionBackgroundRunning {
            session_id: slot.id.to_string(),
        })
        .await;

    let slot_for_task = slot.clone();
    let state_for_task = state.clone();
    let handle = tokio::spawn(async move {
        let sess = slot_for_task.session.read().await.clone();
        let mut stream = sess.send_with_images(text, images).await;
        while let Some(evt) = stream.next().await {
            let frame = ServerMsg::from_harness(evt);
            let _ = slot_for_task.events_tx.send(frame);
        }
        // Turn wrapped up. If this session still needs a nickname, kick off
        // a short generation call on the same provider + model. Runs on
        // its own task so a slow / failing title call doesn't block the
        // next user turn.
        let cfg = sess.config().await;
        let model = cfg.background_model(None);
        let fallback = (model != cfg.model).then(|| cfg.model.clone());
        title::spawn_if_needed(
            sess,
            state_for_task.harness_provider.clone(),
            model,
            fallback,
            state_for_task.clone(),
        );
        // Idle marker — same fan-out reasoning as the running marker
        // above. A background session's sidebar spinner clears even for
        // clients focused elsewhere.
        state_for_task
            .broadcast_all(ServerMsg::SessionBackgroundIdle {
                session_id: slot_for_task.id.to_string(),
            })
            .await;
    });

    // Replace any existing handle; abort the prior one to prevent
    // "two turns racing on one session" (which the harness would refuse
    // anyway, but the JoinHandle would leak).
    let mut guard = slot.turn.lock().await;
    if let Some(prev) = guard.take() {
        prev.abort();
    }
    *guard = Some(handle);
}

async fn send_json<S>(sink: &mut S, msg: &ServerMsg) -> Result<(), ()>
where
    S: SinkExt<Message> + Unpin,
{
    let text = match serde_json::to_string(msg) {
        Ok(t) => t,
        Err(e) => {
            warn!(%e, "ws: serialize failed");
            return Err(());
        }
    };
    sink.send(Message::Text(text)).await.map_err(|_| ())
}

/// Expand a leading `/command` if it names a custom command or an MCP
/// prompt; anything else passes through unchanged.
async fn expand_slash(state: &AppState, text: &str) -> Result<String, String> {
    let Some(rest) = text.trim_start().strip_prefix('/') else {
        return Ok(text.to_owned());
    };
    let (name, args) = rest
        .split_once(char::is_whitespace)
        .map(|(n, a)| (n, a.trim()))
        .unwrap_or((rest.trim(), ""));
    match state.extensions.expand(name, args).await {
        Some(r) => r,
        None => Ok(text.to_owned()),
    }
}

/// Write prompt images into the agent's files dir, returning their names.
///
/// Filenames carry no user content (`img-3.png`, not the pasted name), so a
/// hostile filename cannot escape the dir or overwrite session files. Only
/// png/jpeg/webp/gif are accepted — anything else is rejected rather than
/// sniffed, because the agent will open whatever lands here.
fn stage_agent_images(
    dir: &std::path::Path,
    images: &[mira_core::ImageData],
) -> Result<Vec<String>, String> {
    use base64::Engine;
    let mut names = Vec::new();
    for (i, img) in images.iter().enumerate() {
        let ext = match img.media_type.as_str() {
            "image/png" => "png",
            "image/jpeg" => "jpeg",
            "image/webp" => "webp",
            "image/gif" => "gif",
            other => return Err(format!("unsupported image type: {other}")),
        };
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(img.data.as_bytes())
            .map_err(|e| format!("image {i} is not valid base64: {e}"))?;
        let name = format!("img-{i}.{ext}");
        std::fs::write(dir.join(&name), &bytes)
            .map_err(|e| format!("could not write {name}: {e}"))?;
        names.push(name);
    }
    Ok(names)
}

#[cfg(test)]
mod attachment_tests {
    use super::*;

    fn png_data() -> String {
        // 1x1 transparent PNG.
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==".to_string()
    }

    #[test]
    fn images_stage_with_safe_names() {
        let dir = std::env::temp_dir().join(format!("mira-agent-files-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let imgs = vec![
            mira_core::ImageData { media_type: "image/png".into(), data: png_data() },
            mira_core::ImageData { media_type: "image/jpeg".into(), data: png_data() },
        ];
        let names = stage_agent_images(&dir, &imgs).unwrap();
        assert_eq!(names, vec!["img-0.png", "img-1.jpeg"]);
        assert!(dir.join("img-0.png").is_file());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn hostile_types_are_rejected_not_sniffed() {
        let dir = std::env::temp_dir();
        let imgs = vec![mira_core::ImageData {
            media_type: "application/x-sh".into(),
            data: png_data(),
        }];
        assert!(stage_agent_images(&dir, &imgs).is_err());
        let bad = vec![mira_core::ImageData {
            media_type: "image/png".into(),
            data: "!!!not-base64!!!".into(),
        }];
        assert!(stage_agent_images(&dir, &bad).is_err());
    }
}
