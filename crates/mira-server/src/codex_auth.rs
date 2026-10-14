//! Instance-owned, reconnectable Codex sign-in. No chat or prompt is created.
use crate::{session_scope::ScopedState as State, state::AppState};
use axum::{
    extract::{Path, Query},
    response::{IntoResponse, Response},
    Json,
};
use mira_acp::{
    account::{AccountClient, AccountView, LoginPhase},
    driver::PermissionMode,
};
use serde::Deserialize;
use serde_json::json;
use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    sync::Arc,
};
use tokio::sync::Mutex;

#[derive(Default)]
pub struct AuthRegistry {
    entries: Mutex<HashMap<String, Arc<Mutex<Option<Entry>>>>>,
}
struct Entry {
    fingerprint: u64,
    client: Arc<AccountClient>,
}

fn launch(
    state: &AppState,
    instance: &str,
) -> Result<(u64, mira_acp::driver::LaunchConfig), String> {
    let registry = state.engines.current();
    let inst = registry.get(instance).ok_or("Unknown engine instance")?;
    if inst.driver.as_str() != "codex" || !inst.enabled {
        return Err("Select an enabled Codex instance to sign in".into());
    }
    let cfg = mira_engine::driver_config_for(inst);
    let launch = configured_launch(&cfg)?;
    Ok((launch_fingerprint(&launch), launch))
}

fn configured_launch(
    cfg: &mira_acp::driver::DriverConfig,
) -> Result<mira_acp::driver::LaunchConfig, String> {
    let driver = mira_acp::drivers::CodexDriver;
    let hint = cfg.binary_path.clone().unwrap_or_else(|| "codex".into());
    mira_acp::process::build_native_launch(&driver, cfg, PermissionMode::Ask, &hint, None)
        .ok_or("This Codex instance does not support app-server sign-in".into())
}

fn launch_fingerprint(launch: &mira_acp::driver::LaunchConfig) -> u64 {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    launch.program.hash(&mut hash);
    launch.args.hash(&mut hash);
    for (key, value) in &launch.env {
        key.hash(&mut hash);
        value.hash(&mut hash);
    }
    launch.env_deny.hash(&mut hash);
    hash.finish()
}

impl AuthRegistry {
    async fn client(
        self: &Arc<Self>,
        state: &AppState,
        instance: &str,
        restart: bool,
    ) -> Result<Arc<AccountClient>, String> {
        let (fingerprint, launch) = launch(state, instance)?;
        // Serialize candidate creation. Otherwise concurrent polling/reconnects
        // could create competing callback listeners for the same CLI home.
        let slot = self
            .entries
            .lock()
            .await
            .entry(instance.into())
            .or_default()
            .clone();
        let mut entry = slot.lock().await;
        if let Some(current) = entry.as_ref() {
            if current.fingerprint == fingerprint && (!restart || !current.client.retired()) {
                return Ok(current.client.clone());
            }
        }
        if let Some(old) = entry.take() {
            old.client.cancel().await;
        }
        let client = AccountClient::start(&launch).await?;
        let mut updates = client.subscribe();
        let initial_view = client.snapshot();
        let mut last_account = serde_json::to_value((
            &initial_view.account,
            &initial_view.requirements,
            &initial_view.requires_openai_auth,
            &initial_view.phase,
        ))
        .ok();
        *entry = Some(Entry {
            fingerprint,
            client: client.clone(),
        });
        drop(entry);
        let weak_registry = Arc::downgrade(self);
        let weak_client = Arc::downgrade(&client);
        let mut state = state.clone();
        state.request_slot = None;
        let instance = instance.to_owned();
        tokio::spawn(async move {
            while updates.changed().await.is_ok() {
                let view = updates.borrow_and_update().clone();
                if view.phase == LoginPhase::Pending {
                    continue;
                }
                let (Some(registry), Some(client)) =
                    (weak_registry.upgrade(), weak_client.upgrade())
                else {
                    break;
                };
                if client.retired() {
                    break;
                }
                if !registry.owns(&state, &instance, fingerprint, &client).await {
                    break;
                }
                let signature = serde_json::to_value((
                    &view.account,
                    &view.requirements,
                    &view.requires_openai_auth,
                    &view.phase,
                ))
                .ok();
                if signature == last_account {
                    continue;
                }
                last_account = signature;
                // Reread the account/requirements in the client; then refresh
                // readiness and models. A failed prompt is never resent.
                crate::engines_api::invalidate_account_instance(&instance);
                if let Some(snapshot) = state.engines.current().snapshot_external(&instance).await {
                    if registry.owns(&state, &instance, fingerprint, &client).await {
                        crate::engines_api::remember_external_snapshot(snapshot);
                        for slot in state.list_slots().await {
                            let _dispatch = slot.engine.dispatch_lock.lock().await;
                            let selected = slot.acp_launch.lock().await.clone();
                            if !selected.as_ref().is_some_and(|p| {
                                p.instance == instance
                                    && p.driver_kind == "codex"
                                    && configured_launch(&p.cfg)
                                        .ok()
                                        .is_some_and(|l| launch_fingerprint(&l) == fingerprint)
                            }) {
                                continue;
                            }
                            let agent = match slot.acp_agent.read().await.clone() {
                                Some(handle) => handle.agent().await,
                                None => None,
                            };
                            if let Some(mira_acp::native::AgentHandle::AppServer(codex)) = agent {
                                if let Ok(options) = codex.refresh_account_catalog().await {
                                    if registry.owns(&state, &instance, fingerprint, &client).await
                                    {
                                        let _ = slot.events_tx.send(
                                            crate::protocol::ServerMsg::AcpConfigOptions {
                                                options,
                                                driver: Some("codex".into()),
                                            },
                                        );
                                        crate::session_engine::publish(&state, &slot).await;
                                    }
                                }
                            }
                        }
                    } else {
                        break;
                    }
                }
            }
        });
        Ok(client)
    }
    /// Revoke pending handoffs immediately when a profile changes, is disabled,
    /// or disappears. Saved CLI credentials are never logged out or deleted.
    pub async fn retire_changed(&self, state: &AppState) {
        let slots: Vec<_> = self
            .entries
            .lock()
            .await
            .iter()
            .map(|(id, slot)| (id.clone(), slot.clone()))
            .collect();
        for (instance, slot) in slots {
            let expected = launch(state, &instance).ok().map(|l| l.0);
            let old = {
                let mut entry = slot.lock().await;
                if entry
                    .as_ref()
                    .is_some_and(|e| Some(e.fingerprint) != expected)
                {
                    entry.take().map(|e| e.client)
                } else {
                    None
                }
            };
            if let Some(client) = old {
                crate::engines_api::invalidate_account_instance(&instance);
                if client.snapshot().phase == LoginPhase::Pending {
                    client.cancel().await;
                } else {
                    client.shutdown().await;
                }
            }
        }
    }

    async fn owns(
        &self,
        state: &AppState,
        instance: &str,
        fingerprint: u64,
        client: &Arc<AccountClient>,
    ) -> bool {
        if launch(state, instance).ok().map(|v| v.0) != Some(fingerprint) {
            return false;
        }
        let slot = self.entries.lock().await.get(instance).cloned();
        match slot {
            Some(slot) => slot
                .lock()
                .await
                .as_ref()
                .is_some_and(|e| e.fingerprint == fingerprint && Arc::ptr_eq(&e.client, client)),
            None => false,
        }
    }
}

fn response(view: AccountView) -> Response {
    let mut response = Json(view).into_response();
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}
fn error(message: String) -> Response {
    (
        axum::http::StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({"error":message})),
    )
        .into_response()
}

pub async fn status(
    State(state): State<AppState>,
    Path(instance): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let refresh = query.get("refresh").is_some_and(|v| v == "1");
    match state.codex_auth.client(&state, &instance, refresh).await {
        Ok(client) => {
            if refresh {
                client.refresh().await;
            }
            if !state
                .codex_auth
                .owns(
                    &state,
                    &instance,
                    launch(&state, &instance).map(|v| v.0).unwrap_or_default(),
                    &client,
                )
                .await
            {
                return error(
                    "Codex settings changed while the account was being checked. Check again."
                        .into(),
                );
            }
            response(client.snapshot())
        }
        Err(e) => error(e),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartLogin {
    flow: String,
}
pub async fn start(
    State(state): State<AppState>,
    Path(instance): Path<String>,
    Json(input): Json<StartLogin>,
) -> Response {
    if !matches!(input.flow.as_str(), "browser" | "device") {
        return error("Unknown sign-in flow".into());
    }
    let (fingerprint, _) = match launch(&state, &instance) {
        Ok(v) => v,
        Err(e) => return error(e),
    };
    let client = match state.codex_auth.client(&state, &instance, true).await {
        Ok(c) => c,
        Err(e) => return error(e),
    };
    match client.begin(&input.flow).await {
        Ok(view) => {
            if !state
                .codex_auth
                .owns(&state, &instance, fingerprint, &client)
                .await
            {
                client.cancel().await;
                return error(
                    "Codex settings changed during sign-in. Start again in the selected instance."
                        .into(),
                );
            }
            response(view)
        }
        Err(e) => error(e),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CancelLogin {
    connection_id: String,
    login_id: String,
}

pub async fn cancel(
    State(state): State<AppState>,
    Path(instance): Path<String>,
    Json(input): Json<CancelLogin>,
) -> Response {
    let slot = state
        .codex_auth
        .entries
        .lock()
        .await
        .get(&instance)
        .cloned();
    let client = match slot {
        Some(slot) => slot.lock().await.as_ref().map(|e| e.client.clone()),
        None => None,
    };
    match client {
        Some(client) => match client
            .cancel_attempt(&input.connection_id, &input.login_id)
            .await
        {
            Ok(view) => response(view),
            Err(e) => error(e),
        },
        None => error("No Codex sign-in is pending for this instance".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn account_ownership_changes_with_home_credentials_binary_and_arguments() {
        let cfg = mira_acp::driver::DriverConfig {
            home_path: Some("/tmp/work-codex".into()),
            ..Default::default()
        };
        let first = launch_fingerprint(&configured_launch(&cfg).unwrap());
        let mut changed = cfg.clone();
        changed.home_path = Some("/tmp/personal-codex".into());
        assert_ne!(
            first,
            launch_fingerprint(&configured_launch(&changed).unwrap())
        );
        changed = cfg.clone();
        changed
            .env
            .insert("OPENAI_API_KEY".into(), "fixture-key".into());
        assert_ne!(
            first,
            launch_fingerprint(&configured_launch(&changed).unwrap())
        );
        changed = cfg.clone();
        changed.binary_path = Some("/tmp/other-codex".into());
        assert_ne!(
            first,
            launch_fingerprint(&configured_launch(&changed).unwrap())
        );
        changed = cfg.clone();
        changed.launch_args = vec!["--profile".into(), "work".into()];
        assert_ne!(
            first,
            launch_fingerprint(&configured_launch(&changed).unwrap())
        );
        changed = cfg.clone();
        changed.display_name = Some("A new label".into());
        assert_eq!(
            first,
            launch_fingerprint(&configured_launch(&changed).unwrap())
        );
    }
}
