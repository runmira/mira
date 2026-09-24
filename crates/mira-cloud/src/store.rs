//! Local record of launched cloud tasks (`~/.mira/cloud/tasks.json`),
//! behind `mira cloud list / logs / stop`. The source of truth for a
//! task's outcome is its pull request; this only remembers where to look.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::CloudError;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TaskRecord {
    pub id: String,
    pub prompt: String,
    /// `owner/name`.
    pub repo: String,
    pub api_url: String,
    pub branch: String,
    pub base_branch: String,
    pub environment: String,
    pub sandbox_id: String,
    /// envd token for reading the task log while the sandbox runs.
    #[serde(default)]
    pub envd_access_token: Option<String>,
    /// UNIX seconds.
    pub created_at: u64,
    pub max_runtime_secs: u64,
}

pub struct TaskStore {
    path: PathBuf,
}

impl TaskStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// `~/.mira/cloud/tasks.json`.
    pub fn default_path() -> PathBuf {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default()
            .join(".mira")
            .join("cloud")
            .join("tasks.json")
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<Vec<TaskRecord>, CloudError> {
        match std::fs::read(&self.path) {
            Ok(b) => serde_json::from_slice(&b)
                .map_err(|e| CloudError::Config(format!("{}: {e}", self.path.display()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn add(&self, record: TaskRecord) -> Result<(), CloudError> {
        let mut all = self.load()?;
        all.retain(|r| r.id != record.id);
        all.push(record);
        self.save(&all)
    }

    /// Find by id or unique id prefix.
    pub fn get(&self, id: &str) -> Result<TaskRecord, CloudError> {
        let all = self.load()?;
        let hits: Vec<&TaskRecord> = all.iter().filter(|r| r.id.starts_with(id)).collect();
        match hits.as_slice() {
            [one] => Ok((*one).clone()),
            [] => Err(CloudError::Config(format!(
                "no cloud task `{id}` (see `mira cloud list`)"
            ))),
            _ => Err(CloudError::Config(format!(
                "`{id}` matches several tasks; use more of the id"
            ))),
        }
    }

    fn save(&self, all: &[TaskRecord]) -> Result<(), CloudError> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_vec_pretty(all).map_err(|e| CloudError::Config(e.to_string()))?;
        std::fs::write(&self.path, json)?;
        // Holds envd tokens: keep it private.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(id: &str) -> TaskRecord {
        TaskRecord {
            id: id.into(),
            prompt: "p".into(),
            repo: "o/r".into(),
            api_url: "https://api.github.com".into(),
            branch: "mira/x".into(),
            base_branch: "main".into(),
            environment: "e2b".into(),
            sandbox_id: "sbx".into(),
            envd_access_token: None,
            created_at: 0,
            max_runtime_secs: 3600,
        }
    }

    #[test]
    fn add_get_by_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let s = TaskStore::new(dir.path().join("t.json"));
        s.add(rec("t-abc123")).unwrap();
        s.add(rec("t-abd999")).unwrap();
        assert_eq!(s.get("t-abc").unwrap().id, "t-abc123");
        assert!(s.get("t-ab").is_err());
        assert!(s.get("nope").is_err());
        assert_eq!(s.load().unwrap().len(), 2);
    }
}
