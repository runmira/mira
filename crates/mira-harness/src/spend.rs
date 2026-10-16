//! Durable spend accounting shared by native turns, external agents and their children.
//! The lock covers read/modify/rename across processes; prices are booked at usage time.
use chrono::{Datelike, TimeZone, Utc};
use fs2::FileExt;
use mira_config::{MiraConfig, SpendCaps, SpendLimits};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::OpenOptions,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct SpendAccount {
    pub session: String,
    pub cwd: PathBuf,
    dir: PathBuf,
    #[cfg(test)]
    policy_override: Option<SpendLimits>,
}
#[derive(Default, Serialize, Deserialize)]
struct Ledger {
    #[serde(default)]
    unpriced: BTreeMap<String, BTreeSet<String>>,
    #[serde(default)]
    reported: BTreeMap<String, f64>,
    #[serde(default)]
    parents: BTreeMap<String, String>,
    #[serde(default)]
    charges: Vec<Charge>,
    #[serde(default)]
    overrides: BTreeMap<String, f64>,
    #[serde(default)]
    warned: BTreeSet<String>,
}
#[derive(Serialize, Deserialize)]
struct Charge {
    session: String,
    cwd: String,
    ts: i64,
    usd: f64,
}
#[derive(Clone, Debug, Serialize)]
pub struct SpendStatus {
    pub message: Option<String>,
    pub limits: Vec<LimitStatus>,
    pub blocked: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct LimitStatus {
    pub scope: String,
    pub spent: f64,
    pub limit: f64,
    pub blocked: bool,
    pub warning: bool,
}
impl SpendStatus {
    pub fn stop_reason(&self) -> Option<String> {
        if let Some(message) = &self.message {
            return Some(message.clone());
        }
        self.limits.iter().find(|v| v.blocked).map(|v| {
            format!(
                "Spend limit reached: {} (${:.2} / ${:.2}). Work has been saved. {}",
                v.scope,
                v.spent,
                v.limit,
                if v.scope.ends_with("session") {
                    "Raise the limit for this session to continue."
                } else {
                    "Change the limit in Settings or wait for the UTC period to reset."
                }
            )
        })
    }
}
impl SpendAccount {
    pub fn at(
        dir: impl Into<PathBuf>,
        session: impl Into<String>,
        cwd: impl Into<PathBuf>,
    ) -> Self {
        Self {
            dir: dir.into(),
            #[cfg(test)]
            policy_override: None,
            session: session.into(),
            cwd: cwd.into(),
        }
    }
    pub fn user(session: impl Into<String>, cwd: impl Into<PathBuf>) -> Option<Self> {
        Some(Self::at(
            PathBuf::from(std::env::var_os("HOME")?).join(".mira"),
            session,
            cwd,
        ))
    }
    fn policy(&self) -> Result<SpendLimits, String> {
        #[cfg(test)]
        if let Some(policy) = &self.policy_override {
            return Ok(policy.clone());
        }
        Ok(MiraConfig::load_global()
            .map_err(|e| e.to_string())?
            .spend_limits)
    }
    #[cfg(test)]
    pub(crate) fn with_test_policy(mut self, policy: SpendLimits) -> Self {
        self.policy_override = Some(policy);
        self
    }
    fn transaction<T>(
        &self,
        f: impl FnOnce(&mut Ledger) -> Result<T, String>,
    ) -> Result<T, String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.dir.join("spend-limits.lock"))
            .map_err(|e| e.to_string())?;
        FileExt::lock_exclusive(&lock).map_err(|e| e.to_string())?;
        let path = self.dir.join("spend-limits.json");
        let mut ledger: Ledger = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| format!("cannot read spend ledger: {e}"))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ledger::default(),
            Err(e) => return Err(e.to_string()),
        };
        let before = serde_json::to_vec(&ledger).map_err(|e| e.to_string())?;
        let result = f(&mut ledger)?;
        let tmp = self.dir.join("spend-limits.json.tmp");
        let bytes = serde_json::to_vec(&ledger).map_err(|e| e.to_string())?;
        if bytes == before {
            return Ok(result);
        }
        std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
        OpenOptions::new()
            .write(true)
            .open(&tmp)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
        std::fs::rename(tmp, path).map_err(|e| e.to_string())?;
        Ok(result)
    }
    pub fn register_parent(&self, parent: &str) -> Result<(), String> {
        self.transaction(|s| {
            if parent == self.session {
                return Err("session cannot be its own parent".into());
            }
            let mut p = parent;
            let mut seen = BTreeSet::new();
            while let Some(next) = s.parents.get(p) {
                if next == &self.session || !seen.insert(next) {
                    return Err("cyclic spend ownership".into());
                }
                p = next;
            }
            s.parents.insert(self.session.clone(), parent.into());
            Ok(())
        })
    }
    /// External agents report running totals. Deduplication and charging are
    /// committed together, independently of the Usage page's separate ledger.
    pub fn record_total(&self, key: &str, total: f64) -> Result<(), String> {
        if !total.is_finite() || total < 0.0 {
            return Err("invalid reported spend".into());
        }
        self.transaction(|s| {
            let previous = s.reported.get(key).copied().unwrap_or(0.0);
            let delta = if total + 1e-9 < previous {
                total
            } else {
                (total - previous).max(0.0)
            };
            s.reported.insert(key.into(), total);
            if delta > 0.0 {
                s.charges.push(Charge {
                    session: self.session.clone(),
                    cwd: self.cwd.to_string_lossy().into(),
                    ts: Utc::now().timestamp(),
                    usd: delta,
                });
            }
            Ok(())
        })
    }
    pub fn mark_unpriced(&self, model: &str) -> Result<(), String> {
        self.transaction(|s| {
            s.unpriced
                .entry(self.session.clone())
                .or_default()
                .insert(model.into());
            Ok(())
        })
    }
    pub fn record(&self, usd: f64) -> Result<(), String> {
        self.record_at(usd, Utc::now().timestamp())
    }
    fn record_at(&self, usd: f64, ts: i64) -> Result<(), String> {
        if !usd.is_finite() || usd < 0.0 {
            return Err("invalid reported spend".into());
        }
        if usd == 0.0 {
            return Ok(());
        }
        self.transaction(|s| {
            s.charges.push(Charge {
                session: self.session.clone(),
                cwd: self.cwd.to_string_lossy().into(),
                ts,
                usd,
            });
            Ok(())
        })
    }
    pub fn status(&self) -> Result<SpendStatus, String> {
        self.status_with(&self.policy()?, Utc::now().timestamp())
    }
    pub fn enabled(&self) -> Result<bool, String> {
        let policy = self.policy()?;
        Ok(!policy.caps.is_empty()
            || policy
                .projects
                .contains_key(self.cwd.to_string_lossy().as_ref()))
    }
    pub fn ensure_allowed(&self) -> Result<(), String> {
        match self.status()?.stop_reason() {
            Some(reason) => Err(reason),
            None => Ok(()),
        }
    }
    pub fn take_warnings(&self) -> Result<Vec<String>, String> {
        let policy = self.policy()?;
        let now = Utc::now().timestamp();
        self.transaction(|s| {
            let root = root(s, &self.session);
            let status = evaluate(s, &policy, &root, &self.cwd, now);
            let day = Utc
                .timestamp_opt(now, 0)
                .unwrap()
                .format("%Y-%m-%d")
                .to_string();
            Ok(status
                .limits
                .iter()
                .filter(|v| v.warning && !v.blocked)
                .filter_map(|v| {
                    let period = if v.scope.ends_with("month") {
                        &day[..7]
                    } else if v.scope.ends_with("day") {
                        &day
                    } else {
                        "lifetime"
                    };
                    let key = format!(
                        "{}:{}:{}:{}:{}",
                        root,
                        self.cwd.display(),
                        v.scope,
                        period,
                        v.limit
                    );
                    s.warned.insert(key).then(|| {
                        format!(
                            "Spend warning: {} has used ${:.2} of ${:.2} (80% reached).",
                            v.scope, v.spent, v.limit
                        )
                    })
                })
                .collect())
        })
    }
    pub fn raise_session_limit(&self) -> Result<f64, String> {
        let policy = self.policy()?;
        self.transaction(|s| {
            let root = root(s, &self.session);
            let status = evaluate(s, &policy, &root, &self.cwd, Utc::now().timestamp());
            let session: Vec<_> = status
                .limits
                .iter()
                .filter(|v| v.scope.ends_with("session"))
                .collect();
            if session.is_empty() {
                return Err("this session has no spend cap".into());
            }
            let base = session
                .iter()
                .map(|v| v.spent.max(v.limit))
                .fold(0.0, f64::max);
            let next = ((base * 1.25).max(base + 1.0) * 100.0).ceil() / 100.0;
            s.overrides.insert(root, next);
            Ok(next)
        })
    }
    fn status_with(&self, policy: &SpendLimits, now: i64) -> Result<SpendStatus, String> {
        self.transaction(|s| Ok(evaluate(s, policy, &root(s, &self.session), &self.cwd, now)))
    }
}
fn root(s: &Ledger, id: &str) -> String {
    let mut id = id;
    let mut seen = BTreeSet::new();
    while let Some(parent) = s.parents.get(id) {
        if !seen.insert(id) {
            break;
        }
        id = parent;
    }
    id.into()
}
fn evaluate(s: &Ledger, policy: &SpendLimits, session: &str, cwd: &Path, now: i64) -> SpendStatus {
    let date = Utc.timestamp_opt(now, 0).unwrap();
    let day = date
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp();
    let month = date
        .with_day(1)
        .unwrap()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp();
    let mut limits = Vec::new();
    let mut add = |caps: &SpendCaps, project: bool| {
        for (period, cap) in [
            ("session", caps.per_session),
            ("day", caps.per_day),
            ("month", caps.per_month),
        ] {
            let Some(mut cap) = cap else {
                continue;
            };
            if period == "session" {
                cap = s.overrides.get(session).copied().unwrap_or(cap).max(cap);
            }
            let spent: f64 = s
                .charges
                .iter()
                .filter(|c| {
                    (!project || Path::new(&c.cwd) == cwd)
                        && c.ts <= now
                        && match period {
                            "session" => root(s, &c.session) == session,
                            "day" => c.ts >= day,
                            _ => c.ts >= month,
                        }
                })
                .map(|c| c.usd)
                .sum();
            limits.push(LimitStatus {
                scope: format!("{}{period}", if project { "project " } else { "" }),
                spent,
                limit: cap,
                blocked: spent >= cap,
                warning: spent >= cap * 0.8,
            });
        }
    };
    add(&policy.caps, false);
    if let Some(caps) = policy.projects.get(cwd.to_string_lossy().as_ref()) {
        add(caps, true);
    }
    let unpriced: BTreeSet<_> = s
        .unpriced
        .iter()
        .filter(|(id, _)| root(s, id) == session)
        .flat_map(|(_, models)| models.iter())
        .collect();
    let message = (!limits.is_empty() && !unpriced.is_empty()).then(|| format!("Spend protection stopped this session because usage for {} has no known price. Work has been saved. Disable spend caps in Settings to continue with unpriced usage.", unpriced.into_iter().cloned().collect::<Vec<_>>().join(", ")));
    SpendStatus {
        blocked: message.is_some() || limits.iter().any(|v| v.blocked),
        message,
        limits,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn caps() -> SpendLimits {
        SpendLimits {
            caps: SpendCaps {
                per_session: Some(10.0),
                per_day: Some(20.0),
                per_month: Some(40.0),
            },
            ..Default::default()
        }
    }
    #[test]
    fn children_restart_thresholds_and_utc_periods() {
        let dir = tempfile::tempdir().unwrap();
        let a = SpendAccount::at(dir.path(), "parent", "/a");
        let b = SpendAccount::at(dir.path(), "child", "/a");
        let c = SpendAccount::at(dir.path(), "grandchild", "/a");
        b.register_parent("parent").unwrap();
        c.register_parent("child").unwrap();
        let now = Utc
            .with_ymd_and_hms(2026, 10, 16, 12, 0, 0)
            .unwrap()
            .timestamp();
        a.record_at(3.0, now - 86400).unwrap();
        b.record_at(5.0, now).unwrap();
        let status = a.status_with(&caps(), now).unwrap();
        assert!(status.limits[0].warning);
        assert!(!status.blocked);
        assert_eq!(status.limits[1].spent, 5.0);
        assert_eq!(status.limits[2].spent, 8.0);
        c.record_at(2.0, now).unwrap();
        let resumed = SpendAccount::at(dir.path(), "parent", "/a");
        assert!(resumed.status_with(&caps(), now).unwrap().blocked);
        assert!(c.status_with(&caps(), now).unwrap().blocked);
        assert!(a.register_parent("grandchild").is_err());
        let next = Utc
            .with_ymd_and_hms(2026, 11, 1, 0, 0, 0)
            .unwrap()
            .timestamp();
        let status = resumed.status_with(&caps(), next).unwrap();
        assert_eq!(status.limits[1].spent, 0.0);
        assert_eq!(status.limits[2].spent, 0.0);
        assert_eq!(status.limits[0].spent, 10.0);
    }
    #[test]
    fn unpriced_external_usage_stays_blocked_after_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let account = SpendAccount::at(dir.path(), "s", "/a").with_test_policy(caps());
        account.mark_unpriced("unknown-model").unwrap();
        let reopened = SpendAccount::at(dir.path(), "s", "/a").with_test_policy(caps());
        assert!(reopened.status().unwrap().blocked);
        assert!(reopened
            .ensure_allowed()
            .unwrap_err()
            .contains("unknown-model"));
        let no_caps =
            SpendAccount::at(dir.path(), "s", "/a").with_test_policy(SpendLimits::default());
        assert!(!no_caps.status().unwrap().blocked);
    }
    #[test]
    fn external_running_totals_book_once_across_restarts_and_resets() {
        let dir = tempfile::tempdir().unwrap();
        let a = SpendAccount::at(dir.path(), "parent", "/a");
        a.record_total("claude:sid:model", 3.0).unwrap();
        a.record_total("claude:sid:model", 3.0).unwrap();
        let reopened = SpendAccount::at(dir.path(), "parent", "/a");
        reopened.record_total("claude:sid:model", 5.0).unwrap();
        reopened.record_total("claude:sid:model", 1.0).unwrap();
        assert_eq!(
            reopened
                .status_with(&caps(), Utc::now().timestamp())
                .unwrap()
                .limits[0]
                .spent,
            6.0
        );
    }
    #[test]
    fn warnings_are_deduplicated_and_raising_cannot_bypass_daily_caps() {
        let dir = tempfile::tempdir().unwrap();
        let policy = SpendLimits {
            caps: SpendCaps {
                per_session: Some(10.0),
                per_day: Some(10.0),
                ..Default::default()
            },
            ..Default::default()
        };
        let account = SpendAccount::at(dir.path(), "s", "/a").with_test_policy(policy.clone());
        account.record(8.0).unwrap();
        assert_eq!(account.take_warnings().unwrap().len(), 2);
        assert!(account.take_warnings().unwrap().is_empty());
        let reopened = SpendAccount::at(dir.path(), "s", "/a").with_test_policy(policy);
        assert!(reopened.take_warnings().unwrap().is_empty());
        account.record(2.0).unwrap();
        assert!(account.status().unwrap().blocked);
        assert_eq!(account.raise_session_limit().unwrap(), 12.5);
        let status = reopened.status().unwrap();
        assert!(!status.limits[0].blocked);
        assert!(
            status.limits[1].blocked,
            "daily cap still holds after a session raise"
        );
        assert!(account.record(f64::NAN).is_err());
    }
    #[test]
    fn project_limits_and_concurrent_process_style_writers() {
        let dir = tempfile::tempdir().unwrap();
        let mut policy = caps();
        policy.caps.per_session = None;
        policy.projects.insert(
            "/a".into(),
            SpendCaps {
                per_day: Some(3.0),
                ..Default::default()
            },
        );
        let mut threads = vec![];
        for i in 0..8 {
            let a = SpendAccount::at(dir.path(), format!("s{i}"), if i < 4 { "/a" } else { "/b" });
            threads.push(std::thread::spawn(move || a.record(1.0).unwrap()));
        }
        for t in threads {
            t.join().unwrap();
        }
        let a = SpendAccount::at(dir.path(), "s0", "/a");
        let status = a.status_with(&policy, Utc::now().timestamp()).unwrap();
        assert!(status.blocked);
        assert_eq!(status.limits[0].spent, 8.0);
        assert_eq!(status.limits[2].spent, 4.0);
        std::fs::write(dir.path().join("spend-limits.json"), b"broken").unwrap();
        assert!(a.status_with(&policy, Utc::now().timestamp()).is_err());
    }
}

/// Wrap the session's provider so summaries, evaluators and background extraction
/// use the same account as conversational requests.
pub struct MeteredProvider {
    pub inner: std::sync::Arc<dyn mira_ai::ChatProvider>,
    pub account: SpendAccount,
}
#[async_trait::async_trait]
impl mira_ai::ChatProvider for MeteredProvider {
    async fn stream(
        &self,
        request: mira_ai::ChatRequest,
    ) -> Result<
        futures::stream::BoxStream<'static, Result<mira_ai::ChatEvent, mira_ai::ProviderError>>,
        mira_ai::ProviderError,
    > {
        use futures::StreamExt;
        let fail = mira_ai::ProviderError::Config;
        self.account.ensure_allowed().map_err(fail)?;
        let model = request.model.clone();
        if self.account.enabled().map_err(fail)?
            && mira_ai::cost_usd(&model, Default::default()).is_none()
        {
            return Err(fail(format!("Spend limit cannot be enforced: no price is available for {model}. Choose a priced model or change the limit in Settings.")));
        }
        let stream = self.inner.stream(request).await?;
        let account = self.account.clone();
        Ok(stream
            .map(move |event| {
                if let Ok(mira_ai::ChatEvent::Usage(usage)) = &event {
                    if let Some(usd) = mira_ai::cost_usd(&model, *usage) {
                        account.record(usd).map_err(|e| {
                            fail(format!(
                                "Cannot save spend: {e}. Stopped to protect your limits."
                            ))
                        })?;
                    }
                }
                event
            })
            .boxed())
    }
    fn accepts_context(&self, context: &mira_core::message::ProviderContext) -> bool {
        self.inner.accepts_context(context)
    }
    async fn compact(
        &self,
        request: mira_ai::ChatRequest,
    ) -> Result<Option<mira_core::Message>, mira_ai::ProviderError> {
        // Native compaction has no usage stream: use the metered summarizer when caps apply.
        if self
            .account
            .enabled()
            .map_err(mira_ai::ProviderError::Config)?
        {
            return Ok(None);
        }
        self.inner.compact(request).await
    }
    async fn list_models(&self) -> Result<Vec<mira_ai::ModelInfo>, mira_ai::ProviderError> {
        self.inner.list_models().await
    }
}
