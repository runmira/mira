//! Chats a restart cut off mid-turn get a way to continue.
//!
//! The activity hub keeps the set of running chats on disk. At startup the
//! set the previous run left behind names the chats whose turn never ended;
//! each gets the recovery card (the same one a usage limit offers) with a
//! "Continue" that resumes the task — checking existing results first —
//! instead of the work silently stopping. With "Continue chats after Mira
//! restarts" on (the default) it's pressed automatically.

use std::path::PathBuf;

use crate::state::AppState;

/// `~/.mira/running-chats.json`.
pub fn path() -> PathBuf {
    mira_config::state_path().with_file_name("running-chats.json")
}

/// The chats the previous run left running, read before this run starts
/// overwriting the file.
pub fn previous(path: &std::path::Path) -> Vec<String> {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Offer each interrupted chat a way to continue.
pub fn spawn(state: AppState, interrupted: Vec<String>) {
    if interrupted.is_empty() {
        return;
    }
    tokio::spawn(async move {
        for id in interrupted {
            let Ok(slot) = state
                .ensure_slot(&mira_core::SessionId::from(id.as_str()))
                .await
            else {
                continue;
            };
            // A delegated helper's task died with the run; its parent
            // can delegate again.
            if slot.session.read().await.is_subagent() {
                continue;
            }
            let (engine, model, _) = slot.selection.snapshot();
            if let Err(e) = slot
                .message_queue
                .offer_restart_recovery(engine, model)
                .await
            {
                tracing::warn!(%e, chat = %id, "restart recovery: could not offer");
                continue;
            }
            // Settings → "Continue chats after Mira restarts" (default on):
            // press Continue on the user's behalf.
            let auto = mira_config::MiraConfig::load_global()
                .map(|c| c.sessions.resume_after_restart())
                .unwrap_or(true);
            let recovery = slot
                .message_queue
                .snapshot()
                .await
                .into_iter()
                .rev()
                .find(|i| i.recovery.as_ref().is_some_and(|r| r.reason.as_deref() == Some("restart")) && !i.delivered)
                .map(|i| i.id);
            if let (true, Some(rid)) = (auto, recovery) {
                match slot.message_queue.update_recovery(&rid, "retry").await {
                    Ok(()) => crate::message_queue::wake(&state, &slot),
                    Err(e) => tracing::warn!(%e, chat = %id, "restart recovery: could not continue"),
                }
            }
            let _ = slot
                .events_tx
                .send(crate::protocol::ServerMsg::QueueUpdated {
                    session_id: id.clone(),
                    items: slot.message_queue.snapshot().await,
                });
            tracing::info!(chat = %id, auto, "restart recovery: offered");
        }
    });
}
