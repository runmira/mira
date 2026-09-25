//! MCP servers API for the Plugins page.
//!
//! Reads live status from the MCP manager (connections are live: adding,
//! editing, enabling or approving a server takes effect immediately, no
//! restart). Writes go to the file for the server's scope:
//!
//! - `user`: `mcp_servers:` in `~/.mira/mira.yaml`
//! - `project`: `.mcp.json` in the project (Claude Code's format)
//! - `local`: this project only, in `~/.mira/mcp/state.json`
//!
//! Plugin servers come from their plugin and can only be turned off.

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::Json;
use mira_config::{McpServerConfig, MiraConfig};
use mira_mcp::{ConfigProblem, McpState, ServerView};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::protocol::ServerMsg;
use crate::state::AppState;

#[derive(Serialize)]
pub struct McpListView {
    pub servers: Vec<ServerView>,
    pub problems: Vec<ConfigProblem>,
    pub user_config_path: String,
    pub project: Option<String>,
    /// Names of saved `${VAR}` values (never the values).
    pub saved_variables: Vec<String>,
}

pub(crate) fn error_json(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

pub async fn get_mcp(State(state): State<AppState>) -> Json<McpListView> {
    Json(list_view(&state))
}

fn list_view(state: &AppState) -> McpListView {
    let ext = &state.extensions;
    McpListView {
        servers: ext.mcp().servers(),
        problems: ext.problems(),
        saved_variables: ext.mcp().saved_variables(),
        user_config_path: mira_config::global_path().display().to_string(),
        project: ext.project().map(|p| p.display().to_string()),
    }
}

/// Reload extensions and tell every open tab.
pub(crate) async fn changed(state: &AppState) {
    state.extensions.reload().await;
    state.broadcast_all(ServerMsg::ExtensionsChanged).await;
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteScope {
    User,
    Project,
    Local,
}

#[derive(Deserialize)]
pub struct SaveServer {
    pub name: String,
    pub scope: WriteScope,
    /// A server entry in `.mcp.json` / `mira.yaml` shape.
    pub config: Value,
    /// When renaming or moving: the entry to replace.
    #[serde(default)]
    pub replaces: Option<Replaces>,
}

#[derive(Deserialize)]
pub struct Replaces {
    pub name: String,
    pub scope: WriteScope,
}

fn valid_server_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Add or update a server.
pub async fn save_server(State(state): State<AppState>, Json(req): Json<SaveServer>) -> Response {
    let name = req.name.trim().to_owned();
    if !valid_server_name(&name) {
        return error_json(
            StatusCode::BAD_REQUEST,
            "Use letters, digits, `-`, `_` or `.` for the name (up to 64).",
        );
    }
    let cfg = match mira_mcp::spec::config_from_json(&req.config) {
        Ok(c) => c,
        Err(e) => return error_json(StatusCode::BAD_REQUEST, e),
    };
    if let Some(r) = &req.replaces {
        if let Err(e) = remove_from(&state, &r.name, &r.scope) {
            return error_json(StatusCode::BAD_REQUEST, e);
        }
    }
    if let Err(e) = write_to(&state, &name, &req.scope, Some(cfg)) {
        return error_json(StatusCode::BAD_REQUEST, e);
    }
    // A project server you just wrote yourself is approved.
    if matches!(req.scope, WriteScope::Project) {
        state.extensions.reload().await;
        let _ = state.extensions.mcp().set_approved(&name, true);
    }
    changed(&state).await;
    Json(list_view(&state)).into_response()
}

#[derive(Deserialize)]
pub struct ScopeQuery {
    pub scope: WriteScope,
}

pub async fn delete_server(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
    Query(q): Query<ScopeQuery>,
) -> Response {
    if let Err(e) = remove_from(&state, &name, &q.scope) {
        return error_json(StatusCode::BAD_REQUEST, e);
    }
    changed(&state).await;
    Json(list_view(&state)).into_response()
}

fn remove_from(state: &AppState, name: &str, scope: &WriteScope) -> Result<(), String> {
    write_to(state, name, scope, None)
}

/// Write (or with `None`, delete) one entry in a scope's file.
fn write_to(
    state: &AppState,
    name: &str,
    scope: &WriteScope,
    cfg: Option<McpServerConfig>,
) -> Result<(), String> {
    match scope {
        WriteScope::User => {
            let mut c = MiraConfig::load_global().map_err(|e| format!("{e:#}"))?;
            match cfg {
                Some(cfg) => {
                    c.mcp_servers.insert(name.to_owned(), cfg);
                }
                None => {
                    c.mcp_servers.remove(name);
                }
            }
            c.save_global().map_err(|e| format!("{e:#}"))?;
        }
        WriteScope::Local => {
            let project = state
                .extensions
                .project()
                .ok_or("open a project folder first")?;
            let path = &state.extensions.mcp().options().state_file;
            let mut st = McpState::load(path).map_err(|e| format!("{e:#}"))?;
            st.set_local_server(&project, name, cfg);
            st.save(path).map_err(|e| format!("{e:#}"))?;
        }
        WriteScope::Project => {
            let project = state
                .extensions
                .project()
                .ok_or("open a project folder first")?;
            let path = project.join(".mcp.json");
            let mut doc: Value = match std::fs::read_to_string(&path) {
                Ok(t) => serde_json::from_str(&t)
                    .map_err(|e| format!("{} isn't valid JSON: {e}", path.display()))?,
                Err(_) => json!({}),
            };
            if !doc.is_object() {
                return Err(format!("{} isn't a JSON object", path.display()));
            }
            let servers = doc
                .as_object_mut()
                .unwrap()
                .entry("mcpServers")
                .or_insert_with(|| json!({}));
            let servers = servers
                .as_object_mut()
                .ok_or_else(|| format!("`mcpServers` in {} isn't an object", path.display()))?;
            match cfg {
                Some(cfg) => {
                    servers.insert(
                        name.to_owned(),
                        serde_json::to_value(cfg).unwrap_or_default(),
                    );
                }
                None => {
                    servers.remove(name);
                }
            }
            let text = serde_json::to_string_pretty(&doc).unwrap_or_default() + "\n";
            std::fs::write(&path, text).map_err(|e| format!("write {}: {e}", path.display()))?;
        }
    }
    Ok(())
}

async fn act(state: &AppState, result: Result<(), String>) -> Response {
    match result {
        Ok(()) => {
            state.broadcast_all(ServerMsg::ExtensionsChanged).await;
            Json(list_view(state)).into_response()
        }
        Err(e) => error_json(StatusCode::BAD_REQUEST, e),
    }
}

pub async fn reconnect(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> Response {
    let r = state.extensions.mcp().reconnect(&name);
    act(&state, r).await
}

#[derive(Deserialize)]
pub struct Enabled {
    pub enabled: bool,
}

pub async fn set_enabled(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
    Json(b): Json<Enabled>,
) -> Response {
    let r = state.extensions.mcp().set_enabled(&name, b.enabled);
    act(&state, r).await
}

#[derive(Deserialize)]
pub struct SetVariable {
    pub name: String,
    /// `None` or empty removes it.
    #[serde(default)]
    pub value: Option<String>,
}

/// Save a value for a `${VAR}` used by server definitions (a token, say),
/// in `~/.mira/mcp/variables.json`. Servers using it reconnect.
pub async fn set_variable(State(state): State<AppState>, Json(b): Json<SetVariable>) -> Response {
    let r = state
        .extensions
        .mcp()
        .set_variable(b.name.trim(), b.value.as_deref());
    act(&state, r).await
}

#[derive(Deserialize)]
pub struct Approval {
    pub approve: bool,
}

pub async fn set_approval(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
    Json(b): Json<Approval>,
) -> Response {
    let r = state.extensions.mcp().set_approved(&name, b.approve);
    act(&state, r).await
}

/// Start OAuth; the browser comes back to `/api/mcp/oauth/callback`.
pub async fn sign_in(State(state): State<AppState>, AxumPath(name): AxumPath<String>) -> Response {
    let redirect = format!(
        "http://127.0.0.1:{}/api/mcp/oauth/callback",
        state.local_port
    );
    match state.extensions.mcp().begin_sign_in(&name, &redirect).await {
        Ok(url) => Json(json!({ "url": url })).into_response(),
        Err(e) => error_json(StatusCode::BAD_REQUEST, e),
    }
}

pub async fn sign_out(State(state): State<AppState>, AxumPath(name): AxumPath<String>) -> Response {
    let r = state.extensions.mcp().sign_out(&name).await;
    act(&state, r).await
}

/// Where the provider sends the browser after sign-in.
pub async fn oauth_callback(State(state): State<AppState>, uri: axum::http::Uri) -> Response {
    let url = format!(
        "http://127.0.0.1:{}{}",
        state.local_port,
        uri.path_and_query().map(|p| p.as_str()).unwrap_or("/")
    );
    let result = state.extensions.mcp().finish_sign_in(&url).await;
    state.broadcast_all(ServerMsg::ExtensionsChanged).await;
    let page = match result {
        Ok(server) => mira_auth::loopback::html_page(
            "Signed in",
            &format!(
                "Mira is connected to <b>{}</b>. You can close this tab.",
                html_escape(&server)
            ),
            true,
        ),
        Err(e) => mira_auth::loopback::html_page("Sign-in failed", &html_escape(&e), false),
    };
    Html(page).into_response()
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Custom commands and MCP prompts for the composer's `/` palette.
pub async fn list_commands(State(state): State<AppState>) -> impl IntoResponse {
    Json(state.extensions.commands())
}
