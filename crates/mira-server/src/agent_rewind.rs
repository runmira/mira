//! Claude Code's native file rewind (`rewind_files`).
//!
//! Claude checkpoints the files its Write, Edit and NotebookEdit tools change
//! for each prompt in a session. Mira records each prompt's checkpoint UUID,
//! with the Claude session it belongs to, beside that turn in the sidecar.
//! Rewinding restores those files on disk to their state before the prompt.
//! It does not rewind the conversation, and it does not cover Bash commands,
//! subagent edits or edits made outside Claude. A dry run comes first, and
//! the user confirms. Before a real rewind Mira snapshots tracked files, so
//! the rewind itself can be undone.
use crate::{session_scope::ScopedState as State, state::AppState};
use axum::{
    extract::Path,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use mira_core::SessionId;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What Claude's checkpoints cover, shown before every rewind.
pub const COVERAGE: &str = "Restores files changed by Claude's Write, Edit and NotebookEdit tools since this message. Bash commands, subagent edits and edits made outside Claude are not restored, and the conversation is not rewound.";

#[derive(Debug, Deserialize)]
pub struct RewindBody {
    #[serde(default)]
    text: String,
    #[serde(default)]
    occurrence: usize,
    /// Report what would change without touching files.
    #[serde(default)]
    dry_run: bool,
    /// Undo a rewind: restore the snapshot taken just before it.
    #[serde(default)]
    undo: Option<String>,
}

#[derive(Debug, Default, Serialize, PartialEq)]
pub struct RewindView {
    pub dry_run: bool,
    pub can_rewind: bool,
    pub files_changed: Vec<String>,
    pub insertions: Option<u64>,
    pub deletions: Option<u64>,
    pub skipped: Option<u64>,
    pub coverage: &'static str,
    /// Snapshot of tracked files taken before a real rewind.
    pub undo: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Checkpoint {
    pub id: String,
    pub session: Option<String>,
}

fn fail(status: StatusCode, message: impl Into<String>) -> (StatusCode, String) {
    (status, message.into())
}

/// The checkpoint Claude recorded for the `occurrence`-th most recent user
/// message with this text: the first one inside that turn.
pub(crate) fn checkpoint_for(
    lines: &[Value],
    text: &str,
    occurrence: usize,
) -> Result<Checkpoint, (StatusCode, String)> {
    let at = lines
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, l)| l.pointer("/user/text").and_then(Value::as_str) == Some(text))
        .nth(occurrence)
        .map(|(i, _)| i)
        .ok_or_else(|| {
            fail(
                StatusCode::NOT_FOUND,
                "that message isn't in this chat's agent history",
            )
        })?;
    if lines[at].get("driver").and_then(Value::as_str) != Some("claude-code") {
        return Err(fail(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Claude Code did not answer that message, so it has no checkpoint for it",
        ));
    }
    lines[at + 1..]
        .iter()
        .take_while(|l| l.get("user").is_none() || l.pointer("/user/input_intent").and_then(Value::as_str) == Some("steer"))
        .find_map(|l| l.get("claude_checkpoint"))
        .and_then(|c| {
            Some(Checkpoint {
                id: c.get("id")?.as_str()?.to_owned(),
                session: c.get("session").and_then(Value::as_str).map(str::to_owned),
            })
        })
        .ok_or_else(|| {
            fail(
                StatusCode::CONFLICT,
                "Claude has no file checkpoint for that message (it was sent before checkpoints were enabled)",
            )
        })
}

async fn rewind(
    state: &AppState,
    src: &SessionId,
    body: RewindBody,
) -> Result<RewindView, (StatusCode, String)> {
    let slot = state.slot(src).await.ok_or_else(|| {
        fail(
            StatusCode::CONFLICT,
            "open this chat before rewinding its files",
        )
    })?;
    if let Some(hash) = body.undo {
        let cwd = slot.cwd.read().await.clone();
        mira_acp::snapshot::restore_snapshot(&cwd, &hash).map_err(|e| {
            fail(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("could not undo the rewind: {e}"),
            )
        })?;
        return Ok(RewindView {
            can_rewind: true,
            coverage: COVERAGE,
            ..Default::default()
        });
    }
    let _dispatch = slot.engine.dispatch_lock.lock().await;
    let params = slot
        .acp_launch
        .lock()
        .await
        .clone()
        .filter(|p| p.driver_kind == "claude-code")
        .ok_or_else(|| {
            fail(
                StatusCode::BAD_REQUEST,
                "file rewind is available for Claude Code chats",
            )
        })?;
    if slot
        .engine
        .agent_in_turn
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        return Err(fail(
            StatusCode::CONFLICT,
            "Claude is still working; stop it or wait, then rewind",
        ));
    }
    let lines = state
        .store
        .as_ref()
        .and_then(|s| s.agent_log_path(src))
        .map(|p| mira_acp::agent_sessions::read_lines(&p))
        .unwrap_or_default();
    let checkpoint = checkpoint_for(&lines, &body.text, body.occurrence)?;
    // Checkpoints belong to the session that made them; the runtime resumes
    // the chat's cursor, which must be that session.
    let cursor = mira_acp::agent_sessions::read(&src.to_string(), &params.instance);
    let resumes = cursor
        .as_ref()
        .filter(|c| !c.fork_pending)
        .map(|c| c.agent_session_id.as_str());
    if checkpoint.session.is_some() && checkpoint.session.as_deref() != resumes {
        return Err(fail(
            StatusCode::CONFLICT,
            "that checkpoint belongs to an earlier Claude session (for example, the chat this one was forked from)",
        ));
    }
    drop(_dispatch);
    let handle = crate::session_engine::ensure_agent(state, &slot)
        .await
        .map_err(|e| fail(StatusCode::BAD_GATEWAY, e))?;
    let agent = handle
        .agent()
        .await
        .ok_or_else(|| fail(StatusCode::BAD_GATEWAY, "Claude Code stopped"))?;
    let _dispatch = slot.engine.dispatch_lock.lock().await;
    let undo = if body.dry_run {
        None
    } else {
        let cwd = slot.cwd.read().await.clone();
        mira_acp::snapshot::snapshot_turn(&cwd)
    };
    let result = agent
        .rewind_files(&checkpoint.id, body.dry_run)
        .await
        .ok_or_else(|| {
            fail(
                StatusCode::BAD_REQUEST,
                "this agent has no native file rewind",
            )
        })?
        .map_err(|e| {
            fail(
                StatusCode::BAD_GATEWAY,
                format!("Claude could not rewind files: {e}"),
            )
        })?;
    if !body.dry_run && result.can_rewind && result.error.is_none() {
        if let Some(path) = state.store.as_ref().and_then(|s| s.agent_log_path(src)) {
            mira_acp::agent_sessions::append_line_to(
                &path,
                &serde_json::json!({
                    "t": mira_harness::persist::now_ms(),
                    "driver": "claude-code",
                    "file_rewind": { "checkpoint": checkpoint.id, "files": result.files_changed, "undo": undo },
                }),
            );
        }
    }
    Ok(RewindView {
        dry_run: body.dry_run,
        can_rewind: result.can_rewind && result.error.is_none(),
        files_changed: result.files_changed,
        insertions: result.insertions,
        deletions: result.deletions,
        skipped: result.skipped,
        coverage: COVERAGE,
        undo: if body.dry_run { None } else { undo },
        error: result.error,
    })
}

/// `POST /api/sessions/:id/agent-file-rewind`
pub async fn rewind_files(
    State(state): State,
    Path(id): Path<String>,
    Json(body): Json<RewindBody>,
) -> Response {
    match rewind(&state, &SessionId::from(id.as_str()), body).await {
        Ok(view) => Json(view).into_response(),
        Err((status, error)) => {
            (status, Json(serde_json::json!({ "error": error }))).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn user(text: &str, driver: &str) -> Value {
        json!({ "driver": driver, "user": { "text": text } })
    }
    fn cp(id: &str, session: &str) -> Value {
        json!({ "driver": "claude-code", "claude_checkpoint": { "id": id, "session": session } })
    }

    #[test]
    fn each_message_maps_to_its_own_turns_checkpoint() {
        let lines = vec![
            user("one", "claude-code"),
            cp("u1", "s"),
            json!({ "frame": {} }),
            user("two", "claude-code"),
            json!({ "driver": "claude-code", "user": { "text": "also", "input_intent": "steer" } }),
            cp("u2", "s"),
            user("two", "claude-code"),
            cp("u3", "s"),
        ];
        assert_eq!(checkpoint_for(&lines, "one", 0).unwrap().id, "u1");
        assert_eq!(
            checkpoint_for(&lines, "two", 0).unwrap().id,
            "u3",
            "latest occurrence first"
        );
        assert_eq!(
            checkpoint_for(&lines, "two", 1).unwrap().id,
            "u2",
            "a steer stays in its turn"
        );
        assert_eq!(
            checkpoint_for(&lines, "one", 0).unwrap().session.as_deref(),
            Some("s")
        );
    }

    #[test]
    fn missing_checkpoints_and_other_engines_are_reported_not_guessed() {
        let lines = vec![
            user("old", "claude-code"),
            user("next", "claude-code"),
            cp("u", "s"),
        ];
        assert_eq!(
            checkpoint_for(&lines, "old", 0).unwrap_err().0,
            StatusCode::CONFLICT
        );
        let codex = vec![user("hi", "codex"), cp("u", "s")];
        assert_eq!(
            checkpoint_for(&codex, "hi", 0).unwrap_err().0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(
            checkpoint_for(&codex, "nope", 0).unwrap_err().0,
            StatusCode::NOT_FOUND
        );
    }
}
