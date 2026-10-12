//! Local-only computer settings and credentials scoped to desktop control.
mod control;
mod grants;
mod pairing;
pub mod runtime;
#[cfg(test)]
mod tests;
use crate::{
    pairing::{self as devices, Caller},
    state::AppState,
};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use mira_computer::{
    backend::cua::{CuaBackend, TargetWindow},
    remote::{client, json_response, load_connections, save_connections, Connection},
    Delivery,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
#[derive(Clone)]
struct Api {
    app: AppState,
    remote: Arc<crate::remote::Remote>,
}
#[derive(Clone)]
struct ControlApi {
    computer: Arc<runtime::Runtime>,
    machine_id: String,
}
#[derive(Clone, Serialize, Deserialize)]
struct Identity {
    name: String,
    platform: String,
    machine_id: String,
    background_input: bool,
    limitation: String,
    backend: String,
}
#[derive(Serialize)]
struct ComputerView {
    id: String,
    name: String,
    platform: String,
}
#[derive(Serialize)]
struct View {
    enabled: bool,
    target: String,
    delivery: Delivery,
    window: Option<TargetWindow>,
    computers: Vec<ComputerView>,
    clients: Vec<grants::GrantView>,
    can_change: bool,
}
fn failure(status: StatusCode, message: impl ToString) -> Response {
    (
        status,
        Json(serde_json::json!({"error":message.to_string()})),
    )
        .into_response()
}
fn management(caller: &Caller) -> Option<Response> {
    devices::local_only(caller)
}
fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("authorization")?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}
fn connection(id: &str) -> Result<Connection, String> {
    load_connections()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|c| c.id == id)
        .ok_or_else(|| "That computer is no longer paired.".into())
}
async fn can_change(state: &AppState) -> bool {
    let slots = state
        .slots
        .read()
        .await
        .values()
        .cloned()
        .collect::<Vec<_>>();
    for slot in slots {
        if slot.is_foreground_running().await {
            return false;
        }
    }
    true
}
fn computer_name() -> String {
    #[cfg(unix)]
    {
        let mut bytes = [0u8; 256];
        // gethostname writes only into this bounded buffer; keep an explicit trailing sentinel.
        if unsafe { libc::gethostname(bytes.as_mut_ptr().cast(), bytes.len() - 1) } == 0 {
            let length = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
            let name = String::from_utf8_lossy(&bytes[..length]);
            if !name.is_empty() {
                return name.into_owned();
            }
        }
    }
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "Mira computer".into())
}
fn native_identity(machine_id: &str) -> Identity {
    Identity {
        name: computer_name(),
        platform: std::env::consts::OS.into(),
        machine_id: machine_id.into(),
        background_input: false,
        limitation: "Foreground input uses the visible pointer and keyboard on this computer."
            .into(),
        backend: "native".into(),
    }
}
async fn cua_identity(backend: CuaBackend, machine_id: &str) -> Result<Identity, String> {
    let caps = backend.capabilities().await.map_err(|e| e.to_string())?;
    if caps.protocol_version != 1 {
        return Err("Unsupported CUA companion protocol. Update the companion.".into());
    }
    Ok(Identity {
        name: if caps.hostname.is_empty() {
            "This computer".into()
        } else {
            caps.hostname
        },
        platform: if caps.os.is_empty() {
            std::env::consts::OS.into()
        } else {
            caps.os
        },
        machine_id: machine_id.into(),
        background_input: caps.background_input,
        limitation: caps.background_input_limitation,
        backend: "cua".into(),
    })
}
async fn local_identity(api: &Api) -> Result<Identity, String> {
    let remote = api.remote.view();
    let id = remote["machine_id"].as_str().unwrap_or_default();
    match api.app.computer.local_cua(Delivery::Auto, None).await {
        Ok(b) => cua_identity(b, id).await,
        Err(_) => {
            mira_computer::detect().map_err(|e| e.to_string())?;
            Ok(native_identity(id))
        }
    }
}
async fn configured_cua(api: &Api) -> Result<CuaBackend, String> {
    let cfg = api
        .app
        .computer
        .config
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let mut options = mira_tools::builtin::cua_options(&cfg, Some(Delivery::Auto))
        .ok_or("No configured CUA computer")?;
    options.window = None;
    CuaBackend::connect(&options)
        .await
        .map_err(|e| e.to_string())
}
pub fn routes(app: AppState, remote: Arc<crate::remote::Remote>) -> Router {
    let machine_id = remote.view()["machine_id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    Router::new()
        .route("/api/computer", get(settings).put(configure))
        .route("/api/computer/local", get(local_status))
        .route(
            "/api/computer/pairing",
            post(pairing::open).delete(pairing::close),
        )
        .route("/api/computer/pairing/inspect", post(pairing::inspect))
        .route("/api/computer/pairing/connect", post(pairing::connect))
        .route("/api/computer/targets/:id/windows", get(target_windows))
        .route("/api/computer/targets/:id/status", get(target_status))
        .route("/api/computer/targets/:id", delete(unpair))
        .route("/api/computer/clients/:id", delete(revoke))
        .layer(axum::extract::DefaultBodyLimit::max(2 * 1024 * 1024))
        .with_state(Api {
            app: app.clone(),
            remote,
        })
        .merge(control::routes(ControlApi {
            computer: app.computer,
            machine_id,
        }))
}
async fn settings(
    State(api): State<Api>,
    axum::Extension(caller): axum::Extension<Caller>,
) -> Response {
    if let Some(r) = management(&caller) {
        return r;
    }
    let config = api
        .app
        .computer
        .config
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let configured = config
        .cua
        .as_ref()
        .and_then(|c| c.endpoint.as_ref())
        .is_some_and(|e| {
            e.trim_end_matches('/')
                != api
                    .app
                    .computer
                    .local_options
                    .endpoint
                    .trim_end_matches('/')
        });
    let mut computers: Vec<ComputerView> = match load_connections() {
        Ok(v) => v
            .into_iter()
            .map(|c| ComputerView {
                id: c.id,
                name: c.name,
                platform: c.platform,
            })
            .collect(),
        Err(e) => return failure(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    if configured {
        computers.push(ComputerView {
            id: "configured".into(),
            name: "Configured CUA computer".into(),
            platform: "Advanced connection".into(),
        });
    }
    Json(View {
        enabled: config.enabled(),
        target: config.paired.unwrap_or_else(|| {
            if configured {
                "configured".into()
            } else {
                "local".into()
            }
        }),
        delivery: config
            .delivery
            .and_then(|v| v.parse().ok())
            .unwrap_or(Delivery::Background),
        window: config.window.map(|w| TargetWindow {
            id: w.id,
            epoch: w.epoch,
            title: w.title,
            app: w.app,
        }),
        computers,
        clients: api.app.computer.grants.list(),
        can_change: can_change(&api.app).await,
    })
    .into_response()
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Update {
    enabled: bool,
    target: String,
    delivery: Delivery,
    window: Option<TargetWindow>,
}
async fn configure(
    State(api): State<Api>,
    axum::Extension(caller): axum::Extension<Caller>,
    Json(update): Json<Update>,
) -> Response {
    if let Some(r) = management(&caller) {
        return r;
    }
    let changes = api.app.computer.changes.lock().await;
    // Hold the slot map and dispatch gates: neither a new slot nor a prompt can start while the target changes.
    let slots_guard = api.app.slots.read().await;
    let mut slots = slots_guard.values().cloned().collect::<Vec<_>>();
    slots.sort_by_key(|s| s.id.to_string());
    let mut dispatches = Vec::new();
    for slot in &slots {
        dispatches.push(slot.engine.input_dispatch.lock().await);
    }
    for slot in &slots {
        if slot.is_foreground_running().await {
            return failure(
                StatusCode::CONFLICT,
                "Pause active chats before changing the computer or its access.",
            );
        }
    }
    let mut cfg = match mira_config::MiraConfig::load_global() {
        Ok(c) => c,
        Err(e) => return failure(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    cfg.computer.enabled = Some(update.enabled);
    cfg.computer.delivery = Some(update.delivery.to_string());
    cfg.computer.window = update.window.map(|w| mira_config::ComputerWindow {
        id: w.id,
        epoch: w.epoch,
        title: w.title,
        app: w.app,
    });
    cfg.computer.paired = if update.target == "local" || update.target == "configured" {
        None
    } else {
        if let Err(e) = connection(&update.target) {
            return failure(StatusCode::NOT_FOUND, e);
        }
        Some(update.target.clone())
    };
    if update.target == "local" {
        match local_identity(&api).await {
            Ok(i) if i.backend == "native" => {
                if update.enabled && update.delivery != Delivery::Foreground {
                    return failure(StatusCode::BAD_REQUEST,"This computer supports foreground input only. Select Foreground to continue.");
                }
                cfg.computer.cua = None;
                cfg.computer.window = None;
            }
            Ok(_) => {
                cfg.computer.cua = Some(mira_config::CuaConfig {
                    endpoint: Some(api.app.computer.local_options.endpoint.clone()),
                    token_env: None,
                })
            }
            Err(e) => {
                if update.enabled {
                    return failure(StatusCode::BAD_GATEWAY, e);
                }
            }
        }
    }
    if update.target == "configured" && cfg.computer.cua.as_ref().is_none_or(|c| !c.is_set()) {
        return failure(
            StatusCode::NOT_FOUND,
            "The configured CUA connection is no longer available.",
        );
    }
    if update.enabled && update.delivery == Delivery::Background && cfg.computer.window.is_none() {
        return failure(
            StatusCode::BAD_REQUEST,
            "Choose an application window before enabling background computer use.",
        );
    }
    let controller = if update.enabled {
        match mira_tools::builtin::computer_controller(&cfg.computer, Some(update.delivery)).await {
            Ok(c) => Some(c),
            Err(e) => return failure(StatusCode::BAD_GATEWAY, e),
        }
    } else {
        None
    };
    if let Err(e) = cfg.save_global() {
        return failure(StatusCode::INTERNAL_SERVER_ERROR, e);
    }
    api.app.computer.apply(cfg.computer, controller).await;
    drop(dispatches);
    drop(slots);
    drop(slots_guard);
    drop(changes);
    settings(State(api), axum::Extension(caller)).await
}
async fn local_status(
    State(api): State<Api>,
    axum::Extension(caller): axum::Extension<Caller>,
) -> Response {
    if let Some(r) = management(&caller) {
        return r;
    }
    match local_identity(&api).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => failure(StatusCode::SERVICE_UNAVAILABLE, e),
    }
}
async fn target_windows(
    State(api): State<Api>,
    Path(id): Path<String>,
    axum::Extension(caller): axum::Extension<Caller>,
) -> Response {
    if let Some(r) = management(&caller) {
        return r;
    }
    if id == "configured" || id == "local" {
        if id == "local"
            && local_identity(&api)
                .await
                .is_ok_and(|v| v.backend == "native")
        {
            return Json(Vec::<TargetWindow>::new()).into_response();
        }
        let backend = if id == "local" {
            api.app
                .computer
                .local_cua(Delivery::Auto, None)
                .await
                .map_err(|e| e.to_string())
        } else {
            configured_cua(&api).await
        };
        return match backend {
            Ok(b) => match b.list_windows().await {
                Ok(v) => Json(v).into_response(),
                Err(e) => failure(StatusCode::BAD_GATEWAY, e),
            },
            Err(e) => failure(StatusCode::BAD_GATEWAY, e),
        };
    }
    let c = match connection(&id) {
        Ok(c) => c,
        Err(e) => return failure(StatusCode::NOT_FOUND, e),
    };
    let client = match client() {
        Ok(c) => c,
        Err(e) => return failure(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    match client
        .post(format!("{}/api/computer/windows", c.endpoint))
        .bearer_auth(&c.token)
        .send()
        .await
    {
        Ok(r) => match json_response::<Vec<TargetWindow>>(r).await {
            Ok(v) => Json(v).into_response(),
            Err(e) => failure(StatusCode::BAD_GATEWAY, e),
        },
        Err(e) => failure(StatusCode::BAD_GATEWAY, e.without_url()),
    }
}
async fn target_status(
    State(api): State<Api>,
    Path(id): Path<String>,
    axum::Extension(caller): axum::Extension<Caller>,
) -> Response {
    if let Some(r) = management(&caller) {
        return r;
    }
    if id == "local" {
        return local_status(State(api), axum::Extension(caller)).await;
    }
    if id == "configured" {
        return match configured_cua(&api).await {
            Ok(b) => match cua_identity(b, "configured").await {
                Ok(v) => Json(v).into_response(),
                Err(e) => failure(StatusCode::BAD_GATEWAY, e),
            },
            Err(e) => failure(StatusCode::BAD_GATEWAY, e),
        };
    }
    let c = match connection(&id) {
        Ok(c) => c,
        Err(e) => return failure(StatusCode::NOT_FOUND, e),
    };
    let client = match client() {
        Ok(c) => c,
        Err(e) => return failure(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    match client
        .get(format!("{}/api/computer/identity", c.endpoint))
        .bearer_auth(&c.token)
        .send()
        .await
    {
        Ok(r) => match json_response::<Identity>(r).await {
            Ok(v) => Json(v).into_response(),
            Err(e) => failure(StatusCode::BAD_GATEWAY, e),
        },
        Err(e) => failure(StatusCode::BAD_GATEWAY, e.without_url()),
    }
}
async fn revoke(
    State(api): State<Api>,
    Path(id): Path<String>,
    axum::Extension(caller): axum::Extension<Caller>,
) -> Response {
    if let Some(r) = management(&caller) {
        return r;
    }
    match api.app.computer.revoke(&id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => failure(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}
async fn unpair(
    State(api): State<Api>,
    Path(id): Path<String>,
    axum::Extension(caller): axum::Extension<Caller>,
) -> Response {
    if let Some(r) = management(&caller) {
        return r;
    }
    let changes = api.app.computer.changes.lock().await;
    let slots_guard = api.app.slots.read().await;
    let mut slots = slots_guard.values().cloned().collect::<Vec<_>>();
    slots.sort_by_key(|s| s.id.to_string());
    let mut dispatches = Vec::new();
    for slot in &slots {
        dispatches.push(slot.engine.input_dispatch.lock().await);
    }
    for slot in &slots {
        if slot.is_foreground_running().await {
            return failure(
                StatusCode::CONFLICT,
                "Pause active chats before unpairing the computer.",
            );
        }
    }
    let c = match connection(&id) {
        Ok(c) => c,
        Err(e) => return failure(StatusCode::NOT_FOUND, e),
    };
    let client = match client() {
        Ok(c) => c,
        Err(e) => return failure(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    match client
        .post(format!("{}/api/computer/access/revoke", c.endpoint))
        .bearer_auth(&c.token)
        .send()
        .await
    {
        Ok(r) if r.status().is_success() || r.status() == reqwest::StatusCode::UNAUTHORIZED => {}
        _ => {
            return failure(
                StatusCode::BAD_GATEWAY,
                "Could not revoke access on the other computer. Reconnect it and try again.",
            )
        }
    }
    let mut cfg = match mira_config::MiraConfig::load_global() {
        Ok(c) => c,
        Err(e) => return failure(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    if cfg.computer.paired.as_deref() == Some(&id) {
        cfg.computer.enabled = Some(false);
        cfg.computer.paired = None;
        cfg.computer.window = None;
        if let Err(e) = cfg.save_global() {
            return failure(StatusCode::INTERNAL_SERVER_ERROR, e);
        }
        api.app.computer.apply(cfg.computer, None).await;
    }
    let mut connections = match load_connections() {
        Ok(v) => v,
        Err(e) => return failure(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    connections.retain(|c| c.id != id);
    let result = save_connections(&connections);
    drop(dispatches);
    drop(slots);
    drop(slots_guard);
    drop(changes);
    match result {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => failure(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}
