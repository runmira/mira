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
/// Driver kinds (`codex`) route at that kind's default instance; engine
/// registry ids (`codex-work`) route at the instance, with its own home,
/// key and environment. Anything else is rejected before a process
/// starts, so a typo cannot silently delegate to the ambient account.
pub fn engines() -> Vec<&'static str> {
    let mut out = vec!["mira"];
    out.extend([
        "claude-code",
        "codex",
        "opencode",
        "gemini",
        "grok",
        "cursor",
    ]);
    out.retain(|e| *e == "mira" || mira_acp::drivers::by_kind(e).is_some());
    out
}

/// Whether `engine` names something delegable: the static kinds above or
/// any external instance in the registry.
fn is_delegable(state: &AppState, engine: &str) -> bool {
    if engines().contains(&engine) {
        return true;
    }
    state.engines.current().get(engine).is_some_and(|i| !i.is_native())
}

/// Run `prompt` on `engine` in a child of `parent`; the final answer, or why
/// there isn't one.
///
/// `parent_call_id` is the `delegate_task` call id the parent's transcript
/// rendered. Every visible step the child takes is re-broadcast on the
/// parent's channel as a `DelegateProgress` frame carrying this id, so the
/// delegation card shows live activity rather than a bare spinner.
pub async fn delegate(
    state: &AppState,
    parent: &Arc<SessionSlot>,
    prompt: &str,
    engine: Option<&str>,
    parent_call_id: &str,
) -> Result<String, String> {
    if parent.session.read().await.is_subagent() {
        return Err("a delegated task can't delegate again".into());
    }
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err("`prompt` is empty".into());
    }
    let engine = engine
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .unwrap_or("mira");
    if !is_delegable(state, engine) {
        // Name the static kinds in the error: registry ids are user
        // config, but the kinds are what a model author reaches for.
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
            run_on_mira(parent, &child, &task, parent_call_id).await
        } else {
            run_on_agent(state, parent, &child, engine, &task, parent_call_id).await
        }
    })
    .await
    .unwrap_or_else(|_| {
        Err(format!(
            "the task took longer than {} minutes",
            DELEGATE_TIMEOUT.as_secs() / 60
        ))
    });

    // Nothing of the child keeps running.
    crate::acp_session::stop_agent(&child).await;
    if let Some(h) = child.turn.lock().await.take() {
        h.abort();
    }
    state.remove_slot(&child.id).await;
    result
}

/// Re-broadcast one child step on the parent's channel. Fire-and-forget:
/// with no client attached (a background session) the send just drops.
fn emit_progress(parent: &Arc<SessionSlot>, call_id: &str, kind: &str, text: &str) {
    let _ = parent.events_tx.send(ServerMsg::DelegateProgress {
        call_id: call_id.to_owned(),
        kind: kind.to_owned(),
        text: text.to_owned(),
    });
}

async fn run_on_mira(
    parent: &Arc<SessionSlot>,
    child: &Arc<SessionSlot>,
    task: &str,
    parent_call_id: &str,
) -> Result<String, String> {
    let sess = child.session.read().await.clone();
    let mut stream = sess.send_with_images(task.to_string(), Vec::new()).await;
    // Each harness event the child produces is inspected for a visible step
    // (a tool call, a warning) and forwarded to the parent's card. Token and
    // usage frames are skipped — the card shows steps, not a livestream.
    while let Some(evt) = stream.next().await {
        if let Some((kind, text)) = harness_activity(&evt) {
            emit_progress(parent, parent_call_id, &kind, &text);
        }
    }
    let transcript = sess.transcript().await;
    transcript
        .iter()
        .rev()
        .find(|m| {
            m.role == Role::Assistant && m.content.as_deref().is_some_and(|c| !c.trim().is_empty())
        })
        .and_then(|m| m.content.clone())
        .ok_or_else(|| "Mira's model finished without an answer (is a provider set up?)".into())
}

async fn run_on_agent(
    state: &AppState,
    parent: &Arc<SessionSlot>,
    child: &Arc<SessionSlot>,
    engine: &str,
    task: &str,
    parent_call_id: &str,
) -> Result<String, String> {
    let params = agent_launch_params(state, engine)?;
    let driver = params.driver_kind.clone();
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
    let mut answer_messages = Vec::new();
    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                Ok(frame @ (ServerMsg::AcpText { .. } | ServerMsg::AcpTextSnapshot { .. })) => collect_agent_answer(&mut answer_messages, &frame),
                // The agent's own tool calls, forwarded as card activity.
                Ok(ServerMsg::AcpToolCall { call }) => {
                    let (k, text) = acp_activity(&call);
                    if !text.is_empty() {
                        emit_progress(parent, parent_call_id, &k, &text);
                    }
                }
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
        collect_agent_answer(&mut answer_messages, &m);
    }
    let answer = answer_messages
        .into_iter()
        .map(|(_, text)| text)
        .collect::<Vec<_>>()
        .join("\n\n")
        .trim()
        .to_string();
    if answer.is_empty() {
        Err(format!("{driver} finished without an answer"))
    } else {
        Ok(answer)
    }
}

/// Launch parameters for an external engine. An instance id resolves to its
/// own config (home, key, env); a bare driver kind falls back to that kind's
/// default instance. Either way the agent inherits the instance's credential
/// boundary rather than ambient — work must not bill a stranger's account.
pub(crate) fn agent_launch_params(
    state: &AppState,
    engine: &str,
) -> Result<crate::acp_session::AcpLaunchParams, String> {
    Ok(match state.engines.current().get(engine).filter(|i| !i.is_native()) {
        Some(inst) => {
            let kind = inst.driver.to_string();
            let cfg = state
                .engines
                .current()
                .external_driver_config(engine)
                .unwrap_or_default();
            crate::acp_session::AcpLaunchParams::for_instance(engine, kind, cfg)
        }
        None => {
            let resolved = crate::acp_session::resolve_start_params(None, None, Some(engine))?;
            crate::acp_session::AcpLaunchParams::for_instance(
                resolved.instance,
                resolved.kind,
                resolved.cfg,
            )
        }
    })
}

/// Whether `engine` can run a task: `mira` or an external agent.
pub(crate) fn is_engine(state: &AppState, engine: &str) -> bool {
    is_delegable(state, engine)
}

/* ---------- activity summarizers ---------- */

/// A visible step from one of the child's own harness events: a tool call it
/// dispatched, or a warning it raised. `None` for events with nothing to
/// show (`Token`, `Usage`, `Done`, …).
fn harness_activity(evt: &mira_harness::HarnessEvent) -> Option<(String, String)> {
    use mira_harness::HarnessEvent as E;
    match evt {
        E::ToolStart(call) => {
            let args: serde_json::Value =
                serde_json::from_str(&call.function.arguments).unwrap_or(serde_json::Value::Null);
            let (kind, mut text) = tool_activity(&call.function.name, &args);
            if let Some(target) = tool_target(&call.function.name, &args) {
                text.push(' ');
                text.push_str(&target);
            }
            Some((kind.to_string(), text))
        }
        E::Warning(w) => Some(("note".into(), w.clone())),
        _ => None,
    }
}

/// The coarse bucket and verb for one of Mira's own tools. Kept in the same
/// vocabulary as the frontend's `categoryFor` / `infoFor` so a delegated step
/// reads exactly like a step Mira ran itself.
fn tool_activity(name: &str, _args: &serde_json::Value) -> (&'static str, String) {
    let pair = match name {
        "read_file" => ("read", "Read"),
        "write_file" => ("write", "Wrote"),
        "edit_file" => ("edit", "Edited"),
        "grep" => ("search", "Searched for"),
        "glob" => ("search", "Found files matching"),
        "find_symbol" => ("search", "Found"),
        "find_references" => ("search", "Found references to"),
        "find_callers" => ("search", "Found callers of"),
        "web_search" => ("search", "Searched the web for"),
        "web_fetch" => ("fetch", "Fetched"),
        "bash" => ("run", "Ran"),
        "memory_read" | "memory_search" => ("read", "Recalled"),
        "memory_append" | "memory_edit" | "memory_remember" => ("write", "Remembered"),
        "task_list" => ("read", "Listed tasks"),
        "task_get" => ("read", "Read task"),
        _ => ("other", "Used"),
    };
    (pair.0, pair.1.to_string())
}

/// The human-readable target of a tool call — the path, pattern, command or
/// query its row would show. Empty string when the tool has no natural one.
fn tool_target(name: &str, args: &serde_json::Value) -> Option<String> {
    let s = |v: &serde_json::Value| v.as_str().map(str::to_string);
    let short_path = |p: String| match p.rsplit_once('/') {
        Some((_, base)) if p.matches('/').count() >= 2 => format!("…/{base}"),
        _ => p,
    };
    let val = match name {
        "read_file" | "write_file" | "edit_file" => s(&args["path"]).map(short_path),
        "grep" | "glob" | "find_symbol" | "find_references" | "find_callers" | "web_search" => {
            s(&args["pattern"])
                .or_else(|| s(&args["query"]))
                .or_else(|| s(&args["name"]))
        }
        "bash" => s(&args["command"]).map(|c| {
            let one = c.split_whitespace().collect::<Vec<_>>().join(" ");
            if one.chars().count() > 60 {
                format!("{}…", one.chars().take(60).collect::<String>())
            } else {
                one
            }
        }),
        "web_fetch" => s(&args["url"]),
        "memory_read" | "memory_append" | "memory_edit" | "memory_remember" | "memory_search" => {
            s(&args["path"]).or_else(|| s(&args["query"]))
        }
        _ => None,
    };
    val.filter(|v| !v.is_empty())
}

/// One of an external agent's tool calls, as a step for the card. Uses the
/// agent's own `title` when it gave one (`Edit src/main.rs`), else the kind.
fn acp_activity(call: &mira_acp::events::ToolCallState) -> (String, String) {
    use mira_acp::events::AcpToolKind;
    let title = call.title.trim();
    if title.is_empty() {
        return ("other".into(), String::new());
    }
    let kind = match call.kind {
        Some(AcpToolKind::Read) => "read",
        Some(AcpToolKind::Edit) => "edit",
        Some(AcpToolKind::Delete) | Some(AcpToolKind::Move) => "write",
        Some(AcpToolKind::Search) => "search",
        Some(AcpToolKind::Execute) => "run",
        Some(AcpToolKind::Fetch) => "fetch",
        _ => "other",
    };
    (kind.into(), title.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mira_is_always_an_engine_and_unknown_ones_are_not() {
        let e = engines();
        assert_eq!(e[0], "mira");
        assert!(e
            .iter()
            .all(|k| *k == "mira" || mira_acp::drivers::by_kind(k).is_some()));
        assert!(!e.contains(&"nonsense"));
    }

    #[test]
    fn a_read_step_names_its_file_in_the_card_vocabulary() {
        let call = mira_core::ToolCall {
            id: mira_core::ToolCallId::new(),
            kind: mira_core::ToolCallKind::Function,
            function: mira_core::ToolCallFunction::new(
                "read_file",
                r#"{"path":"crates/mira-auth/src/loopback.rs"}"#,
            ),
        };
        let (kind, text) =
            harness_activity(&mira_harness::HarnessEvent::ToolStart(call)).expect("a step");
        assert_eq!(kind, "read");
        assert!(text.starts_with("Read "), "{text}");
        // Long paths get the same `…/tail` shortening the tool rows use.
        assert!(text.ends_with("loopback.rs"), "{text}");
    }

    #[test]
    fn a_search_step_quotes_its_pattern_and_a_bash_step_its_command() {
        let grep = mira_core::ToolCall {
            id: mira_core::ToolCallId::new(),
            kind: mira_core::ToolCallKind::Function,
            function: mira_core::ToolCallFunction::new("grep", r#"{"pattern":"bind_any"}"#),
        };
        let (kind, text) = harness_activity(&mira_harness::HarnessEvent::ToolStart(grep)).unwrap();
        assert_eq!(kind, "search");
        assert_eq!(text, "Searched for bind_any");

        let bash = mira_core::ToolCall {
            id: mira_core::ToolCallId::new(),
            kind: mira_core::ToolCallKind::Function,
            function: mira_core::ToolCallFunction::new("bash", r#"{"command":"cargo test"}"#),
        };
        let (kind, text) = harness_activity(&mira_harness::HarnessEvent::ToolStart(bash)).unwrap();
        assert_eq!(kind, "run");
        assert_eq!(text, "Ran cargo test");
    }

    #[test]
    fn token_frames_are_not_steps() {
        assert!(harness_activity(&mira_harness::HarnessEvent::Token("hi".into())).is_none());
        assert!(harness_activity(&mira_harness::HarnessEvent::Done).is_none());
    }
}

/// Reconcile native messages for delegated answers just as the chat does.
fn collect_agent_answer(messages: &mut Vec<(Option<String>, String)>, frame: &ServerMsg) {
    let (id, text, snapshot) = match frame {
        ServerMsg::AcpText { message_id, text } => (message_id.clone(), text, false),
        ServerMsg::AcpTextSnapshot { message_id, text } => (Some(message_id.clone()), text, true),
        _ => return,
    };
    let existing = if id.is_some() {
        messages.iter_mut().find(|(key, _)| *key == id)
    } else {
        messages.last_mut().filter(|(key, _)| key.is_none())
    };
    if let Some((_, content)) = existing {
        if snapshot {
            *content = text.clone();
        } else {
            content.push_str(text);
        }
    } else {
        messages.push((id, text.clone()));
    }
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    #[test]
    fn partial_delegated_answers_are_replaced_without_duplicates() {
        let mut messages = Vec::new();
        collect_agent_answer(
            &mut messages,
            &ServerMsg::AcpText {
                message_id: Some("one".into()),
                text: "partial".into(),
            },
        );
        let complete = ServerMsg::AcpTextSnapshot {
            message_id: "one".into(),
            text: "full answer".into(),
        };
        collect_agent_answer(&mut messages, &complete);
        collect_agent_answer(&mut messages, &complete);
        assert_eq!(messages, vec![(Some("one".into()), "full answer".into())]);
    }
}
