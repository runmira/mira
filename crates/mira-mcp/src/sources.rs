//! Gather server definitions from every place they can live and resolve
//! name clashes (local > project > user).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use mira_config::McpServerConfig;
use serde::{Deserialize, Serialize};

use crate::spec::{parse_mcp_json, Scope, ServerSpec};
use crate::state::McpState;

/// A definition that couldn't be used, shown next to the server list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigProblem {
    pub source: PathBuf,
    #[serde(default)]
    pub server: Option<String>,
    pub message: String,
}

#[derive(Clone, Debug, Default)]
pub struct Collected {
    pub servers: Vec<ServerSpec>,
    pub problems: Vec<ConfigProblem>,
}

/// Everything [`collect`] reads.
pub struct SourceInputs<'a> {
    /// `mcp_servers:` from the user's global config.
    pub user: &'a BTreeMap<String, McpServerConfig>,
    pub user_path: &'a Path,
    /// The project whose `.mcp.json` / local servers apply.
    pub project: Option<&'a Path>,
    pub state: &'a McpState,
    /// Servers from enabled plugins, already named.
    pub plugins: Vec<ServerSpec>,
}

pub fn collect(inputs: SourceInputs<'_>) -> Collected {
    let mut out = Collected::default();
    let mut by_name: BTreeMap<String, ServerSpec> = BTreeMap::new();
    let mut add = |spec: ServerSpec, out: &mut Collected| match by_name.get(&spec.name) {
        Some(existing) if !spec.scope.outranks(&existing.scope) => {
            out.problems.push(ConfigProblem {
                source: spec.source.clone().unwrap_or_default(),
                server: Some(spec.name.clone()),
                message: format!(
                    "overridden by the {} server with the same name",
                    existing.scope.label()
                ),
            });
        }
        Some(existing) => {
            out.problems.push(ConfigProblem {
                source: existing.source.clone().unwrap_or_default(),
                server: Some(existing.name.clone()),
                message: format!(
                    "overridden by the {} server with the same name",
                    spec.scope.label()
                ),
            });
            by_name.insert(spec.name.clone(), spec);
        }
        None => {
            by_name.insert(spec.name.clone(), spec);
        }
    };

    for (name, cfg) in inputs.user {
        add(
            ServerSpec::from_config(name, Scope::User, cfg).with_source(inputs.user_path),
            &mut out,
        );
    }

    if let Some(project) = inputs.project {
        let mcp_json = project.join(".mcp.json");
        if let Ok(text) = std::fs::read_to_string(&mcp_json) {
            match parse_mcp_json(&text) {
                Ok(entries) => {
                    for (name, cfg) in entries {
                        match cfg {
                            Ok(cfg) => add(
                                ServerSpec::from_config(&name, Scope::Project, &cfg)
                                    .with_source(&mcp_json),
                                &mut out,
                            ),
                            Err(message) => out.problems.push(ConfigProblem {
                                source: mcp_json.clone(),
                                server: Some(name),
                                message,
                            }),
                        }
                    }
                }
                Err(message) => out.problems.push(ConfigProblem {
                    source: mcp_json.clone(),
                    server: None,
                    message,
                }),
            }
        }

        let yaml = project.join(".mira").join("config.yaml");
        if let Ok(text) = std::fs::read_to_string(&yaml) {
            #[derive(Deserialize)]
            struct Only {
                #[serde(default)]
                mcp_servers: BTreeMap<String, McpServerConfig>,
            }
            match serde_yaml::from_str::<Only>(&text) {
                Ok(only) => {
                    for (name, cfg) in only.mcp_servers {
                        add(
                            ServerSpec::from_config(&name, Scope::Project, &cfg).with_source(&yaml),
                            &mut out,
                        );
                    }
                }
                Err(e) => out.problems.push(ConfigProblem {
                    source: yaml.clone(),
                    server: None,
                    message: format!("mcp_servers: {e}"),
                }),
            }
        }

        for (name, cfg) in inputs.state.local_servers(Some(project)) {
            add(
                ServerSpec::from_config(&name, Scope::Local, &cfg)
                    .with_source(crate::paths::state_file()),
                &mut out,
            );
        }
    }

    for spec in inputs.plugins {
        add(spec, &mut out);
    }

    out.servers = by_name.into_values().collect();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use mira_config::{McpHttpConfig, McpStdioConfig};

    #[test]
    fn project_and_local_override_user() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(".mcp.json"),
            r#"{"mcpServers": {"gh": {"type": "http", "url": "https://project"}, "fs": {"command": "fs"}}}"#,
        )
        .unwrap();
        let mut user = BTreeMap::new();
        user.insert(
            "gh".to_owned(),
            McpServerConfig::Http(McpHttpConfig {
                url: "https://user".into(),
                ..Default::default()
            }),
        );
        user.insert(
            "only-user".to_owned(),
            McpServerConfig::Stdio(McpStdioConfig {
                command: "x".into(),
                ..Default::default()
            }),
        );
        let mut state = McpState::default();
        state.set_local_server(
            dir.path(),
            "fs",
            Some(McpServerConfig::Stdio(McpStdioConfig {
                command: "local-fs".into(),
                ..Default::default()
            })),
        );
        let got = collect(SourceInputs {
            user: &user,
            user_path: Path::new("/home/u/.mira/mira.yaml"),
            project: Some(dir.path()),
            state: &state,
            plugins: vec![],
        });
        let find = |n: &str| got.servers.iter().find(|s| s.name == n).unwrap();
        assert_eq!(find("gh").scope, Scope::Project);
        assert_eq!(find("gh").transport.url(), Some("https://project"));
        assert_eq!(find("fs").scope, Scope::Local);
        assert_eq!(find("only-user").scope, Scope::User);
        assert_eq!(got.servers.len(), 3);
        assert_eq!(got.problems.len(), 2, "{:?}", got.problems);
    }
}
