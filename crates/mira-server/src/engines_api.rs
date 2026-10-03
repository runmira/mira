//! `GET /api/engines` — every backend a turn could run on, one shape.
//!
//! This is the endpoint the pickers render: Mira's own providers and
//! the external agents, side by side, each with a health state
//! (`ready` / `not_configured` / `not_found` / `failed` /
//! `unavailable`), a human auth summary, and — when known — its model
//! catalog. Before this endpoint existed the native providers had no
//! health story at all (a missing API key surfaced as a mid-stream
//! error), and external agents only reported health through the
//! settings panel.
//!
//! Cost model: native snapshots are config reads and always inline.
//! External snapshots launch the agent and complete an `initialize`
//! handshake — hundreds of milliseconds to seconds each — so they are
//! cached and refreshed in the background; the first request answers
//! with presence-only rows and the full probe replaces them shortly.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::{Query, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use mira_engine::{EngineSnapshot, EngineState};
use serde::Serialize;
use serde_json::json;

use crate::state::AppState;

/// How long an external probe result stays fresh. Long, because a
/// probe spawns every agent; the state of "is Claude Code installed"
/// rarely changes mid-session.
const EXTERNAL_TTL: Duration = Duration::from_secs(600);

#[derive(Clone)]
struct ExternalCache {
    at: Instant,
    snapshots: Vec<EngineSnapshot>,
}

static EXTERNAL: Mutex<Option<ExternalCache>> = Mutex::new(None);
/// Results of the sweep in progress, by instance, so each agent shows its
/// real state as soon as its own probe ends.
static PARTIAL: Mutex<Option<std::collections::HashMap<String, EngineSnapshot>>> =
    Mutex::new(None);
static REFRESH_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Serialize)]
pub struct EngineListView {
    pub engines: Vec<EngineSnapshot>,
    /// The instance the active selection points at, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_instance: Option<String>,
    /// The active selection's model, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_model: Option<String>,
    /// False only on the very first hit after boot, when external rows
    /// are presence-only placeholders and the full probe is running.
    pub fresh: bool,
}

pub async fn list_engines(
    State(state): State<AppState>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let cfg = match mira_config::MiraConfig::load_global() {
        Ok(c) => c,
        Err(e) => {
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("load config: {e}") })),
            )
                .into_response()
        }
    };
    let engines = state.engines.clone();

    let mut out = Vec::new();

    // Native rows: cheap config reads, always computed now. Catalog
    // models ride the models cache when fresh so the picker can show
    // real lists without a second request.
    // The models cache holds the *active* provider's catalog, so it is only
    // that provider's list. Handing it to every native row made each
    // provider in the picker show the active one's models; the others'
    // catalogs come from `/api/engines/:instance/models` on demand.
    let (active_for_models, _, _) = state.selection.snapshot();
    for inst in engines.instances() {
        if inst.is_native() {
            let is_active = active_for_models.as_deref() == Some(inst.id.as_str());
            let models = if is_active {
                crate::models::cached()
            } else {
                instance_catalog_cached(inst.id.as_str())
            };
            let snapshot = engines
                .snapshot_native(&cfg, inst.id.as_str(), models)
                .expect("is_native checked");
            out.push(snapshot);
        }
    }

    // External rows: serve the cache; when stale (or the caller asked
    // for a refresh), hand back presence-only placeholders now and let
    // the background probe replace them.
    let force = q.get("refresh").is_some_and(|v| v == "1" || v == "true");
    let cached_hit = EXTERNAL
        .lock()
        .expect("engines cache poisoned")
        .as_ref()
        .filter(|hit| hit.at.elapsed() < EXTERNAL_TTL)
        .filter(|_| !force)
        .cloned();
    let (active_instance, active_model, _small) = state.selection.snapshot();
    let fresh = match cached_hit {
        Some(hit) => {
            out.extend(hit.snapshots);
            true
        }
        None => {
            spawn_refresh(state);
            let partial = PARTIAL.lock().ok().and_then(|g| g.clone()).unwrap_or_default();
            out.extend(
                engines
                    .instances()
                    .filter(|i| !i.is_native())
                    .map(|i| {
                        partial
                            .get(i.id.as_str())
                            .cloned()
                            .unwrap_or_else(|| presence_only(i))
                    }),
            );
            false
        }
    };

    Json(EngineListView {
        engines: out,
        active_instance,
        active_model,
        fresh,
    })
    .into_response()
}

/// A first-paint row for an external instance: what we know without
/// spawning anything.
fn presence_only(inst: &mira_engine::EngineInstance) -> EngineSnapshot {
    let known = mira_acp::drivers::by_kind(inst.driver.as_str()).is_some();
    EngineSnapshot {
        instance: inst.id.clone(),
        driver: inst.driver.clone(),
        flavor: mira_engine::EngineFlavor::External,
        display_name: inst
            .display_name
            .clone()
            .unwrap_or_else(|| inst.id.to_string()),
        enabled: inst.enabled,
        state: if known {
            // Honest and useless in the smallest way: the full probe
            // will say more, and `Ready` here would be a guess.
            EngineState::Failed {
                reason: "checking…".into(),
            }
        } else {
            EngineState::Unavailable {
                reason: format!("this build has no driver `{}`", inst.driver),
            }
        },
        models: Vec::new(),
        default_model: inst.model.clone(),
        auth: None,
        install_hint: None,
        launch: None,
    }
}

/// Probe every external instance off the request path and cache the
/// result. One at a time — a fleet of Node-based agents starting
/// concurrently is exactly the jank the probe design avoids.
fn spawn_refresh(state: AppState) {
    if REFRESH_IN_FLIGHT.swap(true, Ordering::SeqCst) {
        return;
    }
    tokio::spawn(async move {
        // Reset the in-flight flag however this ends — including a panic
        // or a cancelled task — or no refresh would ever spawn again and
        // the cache would answer placeholders forever.
        struct ResetOnDrop;
        impl Drop for ResetOnDrop {
            fn drop(&mut self) {
                REFRESH_IN_FLIGHT.store(false, Ordering::SeqCst);
            }
        }
        let _reset = ResetOnDrop;
        if let Ok(mut p) = PARTIAL.lock() {
            *p = Some(Default::default());
        }
        // The whole sweep is bounded: probes have their own budgets, but
        // they run sequentially enough that a few wedged ones must not
        // hold the flag (and the cache) hostage.
        match tokio::time::timeout(
            Duration::from_secs(300),
            state.engines.snapshot_externals_with(|snap| {
                if let Ok(mut p) = PARTIAL.lock() {
                    p.get_or_insert_with(Default::default)
                        .insert(snap.instance.to_string(), snap.clone());
                }
            }),
        )
        .await
        {
            Ok(snapshots) => {
                if let Ok(mut guard) = EXTERNAL.lock() {
                    *guard = Some(ExternalCache {
                        at: Instant::now(),
                        snapshots,
                    });
                }
            }
            Err(_) => {
                // Keep what finished, and say so for the rest — leaving them
                // on "checking…" reads as a hang that never ends.
                tracing::warn!("external engine probe sweep timed out");
                let partial = PARTIAL.lock().ok().and_then(|g| g.clone()).unwrap_or_default();
                let snapshots = state
                    .engines
                    .instances()
                    .filter(|i| !i.is_native())
                    .map(|i| {
                        partial.get(i.id.as_str()).cloned().unwrap_or_else(|| {
                            let mut s = presence_only(i);
                            if matches!(s.state, EngineState::Failed { .. }) {
                                s.state = EngineState::Failed {
                                    reason: "didn't answer in time — try again".into(),
                                };
                            }
                            s
                        })
                    })
                    .collect();
                if let Ok(mut guard) = EXTERNAL.lock() {
                    *guard = Some(ExternalCache {
                        at: Instant::now(),
                        snapshots,
                    });
                }
            }
        }
    });
}

/// Per-instance catalogs for providers other than the active one.
type Catalogs = std::collections::HashMap<String, (Instant, Vec<mira_ai::ModelInfo>)>;
static INSTANCE_CATALOGS: Mutex<Option<Catalogs>> = Mutex::new(None);
const CATALOG_TTL: Duration = Duration::from_secs(600);

fn instance_catalog_cached(instance: &str) -> Option<Vec<mira_ai::ModelInfo>> {
    let guard = INSTANCE_CATALOGS.lock().ok()?;
    let (at, models) = guard.as_ref()?.get(instance)?;
    (at.elapsed() < CATALOG_TTL).then(|| models.clone())
}

/// `GET /api/engines/:instance/models` — one provider's own catalog.
///
/// The picker shows every configured provider's models, not only the
/// active one's, so a switch to Groq lists Groq's models. Fetched with that
/// provider's own credentials and cached per instance; an unconfigured or
/// unreachable provider answers with the reason instead of a list.
pub async fn instance_models(
    State(state): State<AppState>,
    axum::extract::Path(instance): axum::extract::Path<String>,
) -> Response {
    let (active, _, _) = state.selection.snapshot();
    if active.as_deref() == Some(instance.as_str()) {
        if let Some(models) = crate::models::cached() {
            return Json(json!({ "models": models })).into_response();
        }
    }
    if let Some(models) = instance_catalog_cached(&instance) {
        return Json(json!({ "models": models })).into_response();
    }
    let cfg = mira_config::MiraConfig::load_global().unwrap_or_default();
    let provider = match mira_engine::native::build_native_provider(&cfg, &instance) {
        Ok(p) => p,
        Err(e) => {
            return (
                axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "error": format!("{e:?}") })),
            )
                .into_response()
        }
    };
    match provider.list_models().await {
        Ok(models) => {
            if let Ok(mut guard) = INSTANCE_CATALOGS.lock() {
                guard
                    .get_or_insert_with(Default::default)
                    .insert(instance, (Instant::now(), models.clone()));
            }
            Json(json!({ "models": models })).into_response()
        }
        Err(e) => (
            axum::http::StatusCode::BAD_GATEWAY,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}
