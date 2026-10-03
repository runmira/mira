//! `delegate_task`: an agent hands a task to another engine and gets its
//! answer back — "have Codex review what you just wrote", "ask Mira's own
//! model to research this".
//!
//! The task runs in a new, hidden child chat (a subagent transcript: it has
//! the caller's chat as parent, so the sidebar leaves it out) in the same
//! folder, on Mira's own model or on an external agent. It's **read-only by
//! construction**: the child runs with nobody watching, and its background
//! mode denies anything that would need approval, so it can read, search
//! and think, but not edit files or run commands. Delegated chats can't
//! delegate again.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use mira_core::Role;

use crate::protocol::ServerMsg;
use crate::slot::{BackgroundMode, SessionSlot};
use crate::state::AppState;

/// Longest a delegated task may run before it's stopped.
const DELEGATE_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// Engines a task can go to: Mira's own model, or an external agent.
pub fn engines() -> Vec<&'static str> {
    let mut out = vec!["mira"];
    out.extend(["claude-code", "codex", "opencode", "gemini", "grok", "cursor"]);
    out.retain(|e| *e == "mira" || mira_acp::drivers::by_kind(e).is_some());
    out
}

/// Run `prompt` on `engine` in a child of `parent`; the final answer, or why
/// there isn't one.
pub async fn delegate(
    state: &AppState,
    parent: &Arc<SessionSlot>,
    prompt: &str,
    engine: Option<&str>,
) -> Result<String, String> {
    if parent.session.read().await.is_subagent() {
        return Err("a delegated task can't delegate again".into());
    }
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err("`prompt` is empty".into());
    }
    let engine = engine.map(str::trim).filter(|e| !e.is_empty()).unwrap_or("mira");
    if !engines().contains(&engine) {
        return Err(format!(
            "unknown engine `{engine}` — use one of: {}",
            engines().join(", ")
        ));
    }

    // The child: same folder and model settings as the parent, parented so
    // it stays out of the sidebar, denying anything that needs approval.
    let cwd = parent.cwd.read().await.clone();
    let cfg = parent.session.read().await.config().await;
    let child = crate::slot::build_slot(cwd, cfg, None, &state.slot_deps()).await;
    {
        let s = child.session.read().await.clone();
        *child.session.write().await = s.with_parent_id(parent.id.clone());
    }
    *child.background_mode.write().await = BackgroundMode::Deny;
    state.insert_slot(child.clone()).await;

    let task = format!(
        "{prompt}\n\n(This is a delegated, read-only task: you can read and search, but \
         edits and commands will be refused. Answer with your findings.)"
    );
    let result = tokio::time::timeout(DELEGATE_TIMEOUT, async {
        if engine == "mira" {
            run_on_mira(&child, &task).await
        } else {
            run_on_agent(state, &child, engine, &task).await
        }
    })
    .await
    .unwrap_or_else(|_| Err(format!("the task took longer than {} minutes", DELEGATE_TIMEOUT.as_secs() / 60)));

    // Nothing of the child keeps running.
    crate::acp_session::stop_agent(&child).await;
    if let Some(h) = child.turn.lock().await.take() {
        h.abort();
    }
    state.remove_slot(&child.id).await;
    result
}

async fn run_on_mira(child: &Arc<SessionSlot>, task: &str) -> Result<String, String> {
    let sess = child.session.read().await.clone();
    let mut stream = sess.send_with_images(task.to_string(), Vec::new()).await;
    while stream.next().await.is_some() {}
    let transcript = sess.transcript().await;
    transcript
        .iter()
        .rev()
        .find(|m| m.role == Role::Assistant && m.content.as_deref().is_some_and(|c| !c.trim().is_empty()))
        .and_then(|m| m.content.clone())
        .ok_or_else(|| "Mira's model finished without an answer (is a provider set up?)".into())
}

async fn run_on_agent(
    state: &AppState,
    child: &Arc<SessionSlot>,
    driver: &str,
    task: &str,
) -> Result<String, String> {
    let (kind, cfg) = crate::acp_session::resolve_start_params(None, None, Some(driver))?;
    let params = crate::acp_session::AcpLaunchParams::new(kind, cfg);
    let mut rx = child.events_tx.subscribe();
    let handle = crate::acp_session::start_agent(state, child, &params, None).await?;
    let agent = handle
        .agent()
        .await
        .ok_or_else(|| format!("{driver} stopped before it started"))?;
    let mut prompted = {
        let agent = agent.clone();
        let task = task.to_string();
        tokio::spawn(async move { agent.prompt_text(&task).await })
    };
    let mut prompt_done = false;
    let mut answer = String::new();
    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                Ok(ServerMsg::AcpText { text }) => answer.push_str(&text),
                Ok(ServerMsg::AcpTurnEnd { .. }) => break,
                Ok(ServerMsg::Error { text }) => return Err(text),
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            },
            // An ACP agent's turn ends when its prompt returns (a native
            // agent's returns at once and ends with a turn-end event).
            r = &mut prompted, if !prompt_done => {
                prompt_done = true;
                match r {
                    Ok(Ok(Some(_))) => break,
                    Ok(Ok(None)) => {}
                    Ok(Err(e)) => return Err(e),
                    Err(e) => return Err(e.to_string()),
                }
            }
        }
    }
    // Text already sent before the end was noticed.
    while let Ok(m) = rx.try_recv() {
        if let ServerMsg::AcpText { text } = m {
            answer.push_str(&text);
        }
    }
    let answer = answer.trim().to_string();
    if answer.is_empty() {
        Err(format!("{driver} finished without an answer"))
    } else {
        Ok(answer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mira_is_always_an_engine_and_unknown_ones_are_not() {
        let e = engines();
        assert_eq!(e[0], "mira");
        assert!(e.iter().all(|k| *k == "mira" || mira_acp::drivers::by_kind(k).is_some()));
        assert!(!e.contains(&"nonsense"));
    }
}
