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

async fn build_ready(slot: &SessionSlot, state: &AppState) -> ServerMsg {
    let sess = slot.session.read().await.clone();
    let cfg = sess.config().await;
    let mode = state.policy.lock().await.mode();
    let history = sess.history().await;
    let turns = sess.turns().await;
    let usage = sess.usage().await;
    let tasks = sess.tasks().await;
    let goal = sess.goal().await;
    let previews = sess.previews().await;
    let cwd = slot.cwd.read().await.clone();
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
        ClientMsg::Send { text } => {
            debug!(len = text.len(), "ws: send");
            spawn_turn(state.clone(), slot, text).await;
        }
        ClientMsg::Approve { call_id, allow } => {
            if !approver::resolve(&slot.pending, &call_id, allow).await {
                warn!(call_id, "approval for unknown call");
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
        ClientMsg::SetModel { model } => {
            slot.session.read().await.set_model(&model).await;
            let mut s = RuntimeState::load().unwrap_or_default();
            s.last_model = Some(model.clone());
            if let Err(e) = s.save() {
                warn!(%e, "state.yaml: save failed after model change");
            }
            let _ = slot.events_tx.send(ServerMsg::ModelChanged { model });
        }
        ClientMsg::SetMode { mode } => {
            state.policy.lock().await.set_mode(mode);
            let profile = mira_harness::profile_for_mode(mode);
            // Propagate to EVERY slot's session so a background session's
            // next bash call also honors the new profile. Doing this per
            // slot is cheap (a handful of Arc<RwLock<..>> reads).
            for slot_arc in state.list_slots().await {
                slot_arc
                    .session
                    .read()
                    .await
                    .set_sandbox_profile(profile)
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
            spawn_turn(state.clone(), slot.clone(), kickoff).await;
        }
        ClientMsg::ClearGoal => {
            slot.session.read().await.clear_goal().await;
            let _ = slot.events_tx.send(ServerMsg::GoalCleared);
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

/// Spawn a turn task on `slot`, storing its JoinHandle on the slot so
/// Interrupt and delete_session can tear it down cleanly. Aborts any
/// existing turn handle before starting the new one.
async fn spawn_turn(state: AppState, slot: Arc<SessionSlot>, text: String) {
    // Refresh the skill registry from disk before the turn starts.
    crate::skills::reload_registry(&state).await;

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
        let mut stream = sess.send(text).await;
        while let Some(evt) = stream.next().await {
            let frame = ServerMsg::from_harness(evt);
            let _ = slot_for_task.events_tx.send(frame);
        }
        // Turn wrapped up. If this session still needs a nickname, kick off
        // a short generation call on the same provider + model. Runs on
        // its own task so a slow / failing title call doesn't block the
        // next user turn.
        let model = sess.config().await.model;
        title::spawn_if_needed(
            sess,
            state_for_task.harness_provider.clone(),
            model,
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
