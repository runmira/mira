//! `GET /api/usage?days=N` — token usage across every stored session,
//! for the web UI's Usage page.
//!
//! Returns flat rows aggregated per (session, day, model); the client
//! prices and groups them (by day, model, project or session) with the
//! same pricing table it uses everywhere else. Days are UTC dates.
//!
//! Turns record their own usage and model. Older sessions (and whatever
//! a session used before per-turn tracking existed) only have a session
//! total; that remainder is attributed to the session's last-update day
//! and its configured model, so totals always match the sessions.

use std::collections::BTreeMap;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use mira_harness::{SessionRecord, UsageTotals};
use serde::{Deserialize, Serialize};

use crate::state::AppState;

#[derive(Deserialize)]
pub struct UsageQuery {
    #[serde(default = "default_days")]
    days: u32,
}

fn default_days() -> u32 {
    30
}

#[derive(Serialize, Debug, PartialEq)]
pub struct UsageRow {
    pub session_id: String,
    pub title: Option<String>,
    pub cwd: String,
    pub model: String,
    /// UTC date, `YYYY-MM-DD`.
    pub day: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cached_input_tokens: u64,
    /// The external agent that ran these turns (`claude`, `codex`); absent
    /// for Mira's own turns.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// The agent's own cost estimate, when it reports one. The client
    /// prices everything else from its table.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

fn day_of(secs: u64) -> String {
    chrono::DateTime::from_timestamp(secs as i64, 0)
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

fn add(into: &mut UsageTotals, u: &UsageTotals) {
    into.prompt_tokens += u.prompt_tokens;
    into.completion_tokens += u.completion_tokens;
    into.cached_input_tokens += u.cached_input_tokens;
}

/// Usage rows for one session, from `since_day` (inclusive) on.
pub fn rows_for(rec: &SessionRecord, since_day: &str) -> Vec<UsageRow> {
    let mut by_key: BTreeMap<(String, String), UsageTotals> = BTreeMap::new();
    let mut tracked = UsageTotals::default();
    for t in rec.turns.iter().filter(|t| !t.usage.is_zero()) {
        let model = t.model.clone().unwrap_or_else(|| rec.cfg.model.clone());
        add(
            by_key
                .entry((day_of(t.started_at / 1000), model))
                .or_default(),
            &t.usage,
        );
        add(&mut tracked, &t.usage);
    }
    let rest = UsageTotals {
        prompt_tokens: rec
            .usage
            .prompt_tokens
            .saturating_sub(tracked.prompt_tokens),
        completion_tokens: rec
            .usage
            .completion_tokens
            .saturating_sub(tracked.completion_tokens),
        cached_input_tokens: rec
            .usage
            .cached_input_tokens
            .saturating_sub(tracked.cached_input_tokens),
        rounds: 0,
    };
    if rest.prompt_tokens + rest.completion_tokens > 0 {
        add(
            by_key
                .entry((day_of(rec.updated_at), rec.cfg.model.clone()))
                .or_default(),
            &rest,
        );
    }
    by_key
        .into_iter()
        .filter(|((day, _), _)| day.as_str() >= since_day)
        .map(|((day, model), u)| UsageRow {
            session_id: rec.id.to_string(),
            title: rec.title.clone(),
            cwd: rec.cwd.to_string_lossy().into_owned(),
            model,
            day,
            prompt_tokens: u.prompt_tokens,
            completion_tokens: u.completion_tokens,
            cached_input_tokens: u.cached_input_tokens,
            agent: None,
            cost_usd: None,
        })
        .collect()
}

/// External agents' spend from the ledger, one row per (session, day,
/// agent, model), titled from the session it ran in.
pub fn agent_rows(
    ledger: &[crate::agent_spend::SpendRow],
    records: &[SessionRecord],
    since_day: &str,
) -> Vec<UsageRow> {
    let titles: std::collections::HashMap<String, Option<String>> = records
        .iter()
        .map(|r| (r.id.to_string(), r.title.clone()))
        .collect();
    /// One output row's identity, and what it adds up.
    #[derive(PartialEq, Eq, PartialOrd, Ord)]
    struct Key {
        session_id: String,
        day: String,
        agent: String,
        model: String,
    }
    #[derive(Default)]
    struct Sum {
        cwd: String,
        totals: UsageTotals,
        cost: Option<f64>,
    }

    let mut by_key: BTreeMap<Key, Sum> = BTreeMap::new();
    for r in ledger {
        let day = day_of(r.ts);
        if day.as_str() < since_day {
            continue;
        }
        let key = Key {
            session_id: r.session_id.clone(),
            day,
            agent: r.driver.clone(),
            model: r.model.clone(),
        };
        let sum = by_key.entry(key).or_insert_with(|| Sum {
            cwd: r.cwd.clone(),
            ..Default::default()
        });
        sum.totals.prompt_tokens += r.input_tokens;
        sum.totals.completion_tokens += r.output_tokens;
        sum.totals.cached_input_tokens += r.cached_input_tokens;
        if let Some(c) = r.cost_usd {
            sum.cost = Some(sum.cost.unwrap_or(0.0) + c);
        }
    }
    by_key
        .into_iter()
        .map(|(k, sum)| UsageRow {
            title: titles.get(&k.session_id).cloned().flatten(),
            session_id: k.session_id,
            cwd: sum.cwd,
            model: k.model,
            day: k.day,
            prompt_tokens: sum.totals.prompt_tokens,
            completion_tokens: sum.totals.completion_tokens,
            cached_input_tokens: sum.totals.cached_input_tokens,
            agent: Some(k.agent),
            cost_usd: sum.cost,
        })
        .collect()
}

pub async fn get_usage(State(state): State<AppState>, Query(q): Query<UsageQuery>) -> Response {
    let days = q.days.clamp(1, 366);
    let since = chrono::Utc::now().date_naive() - chrono::Days::new(u64::from(days) - 1);
    let since = since.format("%Y-%m-%d").to_string();
    let Some(store) = state.store.clone() else {
        return Json(serde_json::json!({ "since": since, "rows": [] })).into_response();
    };
    let records = match store.list_all(10_000).await {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("list sessions: {e}") })),
            )
                .into_response()
        }
    };
    let mut rows: Vec<UsageRow> = records.iter().flat_map(|r| rows_for(r, &since)).collect();
    if let Some(ledger) = crate::agent_spend::SpendLedger::user() {
        // A day's margin either side of UTC; `agent_rows` filters by day.
        let since_secs = chrono::NaiveDate::parse_from_str(&since, "%Y-%m-%d")
            .ok()
            .and_then(|d| d.and_hms_opt(0, 0, 0))
            .map(|d| d.and_utc().timestamp().max(0) as u64)
            .unwrap_or(0);
        let spent = tokio::task::spawn_blocking(move || ledger.rows_since(since_secs))
            .await
            .unwrap_or_default();
        rows.extend(agent_rows(&spent, &records, &since));
    }
    Json(serde_json::json!({ "since": since, "rows": rows })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mira_harness::TurnMeta;

    fn totals(p: u64, c: u64) -> UsageTotals {
        UsageTotals {
            prompt_tokens: p,
            completion_tokens: c,
            ..Default::default()
        }
    }

    #[test]
    fn turn_rows_plus_legacy_remainder_match_the_session_total() {
        // 2026-09-25 and 2026-09-26, UTC.
        let (d1, d2) = (1_758_800_000_u64, 1_758_886_400_u64);
        let rec = SessionRecord {
            id: "s1".into(),
            cwd: "/repo".into(),
            cfg: mira_harness::SessionConfig::new("model-b"),
            messages: vec![],
            archived: vec![],
            created_at: d1,
            updated_at: d2,
            title: Some("t".into()),
            turns: vec![
                TurnMeta {
                    started_at: d1 * 1000,
                    usage: totals(100, 10),
                    model: Some("model-a".into()),
                    ..Default::default()
                },
                TurnMeta {
                    started_at: d2 * 1000,
                    usage: totals(50, 5),
                    model: Some("model-a".into()),
                    ..Default::default()
                },
            ],
            // 30/3 of this came before per-turn tracking.
            usage: totals(180, 18),
            parent_id: None,
            tasks: vec![],
            goal: None,
            previews: Default::default(),
            archived_at: None,
            agent: None,
            pinned: false,
        };
        let rows = rows_for(&rec, "2000-01-01");
        let sum = |f: fn(&UsageRow) -> u64| rows.iter().map(f).sum::<u64>();
        assert_eq!(sum(|r| r.prompt_tokens), 180);
        assert_eq!(sum(|r| r.completion_tokens), 18);
        let legacy = rows.iter().find(|r| r.model == "model-b").unwrap();
        assert_eq!(
            (legacy.day.as_str(), legacy.prompt_tokens),
            ("2025-09-26", 30)
        );

        // The day filter drops earlier rows.
        assert!(rows_for(&rec, "2025-09-26")
            .iter()
            .all(|r| r.day == "2025-09-26"));
    }
}
