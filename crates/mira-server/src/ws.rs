//! WebSocket handler.
//!
//! One task per connection reads client frames; another forwards server-side
//! broadcast frames to the same socket. The two halves close together when
//! either side hangs up.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use futures::{SinkExt, StreamExt};
use mira_config::RuntimeState;
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

use crate::approver;
use crate::protocol::{ClientMsg, ServerMsg};
use crate::state::AppState;
use crate::title;

pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    info!("ws: client connected");
    let (mut sink, mut stream) = socket.split();

    // Subscribe before sending Ready so no events are lost between snapshot
    // and the reader task starting.
    let mut rx = state.events_tx.subscribe();

    let ready = build_ready(&state).await;
    if send_json(&mut sink, &ready).await.is_err() {
        return;
    }

    // Forwarder: broadcast → socket.
    let mut forwarder = tokio::spawn(async move {
        loop {
            match rx.recv().await {
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
    });

    // Reader: socket → server actions.
    let reader_state = state.clone();
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
                Ok(cmd) => dispatch(cmd, &reader_state).await,
                Err(e) => {
                    let _ = reader_state.events_tx.send(ServerMsg::Error {
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

async fn build_ready(state: &AppState) -> ServerMsg {
    let sess = state.current_session().await;
    let cfg = sess.config().await;
    let mode = state.policy.lock().await.mode();
    let history = sess.history().await;
    let turns = sess.turns().await;
    let usage = sess.usage().await;
    let tasks = sess.tasks().await;
    let goal = sess.goal().await;
    ServerMsg::Ready {
        session_id: sess.id.to_string(),
        model: cfg.model,
        mode,
        cwd: state.current_cwd().await.display().to_string(),
        history,
        turns,
        usage,
        tasks,
        goal,
    }
}

async fn dispatch(cmd: ClientMsg, state: &AppState) {
    match cmd {
        ClientMsg::Send { text } => {
            debug!(len = text.len(), "ws: send");
            spawn_turn(state.clone(), text);
        }
        ClientMsg::Approve { call_id, allow } => {
            if !approver::resolve(&state.pending, &call_id, allow).await {
                warn!(call_id, "approval for unknown call");
            }
        }
        ClientMsg::PromptResponse {
            prompt_id,
            response,
        } => {
            if !crate::interactive::resolve(&state.prompt_pending, &prompt_id, response).await {
                warn!(prompt_id, "prompt response for unknown id");
            }
        }
        ClientMsg::SetModel { model } => {
            state.current_session().await.set_model(&model).await;
            // Persist so the next restart lands on the same model rather
            // than the yaml default.
            let mut s = RuntimeState::load().unwrap_or_default();
            s.last_model = Some(model.clone());
            if let Err(e) = s.save() {
                warn!(%e, "state.yaml: save failed after model change");
            }
            let _ = state.events_tx.send(ServerMsg::ModelChanged { model });
        }
        ClientMsg::SetMode { mode } => {
            state.policy.lock().await.set_mode(mode);
            let _ = state.events_tx.send(ServerMsg::ModeChanged { mode });
        }
        ClientMsg::SetEffort { effort } => {
            // Normalise: empty string or literal "off" → None (skip the
            // wire field entirely upstream). Anything else passes through
            // as-is so future values (e.g. "very-high") work without a
            // server-side change.
            let normalized = effort
                .as_deref()
                .filter(|v| !v.is_empty() && *v != "off")
                .map(|s| s.to_owned());
            state
                .current_session()
                .await
                .set_reasoning_effort(normalized)
                .await;
        }
        ClientMsg::Interrupt => {
            let sess = state.current_session().await;
            let cancelled = sess.cancel().await;
            let text = if cancelled {
                "turn interrupted".into()
            } else {
                "nothing to interrupt".into()
            };
            // Stamp the in-flight turn so the "Worked for" chip stops
            // ticking and persists an accurate duration on the next
            // reload.
            sess.end_current_turn().await;
            let _ = state.events_tx.send(ServerMsg::Warning { text });
            // Send `Done` so the UI clears its busy/thinking state instead
            // of spinning forever waiting for the aborted stream.
            let _ = state.events_tx.send(ServerMsg::Done);
        }
        ClientMsg::SetGoal {
            condition,
            max_iterations,
            evaluator_model,
        } => {
            let condition = condition.trim().to_owned();
            if condition.is_empty() {
                let _ = state.events_tx.send(ServerMsg::Warning {
                    text: "goal condition cannot be empty".into(),
                });
                return;
            }
            let mut goal = mira_harness::Goal::new(condition.clone());
            if let Some(n) = max_iterations {
                goal = goal.with_max_iterations(n);
            }
            goal = goal.with_evaluator_model(evaluator_model);
            let sess = state.current_session().await;
            sess.set_goal(goal.clone()).await;
            let _ = state.events_tx.send(ServerMsg::GoalSet { goal });
            // Auto-kickoff: the goal loop only tightens after the model
            // has produced at least one clean stop, so simply setting a
            // goal without sending a message would idle forever. Spawn
            // an opening turn on the user's behalf with the goal as the
            // instruction — matches the industry `/goal` pattern where
            // control does not sit waiting for a follow-up prompt.
            let kickoff = format!(
                "Start working toward this standing goal:\n\n{condition}\n\n\
                 Plan briefly if needed, then take the first concrete step. \
                 I'll keep looping automatically until an evaluator agrees \
                 the goal is met.",
            );
            spawn_turn(state.clone(), kickoff);
        }
        ClientMsg::ClearGoal => {
            let sess = state.current_session().await;
            sess.clear_goal().await;
            let _ = state.events_tx.send(ServerMsg::GoalCleared);
        }
        ClientMsg::Sync => {
            let ready = build_ready(state).await;
            let _ = state.events_tx.send(ready);
        }
    }
}

fn spawn_turn(state: AppState, text: String) {
    tokio::spawn(async move {
        // Refresh the skill registry from disk before the turn starts.
        // Cheap — a handful of file reads — and picks up any skill the
        // model wrote via `write_file` on a previous turn (or the user
        // saved from an editor). Without this, the `Skill` tool's spec
        // is snapshotted at server boot and stale for the whole session.
        crate::skills::reload_registry(&state).await;
        let sess = state.current_session().await;
        let mut stream = sess.send(text).await;
        while let Some(evt) = stream.next().await {
            let frame = ServerMsg::from_harness(evt);
            // If no clients are subscribed, sends error — that's fine.
            let _ = state.events_tx.send(frame);
        }
        // Turn wrapped up. If this session still needs a nickname, kick off
        // a short generation call on the same provider + model. Runs on its
        // own task so a slow / failing title call doesn't block the next
        // user turn.
        let model = sess.config().await.model;
        title::spawn_if_needed(
            sess,
            state.harness_provider.clone(),
            model,
            state.events_tx.clone(),
        );
    });
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
