//! What a cloud task is: the brief the launcher hands to the worker.
//!
//! The spec is written into the sandbox as JSON. Secrets are **not** part
//! of it: they travel in a separate file ([`Secrets`]) that the worker
//! reads and deletes before the agent starts.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::CloudError;

/// Where things live inside the sandbox, relative to its workspace root
/// (`/home/user/workspace` on E2B).
pub mod paths {
    pub const SPEC: &str = ".mira-task/spec.json";
    pub const SECRETS: &str = ".mira-task/secrets.json";
    pub const LOG: &str = ".mira-task/task.log";
    pub const REPO: &str = "repo";
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TaskSpec {
    /// Short id (`t-3f9a1c`), also the branch suffix.
    pub id: String,
    /// What the user asked for, verbatim.
    pub prompt: String,
    pub repo: RepoSpec,
    /// Branch to start from (must exist on the remote).
    pub base_branch: String,
    /// Branch the work lands on; created by the worker.
    pub branch: String,
    /// Where the worker clones to; relative paths are resolved against the
    /// worker's working directory.
    pub workdir: PathBuf,
    pub git_identity: GitIdentity,
    pub model: ModelSpec,
    pub limits: Limits,
    /// Environment setup script, run in the repo after cloning.
    #[serde(default)]
    pub setup: Option<String>,
    /// E2B sandbox id, so the worker can shut its own sandbox down when
    /// done. `None` when not running in E2B (e.g. tests).
    #[serde(default)]
    pub sandbox_id: Option<String>,
    /// E2B control-plane URL for that shutdown.
    #[serde(default)]
    pub e2b_api_url: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RepoSpec {
    pub owner: String,
    pub name: String,
    /// Clone URL (https, or `file://` in tests).
    pub clone_url: String,
    /// GitHub REST base, `https://api.github.com` by default.
    pub api_url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GitIdentity {
    pub name: String,
    pub email: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ModelSpec {
    /// Provider name as in `mira.yaml` (`anthropic`, `openrouter`, …).
    pub provider: String,
    pub base_url: String,
    pub model: String,
    #[serde(default)]
    pub evaluator_model: Option<String>,
    #[serde(default)]
    pub prompt_caching: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Limits {
    /// Wall-clock budget for the whole task, including setup and the
    /// final push. The worker stops early enough to deliver.
    pub max_runtime_secs: u64,
    /// Goal-loop iterations (work → evaluate → continue).
    pub max_iterations: usize,
    /// Spend cap in USD, when the model's pricing is known.
    #[serde(default)]
    pub budget_usd: Option<f64>,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_runtime_secs: 3600,
            max_iterations: 20,
            budget_usd: None,
        }
    }
}

/// Credentials, kept out of the spec, the environment and the logs.
#[derive(Clone, Serialize, Deserialize, Default)]
pub struct Secrets {
    pub model_api_key: String,
    pub github_token: String,
    /// Lets the worker delete its own sandbox when done.
    #[serde(default)]
    pub e2b_api_key: Option<String>,
}

impl std::fmt::Debug for Secrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secrets { .. }")
    }
}

impl Secrets {
    /// Read and delete the secrets file, so nothing the agent runs later
    /// can find it on disk.
    pub fn take(path: &Path) -> Result<Self, CloudError> {
        let raw = std::fs::read(path)
            .map_err(|e| CloudError::Config(format!("reading {}: {e}", path.display())))?;
        let _ = std::fs::remove_file(path);
        serde_json::from_slice(&raw).map_err(|e| CloudError::Config(format!("secrets file: {e}")))
    }
}

/// `t-` plus 6 hex characters.
pub fn new_task_id() -> String {
    let u = uuid::Uuid::new_v4().simple().to_string();
    format!("t-{}", &u[..6])
}

/// Branch name for a task: `mira/<slug>-<id>`.
pub fn branch_name(prompt: &str, id: &str) -> String {
    let slug: String = prompt
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .take(6)
        .collect::<Vec<_>>()
        .join("-");
    let slug: String = slug.chars().take(40).collect();
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() {
        format!("mira/{id}")
    } else {
        format!("mira/{slug}-{id}")
    }
}

/// Parse `owner/name` from a GitHub remote URL (https or ssh).
pub fn parse_github_remote(url: &str) -> Option<(String, String)> {
    let url = url.trim();
    let rest = url
        .strip_prefix("git@github.com:")
        .or_else(|| url.strip_prefix("ssh://git@github.com/"))
        .or_else(|| url.strip_prefix("https://github.com/"))
        .or_else(|| url.strip_prefix("http://github.com/"))
        .or_else(|| {
            // https://user@github.com/… or https://x-access-token:…@github.com/…
            url.strip_prefix("https://")
                .and_then(|r| r.split_once("@github.com/").map(|(_, r)| r))
        })?;
    let rest = rest.trim_end_matches('/').trim_end_matches(".git");
    let (owner, name) = rest.split_once('/')?;
    if owner.is_empty() || name.is_empty() || name.contains('/') {
        return None;
    }
    Some((owner.to_owned(), name.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_names_are_slugged_and_bounded() {
        assert_eq!(
            branch_name("Fix the flaky auth test!", "t-abc123"),
            "mira/fix-the-flaky-auth-test-t-abc123"
        );
        assert_eq!(branch_name("???", "t-1"), "mira/t-1");
        let long = branch_name(&"word ".repeat(40), "t-1");
        assert!(long.len() < 60, "{long}");
    }

    #[test]
    fn github_remotes_parse() {
        let want = Some(("runmira".to_owned(), "mira".to_owned()));
        assert_eq!(parse_github_remote("git@github.com:runmira/mira.git"), want);
        assert_eq!(parse_github_remote("https://github.com/runmira/mira"), want);
        assert_eq!(
            parse_github_remote("https://x@github.com/runmira/mira.git"),
            want
        );
        assert_eq!(
            parse_github_remote("ssh://git@github.com/runmira/mira.git"),
            want
        );
        assert_eq!(parse_github_remote("https://gitlab.com/a/b"), None);
    }

    #[test]
    fn secrets_are_deleted_once_read_and_never_printed() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.json");
        std::fs::write(&p, r#"{"model_api_key":"k","github_token":"g"}"#).unwrap();
        let s = Secrets::take(&p).unwrap();
        assert_eq!(s.github_token, "g");
        assert!(!p.exists());
        assert_eq!(format!("{s:?}"), "Secrets { .. }");
    }
}
