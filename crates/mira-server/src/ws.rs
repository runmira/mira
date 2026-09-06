//! WebSocket handler.
//!
//! One task per connection reads client frames; another forwards server-side
//! broadcast frames to the same socket. The two halves close together when
//! either side hangs up.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use futures::{SinkExt, StreamExt};
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

use crate::approver;
use crate::protocol::{ClientMsg, ServerMsg};
use crate::state::AppState;

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
    ServerMsg::Ready {
        session_id: sess.id.to_string(),
        model: cfg.model,
        mode,
        cwd: state.current_cwd().await.display().to_string(),
        history,
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
        ClientMsg::SetModel { model } => {
            state.current_session().await.set_model(&model).await;
            let _ = state.events_tx.send(ServerMsg::ModelChanged { model });
        }
        ClientMsg::SetMode { mode } => {
            state.policy.lock().await.set_mode(mode);
            let _ = state.events_tx.send(ServerMsg::ModeChanged { mode });
        }
        ClientMsg::Interrupt => {
            let cancelled = state.current_session().await.cancel().await;
            let text = if cancelled {
                "turn interrupted".into()
            } else {
                "nothing to interrupt".into()
            };
            let _ = state.events_tx.send(ServerMsg::Warning { text });
            // Send `Done` so the UI clears its busy/thinking state instead
            // of spinning forever waiting for the aborted stream.
            let _ = state.events_tx.send(ServerMsg::Done);
        }
        ClientMsg::Sync => {
            let ready = build_ready(state).await;
            let _ = state.events_tx.send(ready);
        }
    }
}

fn spawn_turn(state: AppState, text: String) {
    tokio::spawn(async move {
        let sess = state.current_session().await;
        let mut stream = sess.send(text).await;
        while let Some(evt) = stream.next().await {
            let frame = ServerMsg::from_harness(evt);
            // If no clients are subscribed, sends error — that's fine.
            let _ = state.events_tx.send(frame);
        }
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
