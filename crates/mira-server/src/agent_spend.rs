//! What external agents spent, for the Usage page.
//!
//! Mira's own turns record their usage on the session; an external agent's
//! turns run in its own process and only *report* spend — as running totals
//! for the agent session (Claude Code's `total_cost_usd` / `modelUsage`,
//! Codex's `tokenUsage.total`). This turns those totals into per-turn
//! amounts and appends them to `~/.mira/agent-spend.jsonl`, which
//! `/api/usage` reads alongside the sessions.
//!
//! The last total seen for each agent session is kept in
//! `agent-spend-state.json`, so a session resumed after a restart (whose
//! totals include what was spent before) isn't counted twice. A total that
//! goes *down* means the agent started counting afresh (an older CLI that
//! doesn't restore totals on resume, or a reset); that total is all new.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use mira_acp::events::ModelSpend;
use serde::{Deserialize, Serialize};

const LEDGER: &str = "agent-spend.jsonl";
const STATE: &str = "agent-spend-state.json";

/// One appended row: what one agent turn spent on one model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpendRow {
    /// Unix seconds.
    pub ts: u64,
    /// The Mira session the agent was driving.
    pub session_id: String,
    pub cwd: String,
    /// Agent kind (`claude`, `codex`).
    pub driver: String,
    /// Engine instance that spent it. Defaults on read for rows written
    /// before the field existed (then it was always the driver kind).
    #[serde(default)]
    pub instance: String,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    /// Whether this was metered or covered by a plan. Rows written before
    /// the field existed read back as unknown, which is the honest reading
    /// for a ledger that never recorded it.
    #[serde(default)]
    pub billing: mira_acp::status::Billing,
}

/// Serialises ledger writes across every session in the process.
static WRITE: Mutex<()> = Mutex::new(());

/// Where the ledger lives: `~/.mira`, or a test directory.
#[derive(Clone, Debug)]
pub struct SpendLedger {
    dir: PathBuf,
}

impl SpendLedger {
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        SpendLedger { dir: dir.into() }
    }

    /// `~/.mira`, when there is a home directory.
    pub fn user() -> Option<Self> {
        let home = std::env::var_os("HOME")?;
        Some(Self::at(PathBuf::from(home).join(".mira")))
    }

    /// Record a running-total report from an agent session. Returns the rows
    /// appended (empty when nothing new was spent).
    pub fn record(
        &self,
        who: &Spender,
        agent_session: Option<&str>,
        totals: &[ModelSpend],
    ) -> Vec<SpendRow> {
        let _guard = WRITE.lock().unwrap_or_else(|p| p.into_inner());
        // Keyed by instance, not kind: two instances may drive agent
        // sessions with colliding ids (shared home, different accounts
        // over time), and kind-keying would fold one's spend into the
        // other's running total.
        let key = format!(
            "{}:{}",
            who.instance,
            agent_session
                .filter(|s| !s.is_empty())
                .unwrap_or(&who.session_id)
        );
        let mut state = read_state(&self.dir.join(STATE));
        let seen = state.entry(key).or_default();
        let ts = now_secs();
        let mut rows = Vec::new();
        for t in totals {
            let model = if t.model.is_empty() {
                who.fallback_model.clone()
            } else {
                t.model.clone()
            };
            let prev = seen.get(&model).cloned().unwrap_or_default();
            let d = delta(&prev, t);
            seen.insert(model.clone(), t.clone());
            if d.input_tokens + d.output_tokens + d.cached_input_tokens == 0
                && d.cost_usd.unwrap_or(0.0) <= 0.0
            {
                continue;
            }
            rows.push(SpendRow {
                ts,
                session_id: who.session_id.clone(),
                cwd: who.cwd.clone(),
                driver: who.driver.clone(),
                instance: who.instance.clone(),
                model,
                input_tokens: d.input_tokens,
                output_tokens: d.output_tokens,
                cached_input_tokens: d.cached_input_tokens,
                cost_usd: d.cost_usd,
                billing: who.billing,
            });
        }
        if let Err(e) = self.persist(&state, &rows) {
            tracing::warn!(%e, "agent spend: could not update the ledger");
        }
        rows
    }

    fn persist(&self, state: &State, rows: &[SpendRow]) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        if !rows.is_empty() {
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.dir.join(LEDGER))?;
            for r in rows {
                writeln!(f, "{}", serde_json::to_string(r).unwrap_or_default())?;
            }
        }
        let tmp = self.dir.join(format!("{STATE}.tmp"));
        std::fs::write(&tmp, serde_json::to_vec(state).unwrap_or_default())?;
        std::fs::rename(tmp, self.dir.join(STATE))
    }

    /// Every row from `since` (unix seconds) on. A torn last line from a
    /// crash mid-write is skipped, not fatal. Rows written before the
    /// instance field existed come back routed at their driver kind, which
    /// is what their default instance id was.
    pub fn rows_since(&self, since: u64) -> Vec<SpendRow> {
        let Ok(text) = std::fs::read_to_string(self.dir.join(LEDGER)) else {
            return Vec::new();
        };
        text.lines()
            .filter_map(|l| serde_json::from_str::<SpendRow>(l).ok())
            .map(|mut r| {
                if r.instance.is_empty() {
                    r.instance = r.driver.clone();
                }
                r
            })
            .filter(|r| r.ts >= since)
            .collect()
    }
}

/// Who is spending: the session and agent a report belongs to.
#[derive(Clone, Debug)]
pub struct Spender {
    pub session_id: String,
    pub cwd: String,
    pub driver: String,
    /// Engine instance being driven. The dedupe key and the ledger row
    /// route by this: kind-routing would merge two accounts' spend.
    pub instance: String,
    /// Model to file a report under when the agent doesn't name one.
    pub fallback_model: String,
    /// How this agent's turns are billed, decided once at start from the
    /// instance's key and the auth methods the agent advertised.
    pub billing: mira_acp::status::Billing,
}

/// Agent-session key → model → last running total seen.
type State = HashMap<String, HashMap<String, ModelSpend>>;

fn read_state(path: &Path) -> State {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// What was spent between two running totals.
fn delta(prev: &ModelSpend, now: &ModelSpend) -> ModelSpend {
    let restarted = now.input_tokens < prev.input_tokens
        || now.output_tokens < prev.output_tokens
        || now.cached_input_tokens < prev.cached_input_tokens
        || now.cost_usd.unwrap_or(0.0) + 1e-9 < prev.cost_usd.unwrap_or(0.0);
    if restarted {
        return now.clone();
    }
    ModelSpend {
        model: now.model.clone(),
        input_tokens: now.input_tokens - prev.input_tokens,
        output_tokens: now.output_tokens - prev.output_tokens,
        cached_input_tokens: now.cached_input_tokens - prev.cached_input_tokens,
        cost_usd: now
            .cost_usd
            .map(|c| (c - prev.cost_usd.unwrap_or(0.0)).max(0.0)),
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spend(model: &str, i: u64, o: u64, cost: f64) -> ModelSpend {
        ModelSpend {
            model: model.into(),
            input_tokens: i,
            output_tokens: o,
            cached_input_tokens: 0,
            cost_usd: Some(cost),
        }
    }

    fn who() -> Spender {
        Spender {
            session_id: "mira-1".into(),
            cwd: "/p".into(),
            driver: "claude".into(),
            instance: "claude-code".into(),
            fallback_model: "claude".into(),
            billing: mira_acp::status::Billing::Unknown,
        }
    }

    #[test]
    fn running_totals_become_per_turn_rows() {
        let dir = tempfile::tempdir().unwrap();
        let l = SpendLedger::at(dir.path());
        let a = l.record(&who(), Some("cc-1"), &[spend("opus", 100, 10, 0.5)]);
        assert_eq!((a[0].input_tokens, a[0].output_tokens), (100, 10));
        let b = l.record(&who(), Some("cc-1"), &[spend("opus", 250, 30, 1.25)]);
        assert_eq!((b[0].input_tokens, b[0].output_tokens), (150, 20));
        assert!((b[0].cost_usd.unwrap() - 0.75).abs() < 1e-9);
        // Same totals again (a resumed session after a restart): nothing new.
        let c = SpendLedger::at(dir.path()).record(
            &who(),
            Some("cc-1"),
            &[spend("opus", 250, 30, 1.25)],
        );
        assert!(c.is_empty());
        assert_eq!(l.rows_since(0).len(), 2);
    }

    #[test]
    fn a_total_that_drops_is_a_fresh_count() {
        let dir = tempfile::tempdir().unwrap();
        let l = SpendLedger::at(dir.path());
        l.record(&who(), Some("cc-1"), &[spend("opus", 500, 50, 2.0)]);
        let r = l.record(&who(), Some("cc-1"), &[spend("opus", 40, 4, 0.1)]);
        assert_eq!((r[0].input_tokens, r[0].output_tokens), (40, 4));
    }

    /// The audit T3-style attribution needs: a parent turn and the child
    /// it delegated to are separate agent sessions, so their spend is
    /// booked separately — and neither is double counted, because the
    /// dedupe key is the agent's own session, not the chat.
    #[test]
    fn a_parent_and_its_delegated_child_book_separately() {
        use super::Spender;
        use mira_acp::status::Billing::Subscription;
        let dir = tempfile::tempdir().unwrap();
        let l = SpendLedger::at(dir.path());
        let parent = Spender {
            session_id: "chat".into(),
            cwd: "/p".into(),
            driver: "claude-code".into(),
            instance: "claude-code".into(),
            fallback_model: "opus".into(),
            billing: mira_acp::status::Billing::Subscription,
        };
        let mut child = parent.clone();
        child.session_id = "child-1".into();
        // The parent reports its own turn, the child its own, each with
        // its own agent session id.
        let a = l.record(&parent, Some("parent-sid"), &[spend("opus", 100, 10, 0.5)]);
        let b = l.record(&child, Some("child-sid"), &[spend("opus", 40, 4, 0.2)]);
        assert_eq!(a[0].input_tokens, 100);
        assert_eq!(b[0].input_tokens, 40);
        assert_eq!(a[0].session_id, "chat");
        assert_eq!(b[0].session_id, "child-1");
        // Both rows say the same thing about how they're billed.
        assert_eq!(a[0].billing, Subscription);
        assert_eq!(b[0].billing, Subscription);
        // Re-reporting the parent's total adds nothing, even though the
        // child has since reported too.
        let again = l.record(&parent, Some("parent-sid"), &[spend("opus", 140, 14, 0.7)]);
        assert_eq!(again[0].input_tokens, 40, "only the parent's increment");
        assert_eq!(again[0].session_id, "chat", "the child is not folded in");
        assert_eq!(l.rows_since(0).len(), 3);
    }

    /// Two chats resuming the *same* agent session (one via history
    /// import, one opened by hand) report overlapping running totals.
    /// Keyed by the agent's own session, the overlap is booked once.
    #[test]
    fn two_chats_on_one_agent_session_count_the_overlap_once() {
        let dir = tempfile::tempdir().unwrap();
        let l = SpendLedger::at(dir.path());
        let mut other = who();
        other.session_id = "other-chat".into();
        let a = l.record(&who(), Some("shared-sid"), &[spend("opus", 100, 10, 0.5)]);
        assert_eq!(a[0].input_tokens, 100);
        // The second chat sees the same total: no new spend, and no row
        // attributed to it.
        let b = l.record(&other, Some("shared-sid"), &[spend("opus", 100, 10, 0.5)]);
        assert!(b.is_empty(), "overlapping totals must not double count");
    }

    #[test]
    fn unnamed_models_are_filed_under_the_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let l = SpendLedger::at(dir.path());
        let r = l.record(
            &who(),
            None,
            &[ModelSpend {
                input_tokens: 7,
                ..Default::default()
            }],
        );
        assert_eq!(r[0].model, "claude");
    }
}
