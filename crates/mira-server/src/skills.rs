//! Loaded-skill roster endpoint.
//!
//! `GET /api/skills` → `{ skills: [{name, description, category, tier, ...}] }`
//!
//! Powers the composer palette (`/verify`, `/commit`, `/skill-creator`, …)
//! and the future settings-UI "Skills" panel. Read-only for now — write
//! flows (creating a skill on disk) happen through the `skill-creator`
//! skill itself, not through this endpoint.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use mira_skills::Skill;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use tokio::sync::broadcast;

use crate::protocol::ServerMsg;
use crate::state::AppState;

/// One entry in the `/api/skills` response.
#[derive(Debug, Serialize)]
pub struct SkillView {
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Icon name from the skill's frontmatter (kebab-case). Front-end
    /// maps a curated set of names to Phosphor icon components; unknown
    /// values fall through to a default sparkle. `None` = no
    /// preference; the client picks a default per tier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Tailwind-flavored color name (e.g. `emerald`, `blue`). Frontend
    /// maps to a preset badge palette; unknown values fall through to a
    /// stable hash-derived tint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Slash-command alias this skill mounts on. `None` when the skill
    /// opted out with `slash: false`. Browser front-ends can use this
    /// to surface `/review` etc. in their own command palette.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slash: Option<String>,
    /// Which tier this skill came from. `bundled` = shipped in the
    /// binary; `user` = loaded from `~/.mira/skills/`; `project` =
    /// loaded from `<cwd>/.mira/skills/`. Distinguished by inspecting
    /// the source path — the loader stamps `source` on every disk-
    /// backed skill and leaves it `None` for bundled ones.
    pub tier: SkillTier,
    /// Whether the skill lives in a directory-shape folder (with
    /// SKILL.md + attached siblings) or as a flat `<name>.md`. Front-
    /// end shows a paperclip on directory-shape skills that have
    /// attachments.
    pub has_attachments: bool,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillTier {
    Bundled,
    /// `~/.agents/skills/` — the cross-tool convention (installed via
    /// `npx skills add …`, or hand-dropped). Separate from `User` so
    /// the UI can label packaged installs distinctly from the user's
    /// own hand-authored overrides under `~/.mira/skills/`.
    Shared,
    User,
    Project,
}

#[derive(Debug, Serialize)]
pub struct SkillsResponse {
    pub skills: Vec<SkillView>,
}

/// Full skill payload returned by `GET /api/skills/:name`. Superset of
/// [`SkillView`] with the markdown body and attached-file names, so the
/// detail drawer can render the SKILL.md and show what siblings ship
/// alongside it in the directory-shape case.
#[derive(Debug, Serialize)]
pub struct SkillDetail {
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Slash-command alias — see [`SkillView::slash`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slash: Option<String>,
    pub tier: SkillTier,
    /// The SKILL.md body — instructions the model reads when the skill
    /// is invoked. Rendered as markdown in the settings detail drawer.
    pub body: String,
    /// Attachment filenames (relative to the skill's directory). Empty
    /// for bundled builtins and flat-file skills.
    pub attachments: Vec<String>,
    /// Where the skill was loaded from. `None` for bundled builtins;
    /// `Some(path)` for `~/.mira/skills/…` or `<cwd>/.mira/skills/…`.
    /// Shown in the detail drawer so users can find and edit the file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

pub async fn list_skills(State(state): State<AppState>) -> Json<SkillsResponse> {
    let reg = state.skills.read().await.clone();
    let shared_dir = mira_config::shared_skills_dir();
    let user_dir = mira_config::user_skills_dir();
    let cwd = state.current_cwd().await;
    let project_dirs = mira_config::well_known_project_skills_dirs(&cwd);

    let skills = reg
        .skills
        .values()
        .map(|s| SkillView {
            name: s.name.clone(),
            description: s.description.clone(),
            category: s.category.clone(),
            icon: s.icon.clone(),
            color: s.color.clone(),
            slash: s.slash.clone(),
            tier: tier_of(s, &shared_dir, &user_dir, &project_dirs),
            has_attachments: !s.attached_files().is_empty(),
        })
        .collect();

    Json(SkillsResponse { skills })
}

/// Re-read the skill files off disk and swap the in-process registry.
/// The `SkillTool`, the `/api/skills` endpoint, and the composer palette
/// all see the new roster on their next read — no restart required.
///
/// Idempotent + cheap (a few file reads); safe to hit repeatedly. Both
/// the Settings-panel "Reload" button AND every user-turn kickoff run
/// through this, so a skill the model just created via `write_file`
/// becomes invocable on the next user message without a restart.
pub async fn reload_registry(state: &AppState) {
    let cwd = state.current_cwd().await;
    let fresh = crate::extensions::load_skills(Some(&cwd), &state.extensions.plugin_skill_dirs());
    let mut w = state.skills.write().await;
    *w = Arc::new(fresh);
}

pub async fn reload_skills(State(state): State<AppState>) -> Json<SkillsResponse> {
    reload_registry(&state).await;
    // Fall through to the standard list-response so the frontend gets
    // the fresh roster in the same round-trip.
    list_skills(State(state)).await
}

/// `GET /api/skills/:name` — full detail for one skill.
///
/// Returns 404 when the name doesn't resolve against the current
/// registry. The registry is a merged three-tier snapshot; the response
/// reflects the effective skill (project > user > bundled).
pub async fn get_skill(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> Response {
    let reg = state.skills.read().await.clone();
    let Some(s) = reg.get(&name).cloned() else {
        return (StatusCode::NOT_FOUND, format!("no skill named `{name}`")).into_response();
    };
    let shared_dir = mira_config::shared_skills_dir();
    let user_dir = mira_config::user_skills_dir();
    let cwd = state.current_cwd().await;
    let project_dirs = mira_config::well_known_project_skills_dirs(&cwd);
    let tier = tier_of(&s, &shared_dir, &user_dir, &project_dirs);
    let attachments = s
        .attached_files()
        .into_iter()
        .filter_map(|p| p.to_str().map(str::to_owned))
        .collect();
    let detail = SkillDetail {
        name: s.name.clone(),
        description: s.description.clone(),
        category: s.category.clone(),
        icon: s.icon.clone(),
        color: s.color.clone(),
        slash: s.slash.clone(),
        tier,
        body: s.body.clone(),
        attachments,
        source: s.source.as_ref().map(|p| p.display().to_string()),
    };
    Json(detail).into_response()
}

/// Spawn a background task that watches all four skill directories and
/// hot-reloads the registry on any change, then broadcasts a
/// `SkillsReloaded` event so connected clients refetch `/api/skills`
/// without the user hitting the Reload button.
///
/// The watcher is coalesced with a short debounce because editors
/// often emit multiple events per save (write → rename → chmod). One
/// registry rebuild + one broadcast per debounce window is plenty.
///
/// A `cwd` swap is picked up on the next user turn (the same
/// `reload_registry` runs there) — dynamically re-registering
/// watchers on cwd change adds complexity for a rare event.
pub fn spawn_skill_watcher(state: AppState) {
    tokio::spawn(async move {
        if let Err(e) = run_watcher(state).await {
            tracing::warn!(%e, "skill watcher stopped");
        }
    });
}

async fn run_watcher(state: AppState) -> anyhow::Result<()> {
    let cwd = state.current_cwd().await;
    let mut paths: Vec<PathBuf> = vec![
        mira_config::shared_skills_dir(),
        mira_config::user_skills_dir(),
    ];
    paths.extend(mira_config::well_known_project_skills_dirs(&cwd));

    // std channel bridge: notify is sync, tokio broadcast is async.
    let (raw_tx, mut raw_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    let mut watcher: RecommendedWatcher = notify::recommended_watcher(move |res| {
        // Any event → coalesce to a single "dirty" ping. We don't
        // filter on file extension here — a `SKILL.md` write, a
        // directory rename, or a new subdir all mean "re-scan".
        if let Ok(_ev) = res {
            let _ = raw_tx.send(());
        }
    })?;

    for p in &paths {
        // Ensure the directory exists so notify doesn't refuse to watch
        // it. `create_dir_all` is idempotent; ignore errors (permissions
        // etc — the tier just won't be watched and the manual Reload
        // button still works).
        let _ = std::fs::create_dir_all(p);
        if let Err(e) = watcher.watch(p, RecursiveMode::Recursive) {
            tracing::debug!(?p, %e, "skill watcher: skipping path");
        }
    }

    // Debounce + reload loop. `raw_rx.recv().await` blocks until at
    // least one change; the second `while` drains everything that
    // arrived in the debounce window so a burst of editor events
    // collapses into one reload.
    while raw_rx.recv().await.is_some() {
        tokio::time::sleep(Duration::from_millis(250)).await;
        while raw_rx.try_recv().is_ok() {}
        reload_registry(&state).await;
        let count = state.skills.read().await.skills.len();
        tracing::debug!(count, "skill watcher: reloaded");
        broadcast_reloaded_all(&state).await;
    }
    Ok(())
}

fn broadcast_reloaded(tx: &broadcast::Sender<ServerMsg>) {
    // Broadcast fails when there are no active subscribers — that's
    // fine, the browser just picks up the fresh roster on its next
    // fetch.
    let _ = tx.send(ServerMsg::SkillsReloaded);
}

/// Fan out `SkillsReloaded` to every live slot's channel so a background
/// tab watching a different session also refetches.
async fn broadcast_reloaded_all(state: &AppState) {
    for slot in state.list_slots().await {
        broadcast_reloaded(&slot.events_tx);
    }
}

fn tier_of(
    s: &Skill,
    shared_dir: &std::path::Path,
    user_dir: &std::path::Path,
    project_dirs: &[PathBuf],
) -> SkillTier {
    let Some(source) = s.source.as_ref() else {
        return SkillTier::Bundled;
    };
    // Match by prefix. Every well-known project location (`.mira`,
    // `.agents`, `.claude`, `.codex`, `.cursor`) maps to `Project` —
    // the UI groups them together as "this repo's skills" regardless
    // of which convention the SKILL.md happens to live under.
    if project_dirs.iter().any(|p| source.starts_with(p)) {
        SkillTier::Project
    } else if source.starts_with(user_dir) {
        SkillTier::User
    } else if source.starts_with(shared_dir) {
        SkillTier::Shared
    } else {
        // Loaded from an unexpected path — treat as user. Shouldn't
        // normally happen; the loader only reads the known dirs plus
        // the bundled tier.
        SkillTier::User
    }
}
