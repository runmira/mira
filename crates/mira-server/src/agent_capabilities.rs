//! What each engine instance's installed agent was last seen to support.
//!
//! A record is written when a runtime starts: the resolved binary, its
//! version, the transport, and the capabilities that runtime negotiated
//! (ACP `initialize`, Codex `initialize`/model catalog, Claude's native
//! interface). Native calls the installed agent answers as unknown methods
//! downgrade the record, so the UI stops offering them.
//!
//! Records are keyed by instance and are only used while the launch they
//! came from is unchanged (same binary, its modification time, home and
//! arguments). An unknown launch keeps the conservative defaults. Starts
//! take an epoch first, and a commit from an older epoch never replaces a
//! newer runtime's record, so a slow probe cannot overwrite its successor.
use mira_acp::runtime::{ForkCapability, RuntimeCapabilities, SteeringCapability};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Mutex;

/// A native operation the installed agent can turn out not to implement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Feature {
    Fork,
    Rollback,
    Steering,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CapabilityRecord {
    pub instance: String,
    pub driver: String,
    /// `native` or `acp`.
    pub transport: String,
    pub binary: String,
    pub version: Option<String>,
    pub launch_fingerprint: u64,
    pub capabilities: RuntimeCapabilities,
    /// Operations the installed agent answered as unsupported.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub unsupported: BTreeSet<Feature>,
    pub observed_at: u64,
    pub epoch: u64,
}

impl CapabilityRecord {
    /// The capabilities after downgrades.
    pub fn effective(&self) -> RuntimeCapabilities {
        downgrade(self.capabilities.clone(), &self.unsupported)
    }
}

pub fn downgrade(
    mut caps: RuntimeCapabilities,
    unsupported: &BTreeSet<Feature>,
) -> RuntimeCapabilities {
    for feature in unsupported {
        match feature {
            Feature::Fork => caps.fork = ForkCapability::Unavailable,
            Feature::Rollback => caps.native_rollback = false,
            Feature::Steering => caps.steering = SteeringCapability::Unavailable,
        }
    }
    caps
}

/// Where a session's capabilities came from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilitySource {
    /// The running agent negotiated them.
    Live,
    /// The last runtime of this unchanged launch negotiated them.
    Recorded,
    /// Nothing has been negotiated for this launch yet.
    #[default]
    Unknown,
}

#[derive(Default, Serialize, Deserialize)]
struct Stored {
    records: HashMap<String, CapabilityRecord>,
}

pub struct CapabilityRegistry {
    path: Option<PathBuf>,
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    records: HashMap<String, CapabilityRecord>,
    next_epoch: u64,
}

impl Default for CapabilityRegistry {
    fn default() -> Self {
        Self::load(
            std::env::var_os("HOME")
                .map(|h| PathBuf::from(h).join(".mira/agent-capabilities.json")),
        )
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Identify a launch: the resolved program (and when it last changed, so a
/// CLI upgrade invalidates the record), its arguments and the environment
/// keys that pick a home or account. Secret values are not hashed.
pub fn launch_fingerprint(launch: &mira_acp::driver::LaunchConfig) -> u64 {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    let program = mira_acp::which::resolve(&launch.program.display().to_string())
        .unwrap_or_else(|| launch.program.clone());
    program.hash(&mut hash);
    if let Ok(modified) = std::fs::metadata(&program).and_then(|m| m.modified()) {
        modified.hash(&mut hash);
    }
    launch.args.hash(&mut hash);
    let mut env: Vec<_> = launch.env.iter().collect();
    env.sort();
    for (key, value) in env {
        key.hash(&mut hash);
        if key.ends_with("_HOME") || key.ends_with("_DIR") || key.ends_with("_CONFIG") {
            value.hash(&mut hash);
        }
    }
    hash.finish()
}

impl CapabilityRegistry {
    pub fn load(path: Option<PathBuf>) -> Self {
        let records = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str::<Stored>(&t).ok())
            .map(|s| s.records)
            .unwrap_or_default();
        let next_epoch = records.values().map(|r| r.epoch).max().unwrap_or(0);
        Self {
            path,
            inner: Mutex::new(Inner {
                records,
                next_epoch,
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn save(&self, inner: &Inner) {
        let Some(path) = &self.path else { return };
        let stored = Stored {
            records: inner.records.clone(),
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string_pretty(&stored) {
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(&tmp, path);
            }
        }
    }

    /// Reserve an epoch for a runtime that is about to start.
    pub fn begin(&self) -> u64 {
        let mut inner = self.lock();
        inner.next_epoch += 1;
        inner.next_epoch
    }

    /// Record what a started runtime negotiated. Returns `false` (and keeps
    /// the existing record) when a newer runtime already committed.
    pub fn commit(&self, mut record: CapabilityRecord) -> bool {
        let mut inner = self.lock();
        if let Some(existing) = inner.records.get(&record.instance) {
            if existing.epoch > record.epoch {
                return false;
            }
            // Downgrades learned from this same launch still hold.
            if existing.launch_fingerprint == record.launch_fingerprint
                && existing.version == record.version
            {
                record
                    .unsupported
                    .extend(existing.unsupported.iter().copied());
            }
        }
        record.observed_at = now_secs();
        inner.records.insert(record.instance.clone(), record);
        self.save(&inner);
        true
    }

    /// The installed agent answered `feature` as an unknown method.
    pub fn mark_unsupported(&self, instance: &str, feature: Feature) {
        let mut inner = self.lock();
        let Some(record) = inner.records.get_mut(instance) else {
            return;
        };
        if record.unsupported.insert(feature) {
            self.save(&inner);
        }
    }

    pub fn unsupported(&self, instance: &str, fingerprint: Option<u64>) -> BTreeSet<Feature> {
        self.lock()
            .records
            .get(instance)
            .filter(|r| fingerprint.is_none_or(|f| f == r.launch_fingerprint))
            .map(|r| r.unsupported.clone())
            .unwrap_or_default()
    }

    /// The version the instance's latest runtime reported.
    pub fn version(&self, instance: &str) -> Option<String> {
        self.lock()
            .records
            .get(instance)
            .and_then(|r| r.version.clone())
    }

    /// The record for this instance, only while its launch is unchanged.
    pub fn recorded(&self, instance: &str, fingerprint: u64) -> Option<CapabilityRecord> {
        self.lock()
            .records
            .get(instance)
            .filter(|r| r.launch_fingerprint == fingerprint)
            .cloned()
    }
}

/// True when an agent's error means "this method does not exist here".
pub fn is_unknown_method(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    error.contains("-32601")
        || lower.contains("method not found")
        || lower.contains("unknown variant")
}

/// The CLI's own `--version`, for agents that report none on the wire.
pub async fn cli_version(program: &std::path::Path) -> Option<String> {
    let mut cmd = tokio::process::Command::new(program);
    cmd.arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let out = tokio::time::timeout(std::time::Duration::from_secs(5), cmd.output())
        .await
        .ok()?
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .next()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| l.chars().take(120).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(epoch: u64, fingerprint: u64, version: &str) -> CapabilityRecord {
        CapabilityRecord {
            instance: "codex".into(),
            driver: "codex".into(),
            transport: "native".into(),
            binary: "/bin/codex".into(),
            version: Some(version.into()),
            launch_fingerprint: fingerprint,
            capabilities: RuntimeCapabilities {
                fork: ForkCapability::Turn,
                native_rollback: true,
                steering: SteeringCapability::Native,
                ..Default::default()
            },
            unsupported: BTreeSet::new(),
            observed_at: 0,
            epoch,
        }
    }

    #[test]
    fn a_stale_runtime_cannot_overwrite_a_newer_owner() {
        let reg = CapabilityRegistry::load(None);
        let old = reg.begin();
        let new = reg.begin();
        assert!(reg.commit(record(new, 1, "0.157")));
        assert!(!reg.commit(record(old, 1, "0.150")), "the older probe lost");
        assert_eq!(
            reg.recorded("codex", 1).unwrap().version.as_deref(),
            Some("0.157")
        );
    }

    #[test]
    fn records_apply_only_to_the_launch_they_came_from() {
        let reg = CapabilityRegistry::load(None);
        reg.commit(record(reg.begin(), 1, "0.157"));
        assert!(reg.recorded("codex", 1).is_some());
        assert!(
            reg.recorded("codex", 2).is_none(),
            "a changed binary is unknown"
        );
        assert!(reg.recorded("codex-work", 1).is_none());
    }

    #[test]
    fn unsupported_answers_downgrade_until_the_launch_changes() {
        let reg = CapabilityRegistry::load(None);
        reg.commit(record(reg.begin(), 1, "0.150"));
        reg.mark_unsupported("codex", Feature::Fork);
        reg.mark_unsupported("codex", Feature::Rollback);
        let caps = reg.recorded("codex", 1).unwrap().effective();
        assert_eq!(caps.fork, ForkCapability::Unavailable);
        assert!(!caps.native_rollback);
        assert_eq!(caps.steering, SteeringCapability::Native);
        // A restart of the same install keeps what it learned.
        reg.commit(record(reg.begin(), 1, "0.150"));
        assert_eq!(
            reg.recorded("codex", 1).unwrap().effective().fork,
            ForkCapability::Unavailable
        );
        // An upgraded CLI is asked again.
        reg.commit(record(reg.begin(), 3, "0.157"));
        assert_eq!(
            reg.recorded("codex", 3).unwrap().effective().fork,
            ForkCapability::Turn
        );
    }

    #[test]
    fn records_survive_a_restart_and_unreadable_files_are_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("caps.json");
        let reg = CapabilityRegistry::load(Some(path.clone()));
        reg.commit(record(reg.begin(), 7, "0.157"));
        let again = CapabilityRegistry::load(Some(path.clone()));
        assert_eq!(
            again.recorded("codex", 7).unwrap().capabilities.fork,
            ForkCapability::Turn
        );
        assert!(again.begin() > 1, "epochs keep increasing across restarts");
        std::fs::write(&path, "{not json").unwrap();
        assert!(CapabilityRegistry::load(Some(path))
            .recorded("codex", 7)
            .is_none());
    }

    #[test]
    fn unknown_method_errors_are_recognized() {
        assert!(is_unknown_method(
            "acp: agent returned an error: -32601 Method not found"
        ));
        assert!(is_unknown_method("unknown variant `thread/fork`"));
        assert!(!is_unknown_method("-32603 Internal error"));
    }
}
