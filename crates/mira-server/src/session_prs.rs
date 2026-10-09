//! The pull request behind each worktree chat, for the sidebar.
//!
//! One `gh pr list` per repository covers every branch in it, and the answer
//! is kept for a minute. A stale answer is served while a fresh one is
//! fetched in the background, so a sidebar refresh only ever waits on GitHub
//! the first time it sees a repository, and then for a few seconds at most.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionPr {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: PrState,
    /// When it merged or closed, in epoch seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrState {
    Open,
    Draft,
    Merged,
    Closed,
}

/// How long an answer counts as current.
const FRESH: Duration = Duration::from_secs(60);
/// How long a caller waits for a repository's first answer.
const FIRST_WAIT: Duration = Duration::from_secs(3);

type Prs = Arc<HashMap<String, SessionPr>>;

#[derive(Default)]
struct Entry {
    /// When `prs` was fetched; `None` until the first answer lands.
    at: Option<Instant>,
    prs: Prs,
    fetching: bool,
}

fn cache() -> &'static Mutex<HashMap<PathBuf, Entry>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Entry>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// The PR for `branch` in the repository at `repo`, newest first. Blocking:
/// call it off the async executor.
pub fn lookup(repo: &Path, branch: &str) -> Option<SessionPr> {
    let deadline = Instant::now() + FIRST_WAIT;
    loop {
        let answered = {
            let mut entries = cache().lock().unwrap_or_else(|e| e.into_inner());
            let entry = entries.entry(repo.to_path_buf()).or_default();
            let stale = entry.at.is_none_or(|at| at.elapsed() >= FRESH);
            if stale && !entry.fetching {
                entry.fetching = true;
                let repo = repo.to_path_buf();
                std::thread::spawn(move || {
                    let prs = Arc::new(fetch(&repo));
                    let mut entries = cache().lock().unwrap_or_else(|e| e.into_inner());
                    let entry = entries.entry(repo).or_default();
                    *entry = Entry {
                        at: Some(Instant::now()),
                        prs,
                        fetching: false,
                    };
                });
            }
            entry.at.is_some().then(|| entry.prs.clone())
        };
        if let Some(prs) = answered {
            return prs.get(branch).cloned();
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhPr {
    number: u64,
    title: String,
    url: String,
    state: String,
    #[serde(default)]
    is_draft: bool,
    head_ref_name: String,
    #[serde(default)]
    is_cross_repository: bool,
    #[serde(default)]
    merged_at: Option<String>,
    #[serde(default)]
    closed_at: Option<String>,
}

/// Every recent PR in the repository, by branch. Empty when `gh` is missing,
/// signed out or offline: the sidebar just shows no PR.
fn fetch(repo: &Path) -> HashMap<String, SessionPr> {
    let out = std::process::Command::new("gh")
        .current_dir(repo)
        .env("GH_PROMPT_DISABLED", "1")
        .args([
            "pr",
            "list",
            "--state",
            "all",
            "--limit",
            "100",
            "--json",
            "number,title,url,state,isDraft,headRefName,isCrossRepository,mergedAt,closedAt",
        ])
        .output();
    match out {
        Ok(out) if out.status.success() => by_branch(&out.stdout),
        _ => HashMap::new(),
    }
}

fn by_branch(json: &[u8]) -> HashMap<String, SessionPr> {
    let Ok(prs) = serde_json::from_slice::<Vec<GhPr>>(json) else {
        return HashMap::new();
    };
    let mut out = HashMap::new();
    // `gh` lists newest first, so a branch's latest PR wins. A fork's
    // branch of the same name is someone else's work.
    for pr in prs.into_iter().filter(|pr| !pr.is_cross_repository) {
        let state = match pr.state.as_str() {
            "MERGED" => PrState::Merged,
            "CLOSED" => PrState::Closed,
            _ if pr.is_draft => PrState::Draft,
            _ => PrState::Open,
        };
        let closed_at = match state {
            PrState::Merged => pr.merged_at.as_deref(),
            PrState::Closed => pr.closed_at.as_deref(),
            _ => None,
        }
        .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
        .map(|at| at.timestamp().max(0) as u64);
        out.entry(pr.head_ref_name).or_insert(SessionPr {
            number: pr.number,
            title: pr.title,
            url: pr.url,
            state,
            closed_at,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_pr_per_branch_with_its_state_and_close_time() {
        let json = br#"[
            {"number":9,"title":"Again","url":"u9","state":"OPEN","isDraft":true,"headRefName":"feat/a","isCrossRepository":false,"mergedAt":null,"closedAt":null},
            {"number":7,"title":"First try","url":"u7","state":"CLOSED","isDraft":false,"headRefName":"feat/a","isCrossRepository":false,"mergedAt":null,"closedAt":"2026-10-01T10:00:00Z"},
            {"number":5,"title":"Done","url":"u5","state":"MERGED","isDraft":false,"headRefName":"feat/b","isCrossRepository":false,"mergedAt":"2026-10-02T00:00:00Z","closedAt":"2026-10-02T00:00:00Z"},
            {"number":4,"title":"Theirs","url":"u4","state":"OPEN","isDraft":false,"headRefName":"feat/c","isCrossRepository":true}
        ]"#;
        let prs = by_branch(json);
        assert_eq!(prs["feat/a"].number, 9);
        assert_eq!(prs["feat/a"].state, PrState::Draft);
        assert_eq!(prs["feat/a"].closed_at, None);
        assert_eq!(prs["feat/b"].state, PrState::Merged);
        assert_eq!(prs["feat/b"].closed_at, Some(1_790_899_200));
        assert!(!prs.contains_key("feat/c"));
    }

    #[test]
    fn unreadable_output_means_no_prs() {
        assert!(by_branch(b"gh: not logged in").is_empty());
    }
}
