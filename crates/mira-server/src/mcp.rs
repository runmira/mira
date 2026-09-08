//! MCP plugins API — read/write `mcp_servers:` in `~/.mira/mira.yaml` from
//! the browser, and surface per-server live status captured at boot.
//!
//! Boot: `serve.rs` connects each configured server and hands us a
//! [`Vec<McpBootStatus>`] recording the config-at-boot, resolved tool list,
//! and any connect error. That snapshot is immutable for the lifetime of
//! the process — we never live-reload (yet). On `GET /api/mcp` we join
//! that snapshot with the current on-disk yaml to compute a
//! `restart_required` flag: entries whose yaml has drifted from what
//! actually booted show an amber pill in the UI.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use mira_config::{McpServerConfig, MiraConfig};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::state::AppState;

/// Per-server connect outcome captured once at server startup. The tool
/// list is a flattened `ToolSpec` view so the UI can render "View tools"
/// without a round-trip to the underlying MCP service.
#[derive(Clone, Debug)]
pub struct McpBootStatus {
    pub name: String,
    pub config: McpServerConfig,
    pub tools: Vec<McpToolInfo>,
    /// `Some(msg)` iff `connect` failed. When set, `tools` is empty.
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct McpToolInfo {
    /// Namespaced name exposed to the model (`mcp__<server>__<tool>`).
    pub name: String,
    pub description: String,
}

/// UI view for one entry. Combines the yaml config with boot status.
#[derive(Debug, Serialize)]
pub struct McpServerView {
    pub name: String,
    pub kind: &'static str, // "stdio" | "http"
    pub config: McpServerConfig,
    pub status: McpStatusView,
    /// True when the yaml differs from what actually booted — i.e. the
    /// entry was added or edited since startup and needs a restart to
    /// take effect. UI renders the amber pill in that case.
    pub restart_required: bool,
    pub tools: Vec<McpToolInfo>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum McpStatusView {
    /// Connected at boot with the config that still matches yaml.
    Connected { tool_count: usize },
    /// Connect attempted at boot and failed.
    Error { message: String },
    /// Never attempted — added or edited since boot. Restart to activate.
    NotLoaded,
}

#[derive(Debug, Serialize)]
pub struct McpListView {
    pub servers: Vec<McpServerView>,
    pub config_path: String,
}

pub async fn get_mcp(State(state): State<AppState>) -> Response {
    let cfg = match MiraConfig::load_global() {
        Ok(c) => c,
        Err(e) => {
            return error_json(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("load config: {e}"),
            )
        }
    };
    let view = build_view(&cfg, &state.mcp_boot);
    Json(view).into_response()
}

/// Full replacement of the `mcp_servers:` map. Simpler than granular
/// add/remove; the UI already holds the full set in memory.
#[derive(Debug, Deserialize)]
pub struct McpUpdate {
    pub servers: BTreeMap<String, McpServerConfig>,
}

pub async fn put_mcp(State(state): State<AppState>, Json(update): Json<McpUpdate>) -> Response {
    let mut cfg = match MiraConfig::load_global() {
        Ok(c) => c,
        Err(e) => {
            return error_json(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("load config: {e}"),
            )
        }
    };
    cfg.mcp_servers = update.servers;
    if let Err(e) = cfg.save_global() {
        return error_json(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("save config: {e}"),
        );
    }
    info!(count = cfg.mcp_servers.len(), "mcp servers saved");
    let view = build_view(&cfg, &state.mcp_boot);
    Json(view).into_response()
}

fn build_view(cfg: &MiraConfig, boot: &[McpBootStatus]) -> McpListView {
    let mut servers = Vec::with_capacity(cfg.mcp_servers.len());
    for (name, server_cfg) in &cfg.mcp_servers {
        let boot_hit = boot.iter().find(|b| b.name == *name);
        let (status, tools, restart_required) = match boot_hit {
            None => (McpStatusView::NotLoaded, Vec::new(), true),
            Some(b) if b.config != *server_cfg => {
                // Config changed since boot — the running server is stale
                // relative to what the user just saved.
                (McpStatusView::NotLoaded, Vec::new(), true)
            }
            Some(b) => {
                if let Some(msg) = &b.error {
                    (
                        McpStatusView::Error {
                            message: msg.clone(),
                        },
                        Vec::new(),
                        false,
                    )
                } else {
                    (
                        McpStatusView::Connected {
                            tool_count: b.tools.len(),
                        },
                        b.tools.clone(),
                        false,
                    )
                }
            }
        };
        servers.push(McpServerView {
            name: name.clone(),
            kind: kind_of(server_cfg),
            config: server_cfg.clone(),
            status,
            restart_required,
            tools,
        });
    }
    McpListView {
        servers,
        config_path: mira_config::global_path().display().to_string(),
    }
}

fn kind_of(cfg: &McpServerConfig) -> &'static str {
    match cfg {
        McpServerConfig::Stdio(_) => "stdio",
        McpServerConfig::Http(_) => "http",
    }
}

fn error_json(status: StatusCode, message: String) -> Response {
    (status, Json(serde_json::json!({ "error": message }))).into_response()
}

/// Kept `Arc`-friendly (behind `Arc<Vec<_>>` in AppState) so cloning the
/// state is cheap. `McpBootStatus` isn't `Send + Sync` free — verify.
pub type McpBootSnapshot = Arc<Vec<McpBootStatus>>;
