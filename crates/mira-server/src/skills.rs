//! Loaded-skill roster endpoint.
//!
//! `GET /api/skills` → `{ skills: [{name, description, category, tier, ...}] }`
//!
//! Powers the composer palette (`/verify`, `/commit`, `/skill-creator`, …)
//! and the future settings-UI "Skills" panel. Read-only for now — write
//! flows (creating a skill on disk) happen through the `skill-creator`
//! skill itself, not through this endpoint.

use std::sync::Arc;

use axum::extract::State;
use axum::Json;
use mira_skills::{Skill, SkillRegistry};
use serde::Serialize;

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
    User,
    Project,
}

#[derive(Debug, Serialize)]
pub struct SkillsResponse {
    pub skills: Vec<SkillView>,
}

pub async fn list_skills(State(state): State<AppState>) -> Json<SkillsResponse> {
    let reg = state.skills.read().await.clone();
    let user_dir = mira_config::user_skills_dir();
    let project_dir = mira_config::project_skills_dir(&*state.cwd.read().await);

    let skills = reg
        .skills
        .values()
        .map(|s| SkillView {
            name: s.name.clone(),
            description: s.description.clone(),
            category: s.category.clone(),
            icon: s.icon.clone(),
            color: s.color.clone(),
            tier: tier_of(s, &user_dir, &project_dir),
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
    let cwd = state.cwd.read().await.clone();
    let fresh = SkillRegistry::load_layered(
        &mira_config::user_skills_dir(),
        &mira_config::project_skills_dir(&cwd),
    );
    let mut w = state.skills.write().await;
    *w = Arc::new(fresh);
}

pub async fn reload_skills(State(state): State<AppState>) -> Json<SkillsResponse> {
    reload_registry(&state).await;
    // Fall through to the standard list-response so the frontend gets
    // the fresh roster in the same round-trip.
    list_skills(State(state)).await
}

fn tier_of(s: &Skill, user_dir: &std::path::Path, project_dir: &std::path::Path) -> SkillTier {
    let Some(source) = s.source.as_ref() else {
        return SkillTier::Bundled;
    };
    // Match by prefix — `source` points at either `<dir>/SKILL.md`
    // (dir shape) or `<dir>/<name>.md` (flat shape). Either lives
    // under the user or project directory, so a prefix check is
    // enough.
    if source.starts_with(project_dir) {
        SkillTier::Project
    } else if source.starts_with(user_dir) {
        SkillTier::User
    } else {
        // Skill loaded from an unexpected path — treat as user. This
        // shouldn't normally happen; the loader only reads from the
        // two known directories plus the bundled tier.
        SkillTier::User
    }
}
