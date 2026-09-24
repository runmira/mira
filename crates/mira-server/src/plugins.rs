//! Plugins API: marketplaces, the catalog, installs.
//!
//! Commands, skills and MCP servers of a newly installed or enabled
//! plugin take effect right away. Agents are read when a session starts,
//! so those show up in new sessions.

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use mira_plugins::{CatalogEntry, MarketplaceView};
use serde::{Deserialize, Serialize};

use crate::mcp::{changed, error_json};
use crate::state::AppState;

#[derive(Serialize)]
pub struct InstalledView {
    pub id: String,
    pub name: String,
    pub display_name: Option<String>,
    pub marketplace: String,
    pub version: String,
    pub description: Option<String>,
    pub author: Option<String>,
    pub enabled: bool,
    pub path: String,
    pub updated_at: u64,
    pub commands: Vec<String>,
    pub agents: Vec<String>,
    pub skills: Vec<String>,
    pub mcp_servers: Vec<String>,
    pub hooks: Vec<String>,
    pub lsp_servers: Vec<String>,
    pub problems: Vec<String>,
}

#[derive(Serialize)]
pub struct PluginsOverview {
    pub marketplaces: Vec<MarketplaceView>,
    pub catalog: Vec<CatalogEntry>,
    pub installed: Vec<InstalledView>,
    /// Marketplaces worth offering when none are added yet.
    pub suggested_marketplaces: Vec<Suggested>,
}

#[derive(Serialize, Clone, Copy)]
pub struct Suggested {
    pub source: &'static str,
    pub name: &'static str,
    pub description: &'static str,
}

const SUGGESTED: &[Suggested] = &[
    Suggested {
        source: "anthropics/claude-plugins-official",
        name: "claude-plugins-official",
        description:
            "Anthropic's directory of Claude Code plugins: dev tools, integrations, MCP servers.",
    },
    Suggested {
        source: "anthropics/claude-code",
        name: "claude-code-plugins",
        description: "Plugins that ship with Claude Code: commit commands, PR review, feature dev.",
    },
];

fn stems(files: &[std::path::PathBuf]) -> Vec<String> {
    files
        .iter()
        .filter_map(|f| f.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .collect()
}

fn overview(state: &AppState) -> Result<PluginsOverview, String> {
    let pm = state.extensions.plugins();
    let installed = pm
        .installed()
        .map_err(|e| format!("{e:#}"))?
        .into_iter()
        .map(|(p, manifest, c)| InstalledView {
            id: p.id(),
            display_name: manifest.as_ref().and_then(|m| m.display_name.clone()),
            description: manifest.as_ref().and_then(|m| m.description.clone()),
            author: manifest
                .as_ref()
                .and_then(|m| m.author.as_ref().map(|a| a.name.clone())),
            name: p.name.clone(),
            marketplace: p.marketplace.clone(),
            version: p.version.clone(),
            enabled: p.enabled,
            path: p.path.display().to_string(),
            updated_at: p.updated_at,
            commands: stems(&c.commands),
            agents: stems(&c.agents),
            skills: c.skills.clone(),
            mcp_servers: c.mcp_server_names.clone(),
            hooks: c.hooks.clone(),
            lsp_servers: c.lsp_servers.clone(),
            problems: c.problems.clone(),
        })
        .collect();
    Ok(PluginsOverview {
        marketplaces: pm.marketplaces().map_err(|e| format!("{e:#}"))?,
        catalog: pm.catalog().map_err(|e| format!("{e:#}"))?,
        installed,
        suggested_marketplaces: SUGGESTED.to_vec(),
    })
}

fn respond(state: &AppState) -> Response {
    match overview(state) {
        Ok(v) => Json(v).into_response(),
        Err(e) => error_json(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

pub async fn get_plugins(State(state): State<AppState>) -> Response {
    respond(&state)
}

pub async fn get_detail(State(state): State<AppState>, AxumPath(id): AxumPath<String>) -> Response {
    match state.extensions.plugins().detail(&id) {
        Ok(d) => Json(d).into_response(),
        Err(e) => error_json(StatusCode::NOT_FOUND, format!("{e:#}")),
    }
}

#[derive(Deserialize)]
pub struct AddMarketplace {
    pub source: String,
}

pub async fn add_marketplace(
    State(state): State<AppState>,
    Json(b): Json<AddMarketplace>,
) -> Response {
    match state.extensions.plugins().add_marketplace(&b.source).await {
        Ok(_) => {
            changed(&state).await;
            respond(&state)
        }
        Err(e) => error_json(StatusCode::BAD_REQUEST, format!("{e:#}")),
    }
}

pub async fn update_marketplace(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> Response {
    match state.extensions.plugins().update_marketplace(&name).await {
        Ok(()) => {
            changed(&state).await;
            respond(&state)
        }
        Err(e) => error_json(StatusCode::BAD_REQUEST, format!("{e:#}")),
    }
}

pub async fn remove_marketplace(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> Response {
    match state.extensions.plugins().remove_marketplace(&name).await {
        Ok(()) => {
            changed(&state).await;
            respond(&state)
        }
        Err(e) => error_json(StatusCode::BAD_REQUEST, format!("{e:#}")),
    }
}

#[derive(Deserialize)]
pub struct Install {
    pub id: String,
}

pub async fn install(State(state): State<AppState>, Json(b): Json<Install>) -> Response {
    match state.extensions.plugins().install(&b.id).await {
        Ok(_) => {
            changed(&state).await;
            respond(&state)
        }
        Err(e) => error_json(StatusCode::BAD_REQUEST, format!("{e:#}")),
    }
}

pub async fn uninstall(State(state): State<AppState>, AxumPath(id): AxumPath<String>) -> Response {
    match state.extensions.plugins().uninstall(&id).await {
        Ok(()) => {
            changed(&state).await;
            respond(&state)
        }
        Err(e) => error_json(StatusCode::BAD_REQUEST, format!("{e:#}")),
    }
}

#[derive(Deserialize)]
pub struct Enabled {
    pub enabled: bool,
}

pub async fn set_enabled(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Json(b): Json<Enabled>,
) -> Response {
    match state.extensions.plugins().set_enabled(&id, b.enabled).await {
        Ok(()) => {
            changed(&state).await;
            respond(&state)
        }
        Err(e) => error_json(StatusCode::BAD_REQUEST, format!("{e:#}")),
    }
}
