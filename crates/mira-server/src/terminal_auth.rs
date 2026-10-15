//! Terminal sign-in for ACP agents (OpenCode).
//!
//! Credentials stay owned by the agent's CLI. Mira initializes the selected
//! instance's agent with terminal sign-in advertised, then runs the method it
//! offers in a Mira terminal with that instance's binary, home and
//! environment, so the login writes the credential store the agent's prompts
//! read. Terminal methods are never sent to `authenticate`. A zero exit
//! refreshes readiness and models and reconnects idle runtimes on the
//! instance; any other exit, a cancel, or a changed configuration does not.
use crate::{acp_session::AcpLaunchParams, session_scope::ScopedState as State, state::AppState};
use axum::{
    extract::Path,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    #[default]
    Idle,
    /// The sign-in is running in its terminal.
    Running,
    /// It exited 0; readiness and models are being refreshed.
    Refreshing,
    Succeeded,
    Failed,
    Cancelled,
    /// The installed agent offers no terminal sign-in; use `command`.
    Unsupported,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct AuthView {
    pub attempt_id: Option<String>,
    pub phase: Phase,
    /// The Mira terminal running the sign-in, to attach to.
    pub terminal_id: Option<String>,
    pub label: Option<String>,
    /// What runs (or, when unsupported, what to run yourself). Arguments
    /// only; environment values are never shown.
    pub command: Option<String>,
    pub exit_code: Option<u32>,
    pub error: Option<String>,
    /// Models the refreshed readiness probe found.
    pub models: Option<usize>,
    pub ready: Option<bool>,
}

#[derive(Default)]
pub struct TerminalAuthRegistry {
    attempts: Mutex<HashMap<String, Attempt>>,
}

struct Attempt {
    view: AuthView,
    fingerprint: u64,
}

fn params_for(state: &AppState, instance: &str) -> Result<AcpLaunchParams, String> {
    let registry = state.engines.current();
    let inst = registry.get(instance).ok_or("Unknown engine instance")?;
    if !inst.enabled {
        return Err("Enable this agent to sign in".into());
    }
    Ok(AcpLaunchParams::for_instance(
        instance,
        inst.driver.as_str(),
        mira_engine::driver_config_for(inst),
    ))
}

/// The launch an attempt belongs to. A changed binary, home or arguments
/// makes an older attempt stale.
fn fingerprint(state: &AppState, instance: &str) -> Option<u64> {
    let params = params_for(state, instance).ok()?;
    let (_, launch) = crate::acp_session::engine_launch(state, &params).ok()?;
    Some(crate::agent_capabilities::launch_fingerprint(&launch))
}

fn shell_word(word: &str) -> String {
    if !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_./-=:@".contains(c))
    {
        word.to_owned()
    } else {
        format!("'{}'", word.replace('\'', "'\\''"))
    }
}

pub(crate) fn display_command(program: &std::path::Path, args: &[String]) -> String {
    std::iter::once(program.display().to_string())
        .chain(args.iter().cloned())
        .map(|w| shell_word(&w))
        .collect::<Vec<_>>()
        .join(" ")
}

impl TerminalAuthRegistry {
    pub fn view(&self, instance: &str) -> AuthView {
        self.attempts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(instance)
            .map(|a| a.view.clone())
            .unwrap_or_default()
    }

    fn set(&self, instance: &str, attempt: &str, update: impl FnOnce(&mut AuthView)) -> bool {
        let mut attempts = self.attempts.lock().unwrap_or_else(|e| e.into_inner());
        match attempts.get_mut(instance) {
            Some(a) if a.view.attempt_id.as_deref() == Some(attempt) => {
                update(&mut a.view);
                true
            }
            _ => false,
        }
    }

    fn owns(&self, instance: &str, attempt: &str, fingerprint: u64) -> bool {
        self.attempts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(instance)
            .is_some_and(|a| {
                a.view.attempt_id.as_deref() == Some(attempt) && a.fingerprint == fingerprint
            })
    }

    /// Stop whatever this instance is running. The attempt is marked
    /// cancelled first, so its exit is not reported as a failure.
    pub fn cancel(&self, instance: &str) -> AuthView {
        let terminal = {
            let mut attempts = self.attempts.lock().unwrap_or_else(|e| e.into_inner());
            let Some(a) = attempts.get_mut(instance) else {
                return AuthView::default();
            };
            if a.view.phase != Phase::Running {
                return a.view.clone();
            }
            a.view.phase = Phase::Cancelled;
            a.view.terminal_id.take()
        };
        if let Some(id) = terminal {
            crate::terminal::kill_terminal(&id);
        }
        self.view(instance)
    }

    pub async fn start(&self, state: &AppState, instance: &str) -> Result<AuthView, String> {
        let params = params_for(state, instance)?;
        let (transport, launch) = crate::acp_session::engine_launch(state, &params)?;
        if !matches!(transport, mira_acp::driver::Transport::Acp) {
            return Err("This agent signs in through its own CLI outside ACP".into());
        }
        let driver = mira_acp::drivers::by_kind(&params.driver_kind)
            .ok_or_else(|| format!("unknown agent: {}", params.driver_kind))?;
        self.cancel(instance);
        let attempt = uuid::Uuid::new_v4().to_string();
        let fp = crate::agent_capabilities::launch_fingerprint(&launch);
        let plan = mira_acp::process::discover_terminal_auth(&launch, driver.client_caps()).await?;
        let Some(plan) = plan else {
            // Older agents: the CLI's own login, for the user to run.
            let view = AuthView {
                attempt_id: Some(attempt),
                phase: Phase::Unsupported,
                command: driver
                    .underlying_cli_names()
                    .first()
                    .map(|_| display_command(&launch.program, &["auth".into(), "login".into()])),
                error: Some(
                    "This installed version does not offer terminal sign-in over ACP.".into(),
                ),
                ..Default::default()
            };
            self.attempts
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(
                    instance.to_owned(),
                    Attempt {
                        view: view.clone(),
                        fingerprint: fp,
                    },
                );
            return Ok(view);
        };
        let cwd = std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let (terminal_id, mut exit) = crate::terminal::spawn_program(
            &plan.program,
            &plan.args,
            &plan.env,
            &plan.env_deny,
            cwd,
        )
        .map_err(|e| format!("could not open a terminal for sign-in: {e}"))?;
        let view = AuthView {
            attempt_id: Some(attempt.clone()),
            phase: Phase::Running,
            terminal_id: Some(terminal_id),
            label: Some(plan.label.clone()),
            command: Some(display_command(&plan.program, &plan.args)),
            ..Default::default()
        };
        self.attempts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                instance.to_owned(),
                Attempt {
                    view: view.clone(),
                    fingerprint: fp,
                },
            );
        let state = state.clone();
        let instance = instance.to_owned();
        tokio::spawn(async move {
            let code = loop {
                if let Some(code) = *exit.borrow_and_update() {
                    break code;
                }
                if exit.changed().await.is_err() {
                    break None;
                }
            };
            let registry = state.terminal_auth.clone();
            let succeeded = code == Some(0);
            let current = registry.set(&instance, &attempt, |v| {
                if v.phase != Phase::Running {
                    return;
                }
                v.exit_code = code;
                v.terminal_id = None;
                if succeeded {
                    v.phase = Phase::Refreshing;
                } else {
                    v.phase = Phase::Failed;
                    v.error = Some(match code {
                        Some(c) => format!("Sign-in exited with status {c}; nothing was changed."),
                        None => "Sign-in ended without an exit status; nothing was changed.".into(),
                    });
                }
            });
            if !current || !succeeded {
                return;
            }
            if fingerprint(&state, &instance) != Some(fp) || !registry.owns(&instance, &attempt, fp)
            {
                registry.set(&instance, &attempt, |v| {
                    v.phase = Phase::Failed;
                    v.error = Some(
                        "This agent's configuration changed during sign-in. Sign in again.".into(),
                    );
                });
                return;
            }
            refresh_after_sign_in(&state, &instance, &attempt, fp).await;
        });
        Ok(view)
    }
}

/// Re-probe readiness and models, then reconnect idle runtimes on this
/// instance so their next prompt initializes again with the new
/// credentials. A running turn is left alone and nothing is resent.
async fn refresh_after_sign_in(state: &AppState, instance: &str, attempt: &str, fp: u64) {
    let registry = state.terminal_auth.clone();
    crate::engines_api::invalidate_account_instance(instance);
    // A fresh login supersedes what earlier turns showed; the next turn
    // (and the credential check in the probe) says what is still missing.
    state.agent_sign_in.signed_in(instance);
    state
        .broadcast_all(crate::protocol::ServerMsg::AgentSignInChanged {
            instance: instance.to_owned(),
            needed: Vec::new(),
        })
        .await;
    let snapshot = state.engines.current().snapshot_external(instance).await;
    if !registry.owns(instance, attempt, fp) {
        return;
    }
    let (ready, models, reason) = match &snapshot {
        Some(s) => (
            s.state.is_ready(),
            s.models.len(),
            match &s.state {
                mira_engine::EngineState::Ready => None,
                mira_engine::EngineState::NotConfigured { reason }
                | mira_engine::EngineState::Failed { reason }
                | mira_engine::EngineState::Unavailable { reason } => Some(reason.clone()),
                mira_engine::EngineState::NotFound { looked_for } => {
                    Some(format!("{looked_for} was not found"))
                }
            },
        ),
        None => (false, 0, Some("Readiness could not be checked".into())),
    };
    if let Some(snapshot) = snapshot {
        crate::engines_api::remember_external_snapshot(snapshot);
    }
    for slot in state.list_slots().await {
        let selected = slot.acp_launch.lock().await.clone();
        if !selected.is_some_and(|p| p.instance == instance) {
            continue;
        }
        let _dispatch = slot.engine.dispatch_lock.lock().await;
        if slot
            .engine
            .agent_in_turn
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            continue;
        }
        if crate::acp_session::stop_agent(&slot).await {
            crate::session_engine::publish(state, &slot).await;
        }
    }
    registry.set(instance, attempt, |v| {
        v.phase = Phase::Succeeded;
        v.ready = Some(ready);
        v.models = Some(models);
        v.error = match (ready, models) {
            (true, 0) => Some(
                "Signed in, but no models are available yet. Connect the provider your model uses."
                    .into(),
            ),
            (true, _) => None,
            (false, _) => {
                reason.or_else(|| Some("Signed in, but the agent is not ready yet.".into()))
            }
        };
    });
}

fn no_store(body: impl Serialize, status: StatusCode) -> Response {
    let mut response = (status, Json(body)).into_response();
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        "no-store".parse().unwrap(),
    );
    response
}

/// `GET /api/engines/:instance/terminal-auth`
pub async fn status(State(state): State, Path(instance): Path<String>) -> Response {
    no_store(state.terminal_auth.view(&instance), StatusCode::OK)
}

/// `POST /api/engines/:instance/terminal-auth`
pub async fn start(State(state): State, Path(instance): Path<String>) -> Response {
    match state.terminal_auth.clone().start(&state, &instance).await {
        Ok(view) => no_store(view, StatusCode::OK),
        Err(error) => no_store(
            serde_json::json!({ "error": error }),
            StatusCode::BAD_REQUEST,
        ),
    }
}

/// `DELETE /api/engines/:instance/terminal-auth`
pub async fn cancel(State(state): State, Path(instance): Path<String>) -> Response {
    no_store(state.terminal_auth.cancel(&instance), StatusCode::OK)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn running(registry: &TerminalAuthRegistry, attempt: &str) {
        registry.attempts.lock().unwrap().insert(
            "opencode".into(),
            Attempt {
                view: AuthView {
                    attempt_id: Some(attempt.into()),
                    phase: Phase::Running,
                    ..Default::default()
                },
                fingerprint: 7,
            },
        );
    }

    #[test]
    fn a_superseded_attempt_cannot_update_its_successor() {
        let reg = TerminalAuthRegistry::default();
        running(&reg, "old");
        running(&reg, "new");
        assert!(!reg.set("opencode", "old", |v| v.phase = Phase::Succeeded));
        assert_eq!(reg.view("opencode").phase, Phase::Running);
        assert!(
            !reg.owns("opencode", "new", 8),
            "a changed launch is not the owner"
        );
        assert!(reg.owns("opencode", "new", 7));
    }

    #[test]
    fn cancelling_marks_the_attempt_before_its_exit_arrives() {
        let reg = TerminalAuthRegistry::default();
        running(&reg, "a");
        let cancelled = reg.cancel("opencode");
        assert_eq!(cancelled.phase, Phase::Cancelled);
        assert_eq!(
            cancelled.terminal_id, None,
            "the killed terminal is not offered to attach"
        );
        // The exit that follows the kill is not reported as a failure.
        reg.set("opencode", "a", |v| {
            if v.phase == Phase::Running {
                v.phase = Phase::Failed;
            }
        });
        assert_eq!(reg.view("opencode").phase, Phase::Cancelled);
        assert_eq!(
            TerminalAuthRegistry::default().cancel("none").phase,
            Phase::Idle
        );
    }

    #[test]
    fn commands_are_shown_without_environment_values() {
        let shown = display_command(
            std::path::Path::new("/Users/me/My Tools/opencode"),
            &["auth".into(), "login".into()],
        );
        assert_eq!(shown, "'/Users/me/My Tools/opencode' auth login");
    }
}
