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
use mira_ai::ChatProvider;
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
    let mut global_activity = state.session_activity.subscribe();
    let activity_hub = state.session_activity.clone();
    let initial_events = initial_slot.events_tx.subscribe();
    let ready = build_ready(&initial_slot, &state).await;
    if send_json(&mut sink, &ready).await.is_err() {
        return;
    }

    // Channel the reader uses to hand new broadcast receivers to the
    // forwarder when the client attaches to a different slot.
    let (rx_swap_tx, mut rx_swap_rx) = mpsc::channel::<broadcast::Receiver<ServerMsg>>(4);

    // Forwarder task: broadcast → socket, with hot-swappable subscription.
    let mut forwarder = tokio::spawn(async move {
        let mut rx = initial_events;
        loop {
            tokio::select! {
                activity = global_activity.recv() => {
                    let frame = match activity {
                        Ok(frame) => frame,
                        Err(broadcast::error::RecvError::Lagged(_)) => ServerMsg::SessionActivitySnapshot {snapshot:activity_hub.snapshot()},
                        Err(broadcast::error::RecvError::Closed) => break,
                    };
                    if send_json(&mut sink,&frame).await.is_err() {break;}
                }
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
                            if send_json(&mut sink,&ServerMsg::SessionActivitySnapshot {snapshot:activity_hub.snapshot()}).await.is_err() {break;}
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
    if let Some(owner) = state.slot(&slot.id).await {
        crate::runtime_requests::wake_outbox(state, &owner);
        crate::message_queue::wake(state, &owner);
    }
    let sess = slot.session.read().await.clone();
    let cfg = sess.config().await;
    let mode = if slot.acp_launch.lock().await.is_some() {
        cfg.agent_approval_mode
    } else {
        state.policy.lock().await.mode()
    };
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
    let (instance, _model, _small) = slot.selection.snapshot();
    // The session's *configured* agent — inherited from the previous
    // chat or retained while running — when none is live. The client
    // shows the setup and the first prompt boots it.
    let agent_configured = if slot.acp_agent.read().await.is_none() {
        slot.acp_launch
            .lock()
            .await
            .as_ref()
            .map(|p| p.driver_kind.clone())
    } else {
        None
    };
    let items = crate::transcript_history::interleave(&history, &agent_transcript);
    let transcript_page = crate::transcript_history::page(&sess.id.to_string(), &items, None, 20)
        .expect("initial history page has no cursor");
    let agent_transcript = crate::transcript_history::latest_state(&agent_transcript);
    let turn_diffs = crate::checkpoints::turn_diffs(&cwd, &sess.id.to_string());
    ServerMsg::Ready {
        queued_inputs: slot.message_queue.snapshot().await,
        session_activity: state.session_activity.snapshot(),
        turn_diffs,
        transcript_page,
        session_id: sess.id.to_string(),
        model: cfg.model,
        mode,
        cwd: cwd.display().to_string(),
        history: Vec::new(),
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
        running: slot.is_running().await,
        runtime_requests: slot.runtime_requests.snapshot().await,
        runtime_work: slot
            .engine
            .activity
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .work_snapshot(),
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
        ClientMsg::SyncActivity => {
            let _ = slot.events_tx.send(ServerMsg::SessionActivitySnapshot {
                snapshot: state.session_activity.snapshot(),
            });
        }
        ClientMsg::QueueInput {
            session_id,
            id,
            text,
            images,
        } => {
            let Ok(slot) = state
                .ensure_slot(&mira_core::SessionId::from(session_id.as_str()))
                .await
            else {
                return;
            };
            let (engine, _, _) = slot.selection.snapshot();
            let item = crate::message_queue::QueuedInput {
                id,
                text,
                images,
                engine,
                dispatching: false,
                delivered: false,
                fingerprint: String::new(),
                error: None,
                recovery: None,
            };
            if let Err(text) = slot.message_queue.enqueue(item).await {
                let _ = slot.events_tx.send(ServerMsg::Error { text });
            }
            crate::message_queue::publish(&slot).await;
            crate::message_queue::wake(state, &slot);
        }
        ClientMsg::RemoveQueuedInput { session_id, id } => {
            let Ok(slot) = state
                .ensure_slot(&mira_core::SessionId::from(session_id.as_str()))
                .await
            else {
                return;
            };
            if let Err(text) = slot.message_queue.remove(&id).await {
                let _ = slot.events_tx.send(ServerMsg::Error { text });
            }
            crate::message_queue::publish(&slot).await;
            crate::message_queue::wake(state, &slot);
        }
        ClientMsg::EditQueuedInput {
            session_id,
            request_id,
            id,
            fingerprint,
            text,
            images,
        } => {
            let result = if session_id != slot.id.to_string() {
                Err("Queue edit belongs to another chat".into())
            } else {
                slot.message_queue
                    .edit(&id, &fingerprint, text, images)
                    .await
            };
            crate::message_queue::publish(&slot).await;
            let _ = slot.events_tx.send(ServerMsg::QueueMutationResult {
                session_id,
                request_id,
                error: result.err(),
            });
            crate::message_queue::wake(state, &slot);
        }
        ClientMsg::ReorderQueuedInput {
            session_id,
            request_id,
            id,
            before_id,
        } => {
            let result = if session_id != slot.id.to_string() {
                Err("Queue reorder belongs to another chat".into())
            } else {
                slot.message_queue.reorder(&id, before_id.as_deref()).await
            };
            crate::message_queue::publish(&slot).await;
            let _ = slot.events_tx.send(ServerMsg::QueueMutationResult {
                session_id,
                request_id,
                error: result.err(),
            });
            crate::message_queue::wake(state, &slot);
        }

        ClientMsg::UpdateLimitRecovery {
            session_id,
            request_id,
            id,
            action,
        } => {
            let result = if session_id != slot.id.to_string() {
                Err("Recovery belongs to another chat".into())
            } else if slot.is_foreground_running().await {
                Err("Wait for the current turn to finish".into())
            } else {
                slot.message_queue.update_recovery(&id, &action).await
            };
            crate::message_queue::publish(&slot).await;
            let _ = slot.events_tx.send(ServerMsg::QueueMutationResult {
                session_id,
                request_id,
                error: result.err(),
            });
            crate::message_queue::wake(state, &slot);
        }
        ClientMsg::History {
            session_id,
            cursor,
            request_id,
        } => {
            let sess = slot.session.read().await.clone();
            let result = if session_id != sess.id.to_string() {
                Err("history request belongs to another chat".into())
            } else {
                let messages = sess.transcript().await;
                let (lines, _) =
                    crate::sessions::read_agent_state(state.store.as_ref(), &sess).await;
                let items = crate::transcript_history::interleave(&messages, &lines);
                crate::transcript_history::page(&session_id, &items, Some(&cursor), 20)
            };
            let (page, error) = match result {
                Ok(page) => (Some(page), None),
                Err(error) => (None, Some(error)),
            };
            let _ = slot.events_tx.send(ServerMsg::HistoryPage {
                session_id,
                request_id,
                page,
                error,
            });
        }

        ClientMsg::Steer {
            request_id,
            text,
            images,
        } => {
            let state = state.clone();
            let slot = slot.clone();
            // Keep the reader free for approvals while provider tools finish.
            tokio::spawn(async move {
                let stored = slot
                    .message_queue
                    .snapshot()
                    .await
                    .into_iter()
                    .find(|i| i.id == request_id);
                let queued = stored.is_some();
                if let Some(stored) = stored {
                    if stored.text != text
                        || serde_json::to_value(stored.images).ok()
                            != serde_json::to_value(&images).ok()
                    {
                        let _ = slot.events_tx.send(ServerMsg::SteerResult {
                            session_id: slot.id.to_string(),
                            request_id,
                            message: None,
                            error: Some("Queued input changed; refresh before steering".into()),
                        });
                        return;
                    }
                }
                if !queued {
                    let _ = slot.events_tx.send(ServerMsg::SteerResult {session_id:slot.id.to_string(),request_id,message:None,error:Some("This queued input was already delivered or removed. Refresh the chat before retrying.".into())});
                    return;
                }
                if !slot.is_foreground_running().await {
                    crate::message_queue::wake(&state, &slot);
                    let _ = slot.events_tx.send(ServerMsg::SteerResult {
                        session_id: slot.id.to_string(),
                        request_id,
                        message: None,
                        error: Some(
                            "The turn has finished. This input will run from the queue.".into(),
                        ),
                    });
                    return;
                }
                if queued {
                    let (engine, _, _) = slot.selection.snapshot();
                    if let Err(error) = slot
                        .message_queue
                        .claim(&request_id, engine.as_deref())
                        .await
                    {
                        let _ = slot.events_tx.send(ServerMsg::SteerResult {
                            session_id: slot.id.to_string(),
                            request_id,
                            message: None,
                            error: Some(error),
                        });
                        return;
                    }
                }
                let mut message =
                    mira_core::Message::user(text.clone()).with_images(images.clone());
                message.input_id = Some(request_id.clone());
                message.input_intent = Some("steer".into());
                let result: Result<(), String> = if slot.acp_launch.lock().await.is_some() {
                    let handle = slot.acp_agent.read().await.clone();
                    let agent = match &handle {
                        Some(handle) => handle.agent().await,
                        None => None,
                    };
                    match agent {
                        Some(mira_acp::native::AgentHandle::AppServer(agent)) => {
                            match agent.steer_with_images(&text, &images).await {
                                Ok(()) => {
                                    Ok(())
                                }
                                Err(error) => Err(error.to_string()),
                            }
                        }
                        Some(mira_acp::native::AgentHandle::Native(agent)) => {
                            agent.steer(&request_id, &text, &images).await.map_err(|e| e.to_string())
                        }
                        Some(mira_acp::native::AgentHandle::Acp(agent)) => {
                            match handle.as_ref().and_then(|h| h.opencode_control.as_ref()) {
                                Some(control) => match agent.session_id() {
                                    Some(sid) => control.steer(&sid, &request_id, &text, &images).await,
                                    None => Err("OpenCode has no active session".into()),
                                },
                                None => Err("This adapter does not support active steering".into()),
                            }
                        }
                        _ => Err("This agent cannot accept input during an active turn. Your message is still queued.".into()),
                    }
                } else {
                    let session = slot.session.read().await.clone();
                    if session
                        .steer_with_id(text.clone(), images.clone(), Some(request_id.clone()))
                        .await
                    {
                        Ok(())
                    } else {
                        Err("The active turn ended before it could accept this message. Your message is still queued.".into())
                    }
                };
                if result.is_ok() && slot.acp_launch.lock().await.is_some() {
                    if let Some(path) = state
                        .store
                        .as_ref()
                        .and_then(|store| store.agent_log_path(&slot.id))
                    {
                        mira_acp::agent_sessions::append_line_to(
                            &path,
                            &serde_json::json!({
                                "t": mira_harness::persist::now_ms(),
                                "user": { "text": message.content, "images": images.len(), "attached_images": images, "input_intent": "steer", "input_id": request_id, "request_id": request_id }
                            }),
                        );
                    }
                }
                if queued {
                    let _ = slot
                        .message_queue
                        .settle(&request_id, result.as_ref().err().cloned())
                        .await;
                    crate::message_queue::publish(&slot).await;
                }
                let _ = slot.events_tx.send(ServerMsg::SteerResult {
                    session_id: slot.id.to_string(),
                    request_id,
                    message: result.is_ok().then_some(message),
                    error: result.err(),
                });
            });
        }

        ClientMsg::Send { text, images } => {
            if let Err(text) = send_input(state, &slot, text, images, None).await {
                let _ = slot.events_tx.send(ServerMsg::Warning { text });
            }
        }
        ClientMsg::Resend {
            original,
            occurrence,
            text,
        } => {
            if let Err(text) = slot.message_queue.supersede_recovery(None).await {
                let _ = slot.events_tx.send(ServerMsg::Warning { text });
                return;
            }
            crate::message_queue::publish(&slot).await;
            // Same engine routing as `Send`: on an agent-configured
            // session the resend is a fresh prompt to that agent — the
            // native rewind doesn't apply to a transcript the agent
            // owns. (The original images were staged into the agent's
            // files dir and can't be re-staged, so text only.)
            let configured_for_agent = slot.acp_launch.lock().await.is_some();
            if configured_for_agent {
                before_prompt(&slot, &text, Some((&original, occurrence))).await;
                prompt_agent(state, &slot, text, Vec::new(), None).await;
                return;
            }
            let sess = slot.session.read().await.clone();
            if let Some(images) = sess.rewind_to_user(&original, occurrence).await {
                // Only once the rewind took: a refused edit changes nothing.
                before_prompt(&slot, &text, Some((&original, occurrence))).await;
                // Edits keep the original message's images.
                spawn_turn(state.clone(), slot, text, images, None).await;
            } else {
                let _ = slot.events_tx.send(ServerMsg::Warning {
                    text: "can't edit that message — a turn is running, or it was \
                           summarized away by compaction"
                        .into(),
                });
                let _ = slot.events_tx.send(ServerMsg::Done);
            }
        }
        ClientMsg::ApprovalRules { call_id } => {
            let call = slot
                .pending
                .lock()
                .await
                .get(&call_id)
                .map(|(_, call)| call.clone());
            let (rules, error) = match call {
                Some(call) if state.base_registry.get(&call.function.name).is_some() => {
                    (approval_rule_strings(state, &call).await, None)
                }
                _ => (
                    Vec::new(),
                    Some(
                        "This agent manages its own permission rules; Mira cannot edit them."
                            .into(),
                    ),
                ),
            };
            let _ = slot.events_tx.send(ServerMsg::ApprovalRules {
                call_id,
                rules,
                error,
            });
        }
        ClientMsg::Approve {
            call_id,
            allow,
            scope,
            rules,
        } => {
            if let Some(ref rules) = rules {
                let native = slot
                    .pending
                    .lock()
                    .await
                    .get(&call_id)
                    .is_some_and(|(_, call)| {
                        state.base_registry.get(&call.function.name).is_some()
                    });
                if !native
                    || !allow
                    || matches!(scope, crate::protocol::ApprovalScope::Once)
                    || rules.is_empty()
                    || rules
                        .iter()
                        .any(|rule| rule.parse::<mira_policy::Rule>().is_err())
                {
                    let _ = slot.events_tx.send(ServerMsg::ApprovalRules {
                        call_id,
                        rules: rules.clone(),
                        error: Some("Enter valid policy rules before saving.".into()),
                    });
                    return;
                }
            }
            // Recorded before the approval resolves: an agent's gate wakes
            // on resolve and reads the scope to decide what rules to adopt.
            if allow {
                if let Ok(mut m) = slot.engine.approval_scopes.lock() {
                    m.insert(call_id.clone(), scope);
                }
            }
            match approver::resolve(&slot.pending, &call_id, allow).await {
                Some(call) => {
                    let _ = slot.events_tx.send(ServerMsg::ApprovalResolved {
                        call_id: call_id.clone(),
                        allow,
                    });
                    if allow && !matches!(scope, crate::protocol::ApprovalScope::Once) {
                        apply_scope_widening(state, &slot, &call, scope, rules).await;
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
            if let mira_tools::prompt::PromptResponse::AskUser(ref answers) = response {
                let _dispatch = if prompt_id.starts_with("async-") {
                    Some(slot.engine.dispatch_lock.lock().await)
                } else {
                    None
                };
                match slot
                    .runtime_requests
                    .resolve(&prompt_id, answers.clone())
                    .await
                {
                    Ok(Some(request)) => {
                        let _ = slot
                            .events_tx
                            .send(ServerMsg::RuntimeRequestUpdated { request });
                        crate::runtime_requests::wake_outbox(state, &slot);
                        return;
                    }
                    Ok(None) => {}
                    Err(e) => {
                        let _ = slot.events_tx.send(ServerMsg::Error { text: e });
                        return;
                    }
                }
            }
            if crate::interactive::resolve(&slot.prompt_pending, &prompt_id, response.clone()).await
            {
                // Every window on this chat closes its copy of the card with
                // the same answer — not only the one that answered.
                // A secret's value goes only to the agent that asked.
                let response = match response {
                    crate::interactive::PromptResponse::Secret(s) => {
                        crate::interactive::PromptResponse::Secret(
                            mira_tools::prompt::SecretResponse {
                                value: None,
                                cancelled: s.cancelled,
                            },
                        )
                    }
                    other => other,
                };
                let _ = slot.events_tx.send(ServerMsg::PromptResolved {
                    prompt_id,
                    response,
                });
            } else {
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
            // legacy bare-driver-with-defaults path.
            //
            // The settings themselves always come from the engine registry
            // (mira.yaml), never the client (#79). A pick that names only a
            // driver uses the instance named after it, and a restart of the
            // recorded agent re-reads its instance, so a key changed in
            // Settings applies to the next start without a new chat.
            let recorded = slot.acp_launch.lock().await.clone();
            let same_driver_as_recorded = recorded
                .as_ref()
                .is_some_and(|r| driver.as_deref().is_none_or(|d| d == r.driver_kind));
            let instance = instance
                .or_else(|| {
                    if same_driver_as_recorded {
                        recorded.as_ref().map(|r| r.instance.clone())
                    } else {
                        driver.clone()
                    }
                })
                .filter(|id| state.engines.current().external_driver_config(id).is_some());
            let instance_cfg = instance.as_deref().and_then(|id| {
                state.engines.current().external_driver_config(id).map(|c| {
                    let kind = state
                        .engines
                        .current()
                        .get(id)
                        .map(|i| i.driver.to_string())
                        .unwrap_or_else(|| id.to_string());
                    crate::acp_session::InstanceStart {
                        instance: id.to_string(),
                        kind,
                        cfg: c,
                    }
                })
            });
            let resolved = match crate::acp_session::resolve_start_params(
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
            let base_kind = resolved.kind;
            let mut cfg = resolved.cfg;
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
            let mut params = crate::acp_session::AcpLaunchParams::for_instance(
                resolved.instance.clone(),
                base_kind.clone(),
                cfg,
            );
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
                mira_acp::agent_sessions::record(&slot.id.to_string(), &params.instance, &sid);
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
                    if p.driver_kind == "codex" {
                        if let Some(mode) = crate::acp_session::codex_approval_mode(&mode_id) {
                            crate::acp_session::set_agent_approval_mode(&slot, mode).await;
                        }
                    }
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
            let Some(agent) = handle.agent().await else {
                return;
            };

            // A mode that grants more than Mira's own approval pipeline
            // would is a standing change to the agent's authority, so it
            // needs the user to have been told what it does. The client can
            // set the flag, which is why the gate is "was this explained and
            // agreed" rather than "is this a privileged mode" — but it
            // cannot grant the authority without a round trip that showed
            // the consequence.
            let privileged = handle.privileged_mode_reason(&mode_id).is_some();
            if privileged && !acknowledge_privileged {
                let _ = slot
                    .events_tx
                    .send(ServerMsg::AcpPrivilegedModeConfirmation {
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

            if handle.driver_kind == "codex" {
                if let Some(mode) = crate::acp_session::codex_approval_mode(&mode_id) {
                    crate::acp_session::set_agent_approval_mode(&slot, mode).await;
                }
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
            let Some(agent) = handle.agent().await else {
                return;
            };
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
                                driver: Some(handle.driver_kind.clone()),
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
            if let Err(text) = send_input(state, &slot, text, images, None).await {
                let _ = slot.events_tx.send(ServerMsg::Warning { text });
            }
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

        ClientMsg::AcpCompact { focus } => {
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
                        let command = match focus.as_deref().map(str::trim) {
                            Some(f) if !f.is_empty() => format!("/compact {f}"),
                            _ => "/compact".to_string(),
                        };
                        match n.prompt(&command).await {
                            Ok(()) => {}
                            Err(e) => {
                                let _ = events.send(ServerMsg::Error {
                                    text: format!("compaction failed: {e}"),
                                });
                            }
                        }
                    }
                    mira_acp::native::AgentHandle::AppServer(a) => match a.compact().await {
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
                    },
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
            let engines = state.engines.current();
            tokio::spawn(async move {
                let mut tasks = Vec::new();
                let mut covered = std::collections::BTreeSet::new();
                for inst in engines.instances().filter(|i| !i.is_native()) {
                    let Some(d) = mira_acp::drivers::by_kind(inst.driver.as_str()) else {
                        continue;
                    };
                    covered.insert(inst.driver.as_str().to_string());
                    let cfg = mira_engine::driver_config_for(inst);
                    tasks.push(tokio::spawn(async move {
                        mira_acp::probe(d.as_ref(), &cfg, Default::default()).await
                    }));
                }
                for d in mira_acp::drivers::all() {
                    if !covered.contains(d.kind()) {
                        tasks.push(tokio::spawn(async move {
                            mira_acp::probe(d.as_ref(), &Default::default(), Default::default())
                                .await
                        }));
                    }
                }
                let mut agents = Vec::with_capacity(tasks.len());
                for task in tasks {
                    if let Ok(agent) = task.await {
                        agents.push(agent);
                    }
                }
                let _ = events.send(ServerMsg::AcpAgentStatus { agents });
            });
        }

        ClientMsg::SetModelOption { id, value } => {
            apply_model_option(&slot, &id, &value).await;
        }
        ClientMsg::SetModel {
            model,
            instance,
            options,
        } => {
            apply_model_selection(state.clone(), slot.clone(), instance, model).await;
            for (id, value) in &options {
                apply_model_option(&slot, id, value).await;
            }
        }
        ClientMsg::SetMode { mode } => {
            if slot.acp_launch.lock().await.is_some() {
                let mut codex_mode = None;
                if let Some(launch) = slot.acp_launch.lock().await.as_mut() {
                    if launch.driver_kind == "codex" {
                        let mode_id = match mode {
                            mira_policy::Mode::Plan => "plan",
                            mira_policy::Mode::Auto => "auto-accept-edits",
                            mira_policy::Mode::Edit => "auto",
                            mira_policy::Mode::Yolo => "full-access",
                            mira_policy::Mode::Manual => "approval-required",
                        };
                        launch.mode_id = Some(mode_id.into());
                        codex_mode = Some(mode_id);
                    }
                }
                // Codex takes its mode per turn, so the running agent follows
                // from its next turn without a restart.
                if let Some(mode_id) = codex_mode {
                    let handle = slot.acp_agent.read().await.clone();
                    if let Some(agent) = match handle {
                        Some(h) => h.agent().await,
                        None => None,
                    } {
                        let _ = agent.set_mode(mode_id).await;
                    }
                }
                crate::acp_session::set_agent_approval_mode(&slot, mode).await;
                let _ = slot.events_tx.send(ServerMsg::ModeChanged { mode });
                return;
            }
            state.policy.lock().await.set_mode(mode);
            let repo_root = state.sandbox.profile().repo_root.clone();
            let profile = mira_harness::profile_for_mode(mode, &repo_root);
            // Harness sessions share this policy. External agents retain
            // their own chat approval setting and sandbox profile.
            for slot_arc in state.list_slots().await {
                if slot_arc.acp_launch.lock().await.is_some() {
                    continue;
                }
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
                // Stop must end the turn even when the agent doesn't say so
                // itself (or says it late): otherwise the composer spins until
                // the chat is reopened. Listen before cancelling so a prompt
                // turn-end isn't missed.
                let mut rx = slot.events_tx.subscribe();
                if let Some(h) = &agent {
                    if h.driver_kind == "grok" {
                        crate::acp_session::stop_agent(&slot).await;
                        crate::acp_host::AcpEventPort::for_slot(&slot).turn_ended("cancelled");
                    } else {
                        h.cancel_current_turn().await;
                    }
                }
                {
                    let turn_port = crate::acp_host::AcpEventPort::for_slot(&slot);
                    tokio::spawn(async move {
                        let ended =
                            tokio::time::timeout(std::time::Duration::from_secs(3), async {
                                loop {
                                    match rx.recv().await {
                                        Ok(ServerMsg::AcpTurnEnd { .. }) => return true,
                                        Err(broadcast::error::RecvError::Closed) => return false,
                                        _ => {}
                                    }
                                }
                            })
                            .await
                            .unwrap_or(false);
                        if !ended {
                            turn_port.turn_ended("cancelled");
                        }
                    });
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
            slot.engine
                .provider_in_turn
                .store(false, std::sync::atomic::Ordering::SeqCst);
            slot.publish_activity();
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
            spawn_turn(state.clone(), slot.clone(), kickoff, Vec::new(), None).await;
        }
        ClientMsg::ClearGoal => {
            slot.session.read().await.clear_goal().await;
            let _ = slot.events_tx.send(ServerMsg::GoalCleared);
        }
        ClientMsg::Compact { focus } => {
            let sess = slot.session.read().await.clone();
            let tx = slot.events_tx.clone();
            tokio::spawn(async move {
                let before = mira_harness::history::estimated_tokens(&sess.history().await);
                let _ = tx.send(ServerMsg::Compacting {
                    trigger: "manual".into(),
                    tokens_before: Some(before),
                });
                let msg = match sess.compact_now(focus.as_deref()).await {
                    Ok(n) => ServerMsg::Compacted {
                        messages_removed: n,
                        tokens_before: Some(before),
                        tokens_after: Some(mira_harness::history::estimated_tokens(
                            &sess.history().await,
                        )),
                    },
                    Err(e) => ServerMsg::CompactionFailed { error: e },
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

/// Set one model option on the session. `off` / `auto` mean "omit the field
/// entirely", which is what non-reasoning models and the default service
/// tier need. Unknown ids are dropped: accepting arbitrary ids would let a
/// client write any config field by name.
async fn apply_model_option(slot: &SessionSlot, id: &str, value: &str) {
    let normalize = |v: &str| -> Option<String> {
        match v {
            "" | "off" | "auto" => None,
            other => Some(other.to_string()),
        }
    };
    let session = slot.session.read().await;
    match id {
        "reasoning_effort" => session.set_reasoning_effort(normalize(value)).await,
        "service_tier" => session.set_service_tier(normalize(value)).await,
        other => debug!("ws: ignoring unknown model option id {other:?} (value {value:?})"),
    }
}

/// The warning for a model switch that can't carry earlier reasoning over,
/// or `None` when nothing is lost. Anthropic (and Claude on Bedrock) replay
/// signed and redacted thinking; unsigned reasoning (OpenAI, DeepSeek,
/// Gemini) is display-only everywhere, and OpenAI-style providers replay
/// none at all.
fn reasoning_loss_warning(
    history: &[mira_core::Message],
    replays_signed: bool,
    target: &str,
) -> Option<String> {
    let lost = history
        .iter()
        .filter(|m| m.role == mira_core::Role::Assistant)
        .filter(|m| {
            m.reasoning
                .iter()
                .any(|b| !(replays_signed && (b.signature.is_some() || b.redacted.is_some())))
        })
        .count();
    (lost > 0).then(|| {
        format!(
            "Switched to {target}: the reasoning from {lost} earlier {} won't carry over, only the text and tool calls. \
             For a clean boundary, run /compact before continuing.",
            if lost == 1 { "reply" } else { "replies" }
        )
    })
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
    let (cur_instance, cur_model, _small) = slot.selection.snapshot();

    // Resolve the target instance: explicit, else keep the current one,
    // else the registry's default native instance.
    let target_instance = match instance {
        Some(id) => {
            if state.engines.current().get(&id).is_none() {
                let _ = slot.events_tx.send(ServerMsg::Warning {
                    text: format!("unknown engine `{id}` — see GET /api/engines"),
                });
                return;
            }
            Some(id)
        }
        None => cur_instance.clone().or_else(|| {
            state
                .engines
                .current()
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
    let engines = state.engines.current();
    let inst = engines.get(&target_instance);
    let is_native = inst.map(|i| i.is_native()).unwrap_or(false);
    if let (Some(i), false) = (inst, is_native) {
        // An external engine instance picked from the model list is an
        // agent pick: same path as the Agent section of the picker.
        let Some(cfg) = state
            .engines
            .current()
            .external_driver_config(&target_instance)
        else {
            return;
        };
        let mut params = crate::acp_session::AcpLaunchParams::for_instance(
            target_instance.clone(),
            i.driver.to_string(),
            cfg,
        );
        params.model = model
            .filter(|m| !m.trim().is_empty())
            .or_else(|| i.model.clone());
        crate::session_engine::select_agent(&state, &slot, params).await;
        return;
    }
    if is_native {
        if slot.is_running().await
            && slot.acp_launch.lock().await.is_none()
            && cur_instance.as_deref() != Some(&target_instance)
        {
            let _ = slot.events_tx.send(ServerMsg::Warning {
                text: "wait for this turn to finish before switching its provider".into(),
            });
            return;
        }
        // Picking a provider is also a decision to leave any external
        // agent behind — the provider takes over the session, and the
        // agent's turns are handed to it so the conversation continues.
        crate::session_engine::select_provider(&state, &slot).await;
        let rebuilt = mira_engine::native::build_native_provider(&engine_cfg, &target_instance);
        match rebuilt {
            Ok(p) => {
                slot.native_provider.register(&target_instance, p.clone());
                slot.native_provider.activate(&target_instance);
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
    let previous_model = cur_model.clone();
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

    *slot
        .selection
        .instance
        .write()
        .expect("selection lock poisoned") = Some(target_instance.clone());
    *slot
        .selection
        .model
        .write()
        .expect("selection lock poisoned") = Some(target_model.clone());
    slot.session
        .read()
        .await
        .set_engine_instance(Some(target_instance.clone()))
        .await;
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

    // Say what a switch drops instead of dropping it silently (#88).
    let switched = cur_instance.as_deref() != Some(target_instance.as_str())
        || previous_model.as_deref() != Some(target_model.as_str());
    if is_native && switched {
        let settings = mira_engine::native::native_settings(&engine_cfg, &target_instance);
        let replays_signed = matches!(settings.preset.as_str(), "anthropic" | "bedrock")
            || settings
                .entry
                .base_url
                .as_deref()
                .is_some_and(|u| u.contains("anthropic.com"));
        let history = slot.session.read().await.transcript().await;
        if let Some(text) = reasoning_loss_warning(&history, replays_signed, &target_model) {
            let _ = slot.events_tx.send(ServerMsg::Warning { text });
        }
    }

    if is_native {
        // Best-effort catalog lookup: if the provider reported a real
        // context length for this model, hand it to the harness so
        // compaction plans against the true window instead of the
        // prefix-match table.
        let provider = slot.native_provider.clone();
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
pub(crate) async fn send_input(
    state: &AppState,
    slot: &Arc<SessionSlot>,
    text: String,
    images: Vec<mira_core::ImageData>,
    input_id: Option<String>,
) -> Result<(), String> {
    let _input_dispatch = slot.engine.input_dispatch.lock().await;
    if slot
        .engine
        .retired
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        return Err("This chat was deleted".into());
    }
    if slot.is_foreground_running().await {
        return Err("A foreground turn is already running".into());
    }
    if let Some(id) = input_id.as_deref() {
        let (engine, model, _) = slot.selection.snapshot();
        slot.message_queue
            .validate_recovery(id, engine.as_deref(), model.as_deref())
            .await?;
    }
    slot.message_queue
        .supersede_recovery(input_id.as_deref())
        .await?;
    crate::message_queue::publish(slot).await;
    // User input brings a chat back to its project. Persist this separately
    // from turn-end activity, including for external-agent sessions.
    let session = slot.session.read().await;
    let (pinned, archived_at, _) = session.sidebar_flags().await;
    let settle = mira_harness::persist::SettleMarks::set(false);
    if let Some(store) = &state.store {
        let mut record = store
            .load(&slot.id)
            .await
            .map_err(|e| format!("load: {e}"))?;
        record.settle = settle;
        store
            .save(&record)
            .await
            .map_err(|e| format!("save: {e}"))?;
    }
    session.set_sidebar_flags(pinned, archived_at, settle).await;
    drop(session);

    before_prompt(slot, &text, None).await;
    if slot.acp_launch.lock().await.is_some() {
        prompt_agent(state, slot, text, images, input_id).await;
    } else {
        spawn_turn(state.clone(), slot.clone(), text, images, input_id).await;
    }
    Ok(())
}

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
    input_id: Option<String>,
) {
    let _dispatch = slot.engine.dispatch_lock.lock().await;
    if slot.is_foreground_running().await {
        let _ = slot.events_tx.send(ServerMsg::Warning {
            text:
                "the agent is still working; wait for it to finish before sending another message"
                    .into(),
        });
        return;
    }
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
            crate::acp_host::AcpEventPort::for_slot(slot).turn_ended("start_failed");
            return;
        }
    };
    let Some(agent) = handle.agent().await else {
        let _ = slot.events_tx.send(ServerMsg::Error {
            text: "the external agent has stopped".into(),
        });
        crate::acp_host::AcpEventPort::for_slot(slot).turn_ended("agent_exited");
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
            "user": { "text": text, "images": images.len(), "attached_images":images, "input_id":input_id },
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
                // Agents that can't title their own chat (Codex) get one from
                // Mira's own provider on its cheap model. No provider set up
                // means the first-prompt title simply stays.
                let provider = slot.native_provider.clone();
                tokio::spawn(async move {
                    let title = match agent.generate_title(&description).await {
                        Some(t) => Some(t),
                        None => {
                            let cfg = session.config().await;
                            let users = vec![description.clone()];
                            crate::title::generate(&provider, &cfg.background_model(None), &users)
                                .await
                                .ok()
                                .filter(|t| !t.trim().is_empty())
                        }
                    };
                    if let Some(title) = title {
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

    // Coming from a provider or another agent, or back from a restart, the
    // agent gets what it missed in front of this prompt, so switching
    // engines mid-conversation keeps the thread.
    // The sidecar above records only what the user typed.
    let mut text = match crate::session_engine::take_context_for_agent(slot).await {
        Some(context) => format!("{context}{text}"),
        None => text,
    };

    // Images get to the agent one of two ways: staged into its files
    // dir for agents that read the filesystem (native CLIs), named in
    // the prompt text; or as `image` content blocks inside the
    // `session/prompt` itself for ACP agents, which is the protocol's
    // own media path and needs no filesystem round-trip.
    let inline_images = agent.accepts_image_blocks();
    if !images.is_empty() && !inline_images {
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
                    crate::acp_host::AcpEventPort::for_slot(slot).turn_ended("attachments_failed");
                    return;
                }
            },
            None => {
                let _ = slot.events_tx.send(ServerMsg::Error {
                    text:
                        "this agent was started without an attachments dir, so images were not sent"
                            .into(),
                });
                crate::acp_host::AcpEventPort::for_slot(slot).turn_ended("attachments_failed");
                return;
            }
        }
    }

    // The turn blocks until the agent finishes, so it runs on its own
    // task: the WS reader must stay free to carry permission requests
    // and the cancel that a blocked turn would otherwise prevent.
    let events = slot.events_tx.clone();
    spawn_quiet_agent_watchdog(&events);
    track_agent_turn(slot);
    let turn_port = crate::acp_host::AcpEventPort::for_slot(slot);
    let recovery_slot = slot.clone();
    tokio::spawn(async move {
        let outcome = if inline_images && !images.is_empty() {
            agent.prompt_with_images(&text, &images).await
        } else {
            agent.prompt_text(&text).await
        };
        match outcome {
            // `None` means a native agent, which announces its own
            // turn end when the CLI reports `result`. Announcing it
            // here as well would end the turn twice.
            Ok(Some(stop_reason)) => {
                if stop_reason == "rate_limited" {
                    crate::message_queue::offer_recovery(&recovery_slot, None).await;
                }
                turn_port.turn_ended(&stop_reason);
            }
            Ok(None) => {}
            Err(e) => {
                // The agent's own last words say why ("unknown option",
                // "not logged in"); "stdin closed" alone leaves the user
                // guessing. Give its stderr a moment to drain first.
                tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                let tail = agent.stderr_tail().await;
                let said = tail
                    .lines()
                    .rev()
                    .map(str::trim)
                    .find(|l| !l.is_empty())
                    .map(|l| l.chars().take(300).collect::<String>());
                let _ = events.send(ServerMsg::Error {
                    text: match (explain_agent_error(&e), said) {
                        (Some(why), _) => why,
                        (None, Some(said)) => format!("agent turn failed: {e}. It said: {said}"),
                        (None, None) => format!("agent turn failed: {e}"),
                    },
                });
                // Nothing else will end this turn: the prompt never landed.
                turn_port.turn_ended("error");
            }
        }
    });
}

/// A plain-language reading of agent errors that aren't Mira's to fix but
/// are confusing raw ("-32603 Internal error: OpenAI Chat tool call delta
/// is missing id or name"). `None` leaves the raw text as is.
fn explain_agent_error(e: &str) -> Option<String> {
    let lower = e.to_lowercase();
    if lower.contains("tool call delta") || lower.contains("invalid-output") {
        return Some(
            "The agent's model sent a malformed tool call, so the agent stopped. \
             This comes from the model, not Mira — it's usually a one-off: try \
             again, or pick a different model for the agent."
                .into(),
        );
    }
    None
}

/// Mark the slot as mid-turn until the agent's turn ends (however it ends:
/// done, error, or a Stop), so a window that opens the chat meanwhile knows.
pub(crate) fn track_agent_turn(slot: &Arc<SessionSlot>) {
    slot.engine.touch();
    slot.engine
        .agent_in_turn
        .store(true, std::sync::atomic::Ordering::SeqCst);
    slot.publish_activity();
}

/// An agent that says nothing at all after a prompt is usually waiting on
/// something Mira can't see — a sign-in, a macOS keychain or permission
/// dialog, a network it can't reach. Say so once instead of spinning in
/// silence. Any frame from the agent (text, a tool, the turn ending) means
/// it's alive and ends the watch.
fn spawn_quiet_agent_watchdog(events: &tokio::sync::broadcast::Sender<ServerMsg>) {
    const QUIET: std::time::Duration = std::time::Duration::from_secs(45);
    let mut rx = events.subscribe();
    let tx = events.clone();
    tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + QUIET;
        loop {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                // The agent working on the turn: words, thoughts, tools, an
                // approval it's waiting on, or the end.
                Ok(Ok(
                    ServerMsg::AcpText { .. }
                    | ServerMsg::AcpTextSnapshot { .. }
                    | ServerMsg::AcpToolOutputDelta { .. }
                    | ServerMsg::AcpThought { .. }
                    | ServerMsg::AcpToolCall { .. }
                    | ServerMsg::AcpToolCallUpdate { .. }
                    | ServerMsg::AcpPlan { .. }
                    | ServerMsg::AcpTurnEnd { .. }
                    | ServerMsg::ApprovalRequest { .. }
                    | ServerMsg::ToolStart { .. }
                    | ServerMsg::Error { .. },
                ))
                | Ok(Err(broadcast::error::RecvError::Closed)) => return,
                // Start-up chatter (modes, commands, engine status) and our
                // own notices don't mean the turn is moving.
                Ok(Ok(_)) => continue,
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => return,
                Err(_) => {
                    let _ = tx.send(ServerMsg::Warning {
                        text: "The agent hasn't responded in 45 seconds. It may be waiting on \
                               something outside Mira — a sign-in, a macOS permission or \
                               keychain prompt, or the network. Try running it in a terminal \
                               to see, or press Stop."
                            .into(),
                    });
                    return;
                }
            }
        }
    });
}

async fn approval_rule_strings(state: &AppState, call: &mira_core::ToolCall) -> Vec<String> {
    let Some(tool) = state.base_registry.get(&call.function.name) else {
        // Interactive/agent tools aren't in base_registry — they run at
        // `Action::Pure` so they wouldn't have triggered Ask anyway. Silent
        // no-op is the right thing.
        return Vec::new();
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
    expanded
        .into_iter()
        .map(|t| rule_string_for(tool.action(), &t))
        .filter(|r| !r.is_empty())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
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
    rules: Option<Vec<String>>,
) {
    let rule_strings = match rules {
        Some(rules) => rules,
        None => approval_rule_strings(state, call).await,
    };

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

    let mut persisted = matches!(scope, crate::protocol::ApprovalScope::Always);
    if persisted {
        if let Err(e) = persist_allow_rules(&rule_strings).await {
            persisted = false;
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

/// Drop runs on success, panic and cancellation. Generation ownership
/// prevents an old aborted task from marking its replacement idle.
struct ForegroundTurnGuard {
    slot: Arc<SessionSlot>,
    generation: u64,
}
impl Drop for ForegroundTurnGuard {
    fn drop(&mut self) {
        if self
            .slot
            .engine
            .provider_generation
            .load(std::sync::atomic::Ordering::SeqCst)
            == self.generation
        {
            self.slot
                .engine
                .provider_in_turn
                .store(false, std::sync::atomic::Ordering::SeqCst);
            self.slot.publish_activity();
        }
    }
}

/// Spawn a turn task on `slot`, storing its JoinHandle on the slot so
/// Interrupt and delete_session can tear it down cleanly. Aborts any
/// existing turn handle before starting the new one.
async fn spawn_turn(
    state: AppState,
    slot: Arc<SessionSlot>,
    text: String,
    images: Vec<mira_core::ImageData>,
    input_id: Option<String>,
) {
    // Tools would run against a half-moved tree mid-switch.
    if slot.environments.is_switching() {
        let _ = slot.events_tx.send(ServerMsg::StreamActivity {
            kind: "workspace".into(),
            title: "Workspace is switching".into(),
            detail: "Send again when the environment switch finishes.".into(),
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

    let generation = slot
        .engine
        .provider_generation
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        + 1;
    let foreground_guard = ForegroundTurnGuard {
        slot: slot.clone(),
        generation,
    };
    slot.engine
        .provider_in_turn
        .store(true, std::sync::atomic::Ordering::SeqCst);
    slot.publish_activity();
    let slot_for_task = slot.clone();
    let state_for_task = state.clone();
    let handle = tokio::spawn(async move {
        let _foreground_guard = foreground_guard;
        let sess = slot_for_task.session.read().await.clone();
        let mut stream = sess.send_with_images_and_id(text, images, input_id).await;
        while let Some(evt) = stream.next().await {
            let frame = ServerMsg::from_harness(evt);
            crate::message_queue::observe_limits(&slot_for_task, &frame);
            if let ServerMsg::Error { text } = &frame {
                let lower = text.to_lowercase();
                if lower.contains("rate limit")
                    || lower.contains("rate_limit")
                    || lower.contains("usage limit")
                    || lower.contains("429")
                {
                    crate::message_queue::offer_recovery(&slot_for_task, None).await;
                }
            }
            let _ = slot_for_task.events_tx.send(frame);
        }
        crate::checkpoints::finish_slot(slot_for_task.clone()).await;
        // Turn wrapped up. If this session still needs a nickname, kick off
        // a short generation call on the same provider + model. Runs on
        // its own task so a slow / failing title call doesn't block the
        // next user turn.
        let cfg = sess.config().await;
        let model = cfg.background_model(None);
        let fallback = (model != cfg.model).then(|| cfg.model.clone());
        title::spawn_if_needed(
            sess,
            Arc::new(slot_for_task.native_provider.clone()),
            model,
            fallback,
            state_for_task.clone(),
        );
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

/// Before a prompt runs, snapshot the working tree: once per chat as the
/// baseline its changes are measured from (`session_changes`), and once per
/// message as the checkpoint it can be restored to (`checkpoints`).
/// `replaces` is the message being edited, for a resend. No-ops outside a
/// repo.
pub(crate) async fn before_prompt(slot: &SessionSlot, text: &str, replaces: Option<(&str, usize)>) {
    let cwd = slot.cwd.read().await.clone();
    let id = slot.id.to_string();
    let text = text.to_string();
    let replaces = replaces.map(|(t, n)| (t.to_string(), n));
    let _ = tokio::task::spawn_blocking(move || {
        crate::session_changes::ensure_baseline(&cwd, &id);
        let replaces = replaces.as_ref().map(|(t, n)| (t.as_str(), *n));
        crate::checkpoints::record(&cwd, &id, &text, replaces);
    })
    .await;
}

#[cfg(test)]
mod reasoning_warning_tests {
    use super::reasoning_loss_warning;
    use mira_core::{Message, ReasoningBlock};

    fn with(blocks: Vec<ReasoningBlock>) -> Message {
        let mut m = Message::assistant("done");
        m.reasoning = blocks;
        m
    }
    fn block(signed: bool) -> ReasoningBlock {
        ReasoningBlock {
            text: "thinking".into(),
            signature: signed.then(|| "sig".into()),
            redacted: None,
        }
    }

    #[test]
    fn warns_only_about_reasoning_the_target_cant_replay() {
        let history = vec![
            Message::user("hi"),
            with(vec![block(true)]),
            with(vec![block(false)]),
            Message::assistant("plain"),
        ];
        // To an OpenAI-style model: both replies with reasoning lose it.
        let w = reasoning_loss_warning(&history, false, "gpt-5").unwrap();
        assert!(
            w.contains("2 earlier replies") && w.contains("/compact"),
            "{w}"
        );
        // To Claude: only the unsigned one.
        assert!(reasoning_loss_warning(&history, true, "claude-opus")
            .unwrap()
            .contains("1 earlier reply"));
        // Nothing to lose: no warning.
        assert!(reasoning_loss_warning(&[Message::assistant("x")], false, "gpt-5").is_none());
        assert!(reasoning_loss_warning(&[with(vec![block(true)])], true, "claude").is_none());
    }
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
            mira_core::ImageData {
                source: None,
                media_type: "image/png".into(),
                data: png_data(),
            },
            mira_core::ImageData {
                source: None,
                media_type: "image/jpeg".into(),
                data: png_data(),
            },
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
            source: None,
            media_type: "application/x-sh".into(),
            data: png_data(),
        }];
        assert!(stage_agent_images(&dir, &imgs).is_err());
        let bad = vec![mira_core::ImageData {
            source: None,
            media_type: "image/png".into(),
            data: "!!!not-base64!!!".into(),
        }];
        assert!(stage_agent_images(&dir, &bad).is_err());
    }
}

#[cfg(test)]
mod agent_error_tests {
    use super::explain_agent_error;

    #[test]
    fn explains_a_models_malformed_tool_call() {
        let raw = "acp: agent returned an error: -32603 Internal error: OpenAI Chat tool call delta is missing id or name";
        assert!(explain_agent_error(raw)
            .unwrap()
            .contains("malformed tool call"));
        assert!(explain_agent_error("acp: the agent's stdin closed").is_none());
    }
}

#[cfg(test)]
mod approval_rule_tests {
    use super::rule_string_for;
    use mira_policy::{Request, Rule};
    use mira_tools::Action;

    #[test]
    fn preview_rules_parse_and_match_the_original_targets() {
        for (action, target) in [
            (Action::Read, "src/main.rs"),
            (Action::Edit, "src/main.rs"),
            (Action::Write, "src/new.rs"),
            (Action::Bash, "cargo test --workspace"),
        ] {
            let text = rule_string_for(action, target);
            let rule: Rule = text.parse().unwrap();
            assert!(rule.matches(&Request { action, target }));
        }
    }

    #[test]
    fn edited_bash_rule_allows_arguments_but_not_a_command_chain() {
        let rule: Rule = "Bash(cargo test:*)".parse().unwrap();
        assert!(rule.matches(&Request {
            action: Action::Bash,
            target: "cargo test --workspace"
        }));
        assert!(!rule.matches(&Request {
            action: Action::Bash,
            target: "cargo test && rm -rf /tmp/project"
        }));
        assert!("not a policy rule".parse::<Rule>().is_err());
    }

    #[test]
    fn approval_wire_keeps_edited_rules_and_accepts_legacy_clients() {
        let legacy: crate::protocol::ClientMsg =
            serde_json::from_str(r#"{"type":"approve","call_id":"a","allow":true}"#).unwrap();
        assert!(matches!(
            legacy,
            crate::protocol::ClientMsg::Approve { rules: None, .. }
        ));
        let edited: crate::protocol::ClientMsg = serde_json::from_str(r#"{"type":"approve","call_id":"a","allow":true,"scope":"always","rules":["Read(src/**)","Bash(cargo test:*)"]}"#).unwrap();
        assert!(
            matches!(edited, crate::protocol::ClientMsg::Approve { rules: Some(rules), .. } if rules == vec!["Read(src/**)", "Bash(cargo test:*)"])
        );
    }
}
