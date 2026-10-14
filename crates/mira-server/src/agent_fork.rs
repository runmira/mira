//! Native forks of external-agent chats, as separate Mira chats.
//!
//! The agent's own session is forked (Codex `thread/fork`, ACP
//! `session/fork`) or marked to fork on first start (Claude's
//! `--resume <id> --fork-session`). A new chat is then bound to the fork:
//! it gets the transcript sidecar through the fork point, the same engine
//! instance and launch settings, and a resume cursor for the new native
//! session. The original chat, its runtime and its cursor are not touched.
//! Nothing is created unless the native fork succeeded.
use crate::{acp_session::AcpLaunchParams, state::AppState};
use axum::http::StatusCode;
use mira_acp::{process::ForkFailure, runtime::ForkCapability};
use mira_core::SessionId;
use serde_json::Value;

/// Where the fork branches off.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Boundary {
    /// Everything so far.
    Whole,
    /// Through the turn opened by the `occurrence`-th most recent user
    /// message with this text (the key edit & resend uses).
    At { text: String, occurrence: usize },
}

#[derive(Debug, PartialEq, Eq)]
pub struct ForkError {
    pub status: StatusCode,
    pub message: String,
}

fn fail(status: StatusCode, message: impl Into<String>) -> ForkError {
    ForkError {
        status,
        message: message.into(),
    }
}

/// What to copy, decided from the source sidecar alone.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ForkPlan {
    /// Sidecar lines `[..cut]` belong to the fork.
    pub cut: usize,
    /// User turns kept (1-based turn numbers `1..=turns`).
    pub turns: u64,
    /// The boundary message's text, for the "forked from" label.
    pub at: String,
    /// The boundary is the newest turn.
    pub latest: bool,
    /// Native turns completed inside the boundary turn, oldest first.
    pub native_turns: Vec<String>,
    /// Harness messages from before the fork point.
    pub harness_len: Option<usize>,
}

fn is_steer(line: &Value) -> bool {
    line.pointer("/user/input_intent").and_then(Value::as_str) == Some("steer")
}

/// Map a boundary onto the sidecar. Only turns this agent instance answered
/// can be forked natively: a message the provider (or another agent)
/// answered is not in the native session.
pub(crate) fn plan(
    lines: &[Value],
    boundary: &Boundary,
    driver: &str,
    instance: &str,
    native_thread: Option<&str>,
) -> Result<ForkPlan, ForkError> {
    let users: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.get("user").is_some())
        .map(|(i, _)| i)
        .collect();
    let Some(&last) = users.last() else {
        return Err(fail(
            StatusCode::CONFLICT,
            "this chat has no agent turns to fork yet; send a prompt first",
        ));
    };
    let at_index = match boundary {
        Boundary::Whole => users
            .iter()
            .rev()
            .copied()
            .find(|&i| !is_steer(&lines[i]))
            .unwrap_or(last),
        Boundary::At { text, occurrence } => users
            .iter()
            .rev()
            .copied()
            .filter(|&i| lines[i].pointer("/user/text").and_then(Value::as_str) == Some(text))
            .nth(*occurrence)
            .ok_or_else(|| {
                fail(
                    StatusCode::NOT_FOUND,
                    "that message isn't in this chat's agent history",
                )
            })?,
    };
    let line = &lines[at_index];
    let owned_by = |l: &Value| {
        l.get("driver").and_then(Value::as_str) == Some(driver)
            && l.get("instance")
                .and_then(Value::as_str)
                .is_none_or(|i| i == instance)
    };
    if !owned_by(line) {
        return Err(fail(
            StatusCode::UNPROCESSABLE_ENTITY,
            "that message was answered before this agent took over the chat; fork from a later message",
        ));
    }
    // The turn runs until the next prompt or engine switch. A steer joined
    // the running turn.
    let cut = lines[at_index + 1..]
        .iter()
        .position(|l| (l.get("user").is_some() && !is_steer(l)) || l.get("switch").is_some())
        .map_or(lines.len(), |p| at_index + 1 + p);
    let turns = lines[..cut]
        .iter()
        .filter(|l| l.get("user").is_some())
        .count() as u64;
    let native_turns = lines[at_index..cut]
        .iter()
        .filter(|l| owned_by(l))
        .filter_map(|l| l.get("native_turn"))
        .filter(|t| {
            native_thread
                .is_none_or(|thread| t.get("thread").and_then(Value::as_str) == Some(thread))
        })
        .filter_map(|t| t.get("turn").and_then(Value::as_str))
        .map(str::to_owned)
        .collect();
    // Harness history recorded at the first engine switch from the cut on
    // is what preceded the fork point; with no later switch, all of it did.
    let harness_len = lines[cut..]
        .iter()
        .filter_map(|l| l.pointer("/switch/harness_len"))
        .find_map(Value::as_u64)
        .map(|n| n as usize);
    Ok(ForkPlan {
        cut,
        turns,
        at: line
            .pointer("/user/text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        // Nothing this agent answered comes after the boundary.
        latest: !lines[cut..]
            .iter()
            .any(|l| l.get("user").is_some() && owned_by(l)),
        native_turns,
        harness_len,
    })
}

/// The fork scope this agent can offer without starting it.
pub(crate) fn scope(driver_kind: &str, transport: mira_acp::driver::Transport) -> ForkCapability {
    use mira_acp::{driver::Transport, native::NativeFlavor};
    let flavor = mira_acp::drivers::by_kind(driver_kind).and_then(|d| d.native_flavor());
    match (transport, flavor) {
        (Transport::Native, Some(NativeFlavor::CodexAppServer)) => ForkCapability::Turn,
        (Transport::Native, Some(NativeFlavor::Claude)) => ForkCapability::Session,
        // ACP forks are negotiated when the helper process initializes.
        _ => ForkCapability::Session,
    }
}

fn native_failure(state: &AppState, instance: &str, name: &str, e: ForkFailure) -> ForkError {
    match e {
        ForkFailure::Unsupported(m) => {
            state
                .agent_capabilities
                .mark_unsupported(instance, crate::agent_capabilities::Feature::Fork);
            fail(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("{name} can't fork natively: {m}"),
            )
        }
        ForkFailure::InProgress(m) => fail(StatusCode::CONFLICT, m),
        ForkFailure::Failed(m) => fail(
            StatusCode::BAD_GATEWAY,
            format!("{name} could not fork the conversation: {m}"),
        ),
    }
}

/// Fork `src` into a new chat bound to a native fork of its agent session.
pub async fn fork_chat(
    state: &AppState,
    src: &SessionId,
    boundary: Boundary,
) -> Result<SessionId, ForkError> {
    let store = state.store.clone().ok_or_else(|| {
        fail(
            StatusCode::BAD_REQUEST,
            "forking needs saved chats (persistence is off)",
        )
    })?;
    let slot = state.slot(src).await;
    // Read the source under its dispatch lock, so a prompt cannot slip in
    // between choosing the boundary and checking whether it is running.
    let (params, lines, running) = {
        let _dispatch = match &slot {
            Some(slot) => Some(slot.engine.dispatch_lock.lock().await),
            None => None,
        };
        if let Some(slot) = &slot {
            slot.session.read().await.clone().save_now().await;
        }
        let record = store
            .load(src)
            .await
            .map_err(|e| fail(StatusCode::NOT_FOUND, format!("load: {e}")))?;
        let meta = record.agent.clone().filter(|a| a.active).ok_or_else(|| {
            fail(
                StatusCode::BAD_REQUEST,
                "this chat is not run by an external agent",
            )
        })?;
        let live = match &slot {
            Some(slot) => slot.acp_launch.lock().await.clone(),
            None => None,
        };
        let params = live.unwrap_or_else(|| AcpLaunchParams::from_meta(&meta));
        let lines = store
            .agent_log_path(src)
            .map(|p| mira_acp::agent_sessions::read_lines(&p))
            .unwrap_or_default();
        let running = slot.as_ref().is_some_and(|s| {
            s.engine
                .agent_in_turn
                .load(std::sync::atomic::Ordering::SeqCst)
        });
        (params, lines, running)
    };
    let name = params.display_name();
    let cursor =
        mira_acp::agent_sessions::read(&src.to_string(), &params.instance).ok_or_else(|| {
            fail(
                StatusCode::CONFLICT,
                format!("{name} has no session to fork yet; send a prompt first"),
            )
        })?;
    let source = cursor.agent_session_id.clone();
    let (transport, launch) = crate::acp_session::engine_launch(state, &params)
        .map_err(|e| fail(StatusCode::BAD_REQUEST, e))?;
    let scope = scope(&params.driver_kind, transport);
    let plan = plan(
        &lines,
        &boundary,
        &params.driver_kind,
        &params.instance,
        (scope == ForkCapability::Turn).then_some(source.as_str()),
    )?;
    if plan.latest && running {
        return Err(fail(
            StatusCode::CONFLICT,
            "that turn is still running; wait for it to finish or stop it, then fork",
        ));
    }
    if !plan.latest && scope != ForkCapability::Turn {
        return Err(fail(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("{name} can only fork the entire conversation, not from an earlier message"),
        ));
    }

    let record = store
        .load(src)
        .await
        .map_err(|e| fail(StatusCode::NOT_FOUND, format!("load: {e}")))?;
    let cwd = record.cwd.clone();
    let new_id = SessionId::new();
    // Ok(Some(id)): a native session exists now. Ok(None): Claude creates it
    // on the fork's first start.
    let forked: Option<String> = match (transport, scope) {
        (mira_acp::driver::Transport::Native, ForkCapability::Turn) => {
            let last_turn_id = if plan.latest {
                None
            } else {
                Some(plan.native_turns.last().cloned().ok_or_else(|| {
                    fail(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "Mira has no native turn record for that message (it predates native forks); fork from the latest message instead",
                    )
                })?)
            };
            let fork = mira_acp::appserver::ThreadFork {
                source_thread_id: source.clone(),
                last_turn_id,
                cwd: Some(cwd.display().to_string()),
                model: params.model.clone(),
                runtime_mode: params.mode_id.clone().unwrap_or_else(|| {
                    mira_acp::appserver::mode_for_permission(params.mode).to_string()
                }),
            };
            Some(
                mira_acp::appserver::fork_thread(&launch, &fork)
                    .await
                    .map_err(|e| native_failure(state, &params.instance, &name, e))?,
            )
        }
        (mira_acp::driver::Transport::Native, _) => None,
        _ => {
            let driver = mira_acp::drivers::by_kind(&params.driver_kind).ok_or_else(|| {
                fail(
                    StatusCode::BAD_REQUEST,
                    format!("unknown agent: {}", params.driver_kind),
                )
            })?;
            Some(
                mira_acp::process::fork_detached(&launch, driver.client_caps(), &source, &cwd)
                    .await
                    .map_err(|e| native_failure(state, &params.instance, &name, e))?,
            )
        }
    };

    let mut meta = record.agent.clone().expect("checked above");
    meta.instance = Some(params.instance.clone());
    meta.model = params.model.clone();
    meta.launch = Some(params.persisted_with_options(meta.launch.as_ref()));
    let keep = plan.harness_len.unwrap_or(record.messages.len());
    let fork = record.fork_agent_chat(new_id.clone(), &plan.at, keep, meta);

    let undo = |sidecar: Option<&std::path::Path>| {
        if let Some(path) = sidecar {
            let _ = std::fs::remove_file(path);
        }
        mira_acp::agent_sessions::remove(&new_id.to_string(), &params.instance);
    };
    let sidecar = store.agent_log_path(&new_id);
    if let Some(path) = sidecar.as_deref() {
        let kept: Vec<String> = lines[..plan.cut]
            .iter()
            // Snapshots past the kept turns describe work this fork never had.
            .filter(|l| {
                l.pointer("/snapshot/turn")
                    .and_then(Value::as_u64)
                    .is_none_or(|t| t <= plan.turns)
            })
            .map(Value::to_string)
            .collect();
        let text = if kept.is_empty() {
            String::new()
        } else {
            kept.join("\n") + "\n"
        };
        if let Err(e) = std::fs::write(path, text) {
            undo(Some(path));
            return Err(fail(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("could not save the fork's transcript: {e}"),
            ));
        }
    }
    match &forked {
        Some(id) => mira_acp::agent_sessions::record(&new_id.to_string(), &params.instance, id),
        None => mira_acp::agent_sessions::record_pending_fork(
            &new_id.to_string(),
            &params.instance,
            &source,
        ),
    }
    if let Err(e) = store.save(&fork).await {
        undo(sidecar.as_deref());
        if let Some(id) = &forked {
            tracing::warn!(native = %id, "native fork created but its chat could not be saved");
        }
        return Err(fail(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("save: {e}"),
        ));
    }
    Ok(new_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn user(text: &str, driver: &str) -> Value {
        json!({ "driver": driver, "instance": driver, "user": { "text": text } })
    }
    fn turn(id: &str, thread: &str) -> Value {
        json!({ "driver": "codex", "instance": "codex", "native_turn": { "thread": thread, "turn": id } })
    }
    fn frame() -> Value {
        json!({ "driver": "codex", "frame": { "type": "acp_text" } })
    }
    fn at(text: &str, occurrence: usize) -> Boundary {
        Boundary::At {
            text: text.into(),
            occurrence,
        }
    }

    fn sidecar() -> Vec<Value> {
        vec![
            user("one", "codex"),
            frame(),
            turn("tu-1", "th"),
            user("two", "codex"),
            frame(),
            turn("child", "sub"),
            turn("tu-2", "th"),
            json!({ "driver": "codex", "user": { "text": "also", "input_intent": "steer" } }),
            turn("tu-3", "th"),
            json!({ "snapshot": { "turn": 3, "hash": "h" } }),
            user("three", "codex"),
            frame(),
        ]
    }

    #[test]
    fn a_boundary_keeps_its_whole_turn_and_names_its_last_native_turn() {
        let lines = sidecar();
        let p = plan(&lines, &at("two", 0), "codex", "codex", Some("th")).unwrap();
        assert_eq!(p.cut, 10, "through the steer, up to the next prompt");
        assert_eq!(p.turns, 3, "the steer is a user line too");
        assert!(!p.latest);
        assert_eq!(
            p.native_turns,
            ["tu-2", "tu-3"],
            "child threads are not boundaries"
        );
        assert_eq!(p.at, "two");
    }

    #[test]
    fn the_latest_turn_or_the_whole_chat_copies_everything() {
        let lines = sidecar();
        let whole = plan(&lines, &Boundary::Whole, "codex", "codex", None).unwrap();
        assert!(whole.latest);
        assert_eq!(whole.cut, lines.len());
        assert_eq!(whole.at, "three");
        let last = plan(&lines, &at("three", 0), "codex", "codex", None).unwrap();
        assert_eq!(last, whole);
    }

    #[test]
    fn turns_another_engine_answered_are_not_native_boundaries() {
        let mut lines = vec![user("from claude", "claude-code")];
        lines.extend(sidecar());
        let e = plan(&lines, &at("from claude", 0), "codex", "codex", None).unwrap_err();
        assert_eq!(e.status, StatusCode::UNPROCESSABLE_ENTITY);
        // Another instance of the same driver is a different account.
        let mut other = user("work", "codex");
        other["instance"] = json!("codex-work");
        let e = plan(&[other], &at("work", 0), "codex", "codex", None).unwrap_err();
        assert_eq!(e.status, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[test]
    fn missing_messages_and_empty_chats_are_reported() {
        let lines = sidecar();
        assert_eq!(
            plan(&lines, &at("nope", 0), "codex", "codex", None)
                .unwrap_err()
                .status,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            plan(&[frame()], &Boundary::Whole, "codex", "codex", None)
                .unwrap_err()
                .status,
            StatusCode::CONFLICT
        );
    }

    #[test]
    fn harness_history_stops_at_the_next_switch() {
        let mut lines = sidecar();
        lines.insert(
            10,
            json!({ "switch": { "harness_len": 4, "to": "provider" } }),
        );
        let p = plan(&lines, &at("two", 0), "codex", "codex", Some("th")).unwrap();
        assert_eq!(p.cut, 10, "the provider's turns are not copied");
        assert_eq!(p.harness_len, Some(4));
        let p = plan(&lines, &Boundary::Whole, "codex", "codex", None).unwrap();
        assert_eq!(p.harness_len, None);
    }

    #[test]
    fn scope_follows_the_transport() {
        use mira_acp::driver::Transport;
        assert_eq!(scope("codex", Transport::Native), ForkCapability::Turn);
        assert_eq!(
            scope("claude-code", Transport::Native),
            ForkCapability::Session
        );
        assert_eq!(scope("opencode", Transport::Acp), ForkCapability::Session);
    }
}
