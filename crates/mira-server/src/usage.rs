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
use mira_acp::status::Billing;
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
    /// The engine instance that ran them (`codex-work`, or the driver
    /// kind for the default instance). Lets the client tell two accounts
    /// apart when they share a driver.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    /// The agent's own cost estimate, when it reports one. The client
    /// prices everything else from its table.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    /// How these turns are billed. An API-equivalent dollar figure on a
    /// subscription row is not money spent, so the client says which it
    /// is rather than summing everything into one total.
    #[serde(default)]
    pub billing: Billing,
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
    into.cache_write_tokens += u.cache_write_tokens;
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
        cache_write_tokens: rec
            .usage
            .cache_write_tokens
            .saturating_sub(tracked.cache_write_tokens),
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
            instance: None,
            cost_usd: None,
            // Mira's own turns are a provider call, so they're metered
            // whenever the provider prices per token. A local endpoint
            // costs nothing, but that's the client's table to know.
            billing: Billing::Api,
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
        instance: String,
        model: String,
    }
    #[derive(Default)]
    struct Sum {
        cwd: String,
        totals: UsageTotals,
        cost: Option<f64>,
        billing: Option<Billing>,
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
            instance: r.instance.clone(),
            model: r.model.clone(),
        };
        let sum = by_key.entry(key).or_insert_with(|| Sum {
            cwd: r.cwd.clone(),
            ..Default::default()
        });
        // `prompt_tokens` includes cached input, as on Mira's own rows; the
        // ledger keeps fresh input separate.
        sum.totals.prompt_tokens += r.input_tokens + r.cached_input_tokens;
        sum.totals.completion_tokens += r.output_tokens;
        sum.totals.cached_input_tokens += r.cached_input_tokens;
        if let Some(c) = r.cost_usd {
            sum.cost = Some(sum.cost.unwrap_or(0.0) + c);
        }
        // Rows from one agent session share a classification. Keep the
        // most specific one seen, so an old unknown row next to a new
        // classified one doesn't hide what we now know.
        sum.billing = match (sum.billing, r.billing) {
            (None, b) => Some(b),
            (Some(Billing::Unknown), b) => Some(b),
            (a, _) => a,
        };
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
            instance: Some(k.instance),
            cost_usd: sum.cost,
            billing: sum.billing.unwrap_or_default(),
        })
        .collect()
}

/// One agent's own accounting, rolled up per (day, model).
#[derive(Serialize, Debug, PartialEq)]
pub struct ExternalUsageRow {
    /// Which agent's store this came from.
    pub agent: String,
    pub model: String,
    /// UTC date, `YYYY-MM-DD`.
    pub day: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cached_input_tokens: u64,
    /// Turn records the roll-up consumed — the honest "how much did we
    /// read" figure behind the totals.
    pub turns: usize,
}

/// Aggregate an agent's own transcript accounting into per-(day, model)
/// rows. Pure, so the aggregation is testable without a home directory.
pub fn external_usage_rows(
    rows: &[mira_acp::history_import::NativeUsageRow],
) -> Vec<ExternalUsageRow> {
    /// (agent, day, model) → the totals, plus how many turns produced them.
    #[derive(Default, PartialEq, Eq, PartialOrd, Ord)]
    struct Key {
        agent: String,
        day: String,
        model: String,
    }
    #[derive(Default)]
    struct Sum {
        prompt: u64,
        completion: u64,
        cached: u64,
        turns: usize,
    }
    let mut by_key: BTreeMap<Key, Sum> = BTreeMap::new();
    for r in rows {
        // An unreadable timestamp leaves the day empty; it can't be placed
        // on a calendar, so it is not reported as if it were a real day.
        if r.day.is_empty() {
            continue;
        }
        let sum = by_key
            .entry(Key {
                agent: r.source.label().to_string(),
                day: r.day.clone(),
                model: r.model.clone(),
            })
            .or_default();
        sum.prompt = sum.prompt.saturating_add(r.input_tokens);
        sum.completion = sum.completion.saturating_add(r.output_tokens);
        sum.cached = sum.cached.saturating_add(r.cached_input_tokens);
        sum.turns += 1;
    }
    by_key
        .into_iter()
        .map(|(k, sum)| ExternalUsageRow {
            agent: k.agent,
            model: k.model,
            day: k.day,
            prompt_tokens: sum.prompt,
            completion_tokens: sum.completion,
            cached_input_tokens: sum.cached,
            turns: sum.turns,
        })
        .collect()
}

/// `GET /api/usage/external` — token usage from the agents' own stores.
///
/// Deliberately its own endpoint rather than more rows on `/api/usage`:
/// these turns belong to no Mira session, so they have no chat to open,
/// no project, and (for Codex) not even a model name. Folding them into
/// the session rows would let them distort per-chat and per-project
/// totals. Read-only and cheap enough to call on demand; the scan is
/// capped per agent.
pub async fn get_external_usage(Query(q): Query<UsageQuery>) -> Json<Vec<ExternalUsageRow>> {
    let days = q.days.clamp(1, 366);
    let since = (chrono::Utc::now().date_naive() - chrono::Days::new(u64::from(days) - 1))
        .format("%Y-%m-%d")
        .to_string();
    let mut all = Vec::new();
    for source in [
        mira_acp::history_import::Source::ClaudeCode,
        mira_acp::history_import::Source::Codex,
    ] {
        let Some(home) = mira_acp::history_import::source_home(source) else {
            continue;
        };
        all.extend(mira_acp::history_import::native_usage_rows(
            source,
            &home,
            since.as_str(),
        ));
    }
    Json(external_usage_rows(&all))
}

pub async fn get_pricing() -> Json<mira_ai::PricingSnapshot> {
    // Refresh on read, not on the request hot path of every chat: the
    // fetch only fires when the cache is stale (once a day) or when this
    // is the first read after a failed warm-up.
    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        mira_ai::ensure_pricing(),
    )
    .await;
    Json(mira_ai::pricing_snapshot())
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
            forked_from: None,
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

    fn nat(
        source: mira_acp::history_import::Source,
        model: &str,
        day: &str,
        input: u64,
        cached: u64,
        out: u64,
    ) -> mira_acp::history_import::NativeUsageRow {
        mira_acp::history_import::NativeUsageRow {
            source,
            model: model.to_string(),
            file_id: "f".to_string(),
            day: day.to_string(),
            input_tokens: input,
            cached_input_tokens: cached,
            output_tokens: out,
        }
    }

    #[test]
    fn external_rows_group_by_agent_day_and_model() {
        use mira_acp::history_import::Source;
        let rows = vec![
            nat(
                Source::ClaudeCode,
                "claude-sonnet-4-6",
                "2026-10-04",
                100,
                60,
                10,
            ),
            nat(
                Source::ClaudeCode,
                "claude-sonnet-4-6",
                "2026-10-04",
                50,
                20,
                5,
            ),
            // Same model, different day: its own row.
            nat(
                Source::ClaudeCode,
                "claude-sonnet-4-6",
                "2026-10-03",
                10,
                0,
                1,
            ),
            // Same day, different agent: its own row.
            nat(Source::Codex, "unknown-codex", "2026-10-04", 7, 3, 2),
        ];
        let out = external_usage_rows(&rows);
        assert_eq!(out.len(), 3, "got {out:?}");
        let claude = out
            .iter()
            .find(|r| r.day == "2026-10-04" && r.agent == "Claude Code")
            .unwrap();
        assert_eq!(claude.prompt_tokens, 150);
        assert_eq!(claude.cached_input_tokens, 80);
        assert_eq!(claude.completion_tokens, 15);
        assert_eq!(claude.turns, 2);
        let codex = out.iter().find(|r| r.agent == "Codex").unwrap();
        assert_eq!((codex.prompt_tokens, codex.turns), (7, 1));
    }

    #[test]
    fn a_subscription_row_is_labelled_so_the_client_can_say_so() {
        use mira_acp::history_import::Source;
        use mira_acp::status::Billing;
        // One agent session's rows all carry its classification, and the
        // row says so even though the dollar figure is API-equivalent.
        let rec = SessionRecord {
            id: "s1".into(),
            cwd: "/p".into(),
            cfg: mira_harness::SessionConfig::new("claude-opus-4-7"),
            messages: vec![],
            archived: vec![],
            created_at: 0,
            updated_at: 0,
            title: Some("t".into()),
            turns: vec![],
            usage: Default::default(),
            parent_id: None,
            tasks: vec![],
            goal: None,
            previews: Default::default(),
            pinned: false,
            archived_at: None,
            agent: None,
            forked_from: None,
        };
        let ledger = vec![crate::agent_spend::SpendRow {
            ts: 1_788_000_000,
            session_id: "s1".into(),
            cwd: "/p".into(),
            driver: "claude-code".into(),
            instance: "claude-code".into(),
            model: "claude-opus-4-7".into(),
            input_tokens: 100,
            output_tokens: 10,
            cached_input_tokens: 50,
            cost_usd: Some(0.25),
            billing: Billing::Subscription,
        }];
        let rows = agent_rows(&ledger, &[rec], "2000-01-01");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].billing, Billing::Subscription);
        assert!(rows[0].cost_usd.is_some(), "the estimate is still reported");
        let _ = Source::ClaudeCode; // keep the import meaningful for readers
    }

    #[test]
    fn a_mixture_keeps_the_most_specific_billing_it_saw() {
        use mira_acp::status::Billing;
        let row = |billing: Billing, cost: f64| crate::agent_spend::SpendRow {
            ts: 1_788_000_000,
            session_id: "s1".into(),
            cwd: "/p".into(),
            driver: "codex".into(),
            instance: "codex".into(),
            model: "gpt-5".into(),
            input_tokens: 10,
            output_tokens: 1,
            cached_input_tokens: 0,
            cost_usd: Some(cost),
            billing,
        };
        // A row written before the field existed (unknown) next to one we
        // classified: the classification wins, so the client can say it.
        let rows = agent_rows(
            &[row(Billing::Unknown, 0.1), row(Billing::Subscription, 0.2)],
            &[],
            "2000-01-01",
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].billing, Billing::Subscription);
    }

    #[test]
    fn external_rows_drop_records_without_a_day() {
        // A turn we can't place on a calendar is not reported as if it
        // were a real day of spending.
        use mira_acp::history_import::Source;
        let rows = vec![nat(Source::ClaudeCode, "claude-sonnet-4-6", "", 900, 0, 9)];
        assert!(external_usage_rows(&rows).is_empty());
    }
}
