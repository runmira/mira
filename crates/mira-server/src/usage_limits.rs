//! `GET /api/usage/limits` — each agent's usage-limit windows (a five-hour
//! window, a weekly one, …), as the agent last reported them.
//!
//! Agents report their windows mid-turn, to whichever chat is running
//! them, so on their own they're only visible in that chat's composer.
//! The latest report per engine instance is kept here (and on disk, so
//! the Usage page has something to show before an agent next runs) for a
//! server-wide view.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use axum::Json;
use mira_acp::events::LimitWindow;
use serde::{Deserialize, Serialize};

/// One engine instance's windows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderLimits {
    /// Engine instance id (`claude-code`, `codex-work`).
    pub instance: String,
    /// Driver kind (`claude-code`, `codex`), for the provider's mark.
    pub driver: String,
    pub windows: Vec<LimitWindow>,
    /// When the agent reported these, unix seconds.
    pub reported_at: i64,
}

struct Store {
    by_instance: BTreeMap<String, ProviderLimits>,
    /// Where they're kept; unset (tests) means memory only.
    path: Option<PathBuf>,
}

fn store() -> &'static Mutex<Store> {
    static S: OnceLock<Mutex<Store>> = OnceLock::new();
    S.get_or_init(|| {
        Mutex::new(Store {
            by_instance: BTreeMap::new(),
            path: None,
        })
    })
}

/// `~/.mira/usage-limits.json`.
pub fn default_path() -> PathBuf {
    mira_config::state_path().with_file_name("usage-limits.json")
}

/// Keep the windows at `path`, loading what an earlier run saved.
pub fn persist_to(path: PathBuf) {
    let saved: Vec<ProviderLimits> = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let mut s = store().lock().unwrap_or_else(|e| e.into_inner());
    for l in saved {
        s.by_instance.entry(l.instance.clone()).or_insert(l);
    }
    s.path = Some(path);
}

fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

/// An agent reported its windows.
pub fn record(instance: &str, driver: &str, windows: &[LimitWindow]) {
    if windows.is_empty() {
        return;
    }
    let mut s = store().lock().unwrap_or_else(|e| e.into_inner());
    s.by_instance.insert(
        instance.to_string(),
        ProviderLimits {
            instance: instance.to_string(),
            driver: driver.to_string(),
            windows: windows.to_vec(),
            reported_at: now_secs(),
        },
    );
    if let Some(path) = s.path.clone() {
        let all: Vec<&ProviderLimits> = s.by_instance.values().collect();
        let tmp = path.with_extension("json.tmp");
        let ok = std::fs::write(&tmp, serde_json::to_vec(&all).unwrap_or_default())
            .and_then(|_| std::fs::rename(&tmp, &path));
        if let Err(e) = ok {
            tracing::warn!(%e, "usage limits: could not save");
        }
    }
}

/// Every instance's latest windows, by instance id.
pub fn snapshot() -> Vec<ProviderLimits> {
    store()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .by_instance
        .values()
        .cloned()
        .collect()
}

pub async fn get_limits() -> Json<Vec<ProviderLimits>> {
    Json(snapshot())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(name: &str, used: f64) -> LimitWindow {
        LimitWindow {
            name: name.into(),
            utilization: used,
            resets_at: Some(1_900_000_000),
        }
    }

    #[test]
    fn the_latest_report_per_instance_wins_and_empty_reports_are_ignored() {
        record("test-claude", "claude-code", &[window("five_hour", 0.2)]);
        record("test-claude", "claude-code", &[window("five_hour", 0.6), window("seven_day", 0.1)]);
        record("test-claude", "claude-code", &[]);
        let mine: Vec<_> = snapshot().into_iter().filter(|l| l.instance == "test-claude").collect();
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].windows.len(), 2);
        assert_eq!(mine[0].windows[0].utilization, 0.6);
    }
}
