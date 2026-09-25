//! What an installed plugin contributes, found at the default locations
//! (`commands/`, `agents/`, `skills/`, `hooks/hooks.json`, `.mcp.json`)
//! plus any custom paths in its manifest.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use mira_config::McpServerConfig;
use serde::Serialize;
use serde_json::Value;

use crate::manifest::PluginManifest;

#[derive(Clone, Debug, Default, Serialize)]
pub struct Components {
    /// Slash command files.
    pub commands: Vec<PathBuf>,
    /// Subagent definition files.
    pub agents: Vec<PathBuf>,
    /// Directories holding `<skill>/SKILL.md`.
    pub skill_dirs: Vec<PathBuf>,
    /// Skill names found in `skill_dirs`, for display.
    pub skills: Vec<String>,
    /// Hook event names (`PreToolUse`, …). Mira doesn't run hooks yet.
    pub hooks: Vec<String>,
    /// The hooks definition (`{event: [{matcher, hooks: [...]}]}`).
    #[serde(skip)]
    pub hooks_config: Option<Value>,
    /// MCP servers by the plugin's name for them.
    #[serde(skip)]
    pub mcp_servers: Vec<(String, Result<McpServerConfig, String>)>,
    pub mcp_server_names: Vec<String>,
    /// LSP servers by name. Mira doesn't run LSP servers yet.
    pub lsp_servers: Vec<String>,
    /// Paths or entries that couldn't be used.
    pub problems: Vec<String>,
}

impl Components {
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
            && self.agents.is_empty()
            && self.skills.is_empty()
            && self.hooks.is_empty()
            && self.mcp_servers.is_empty()
            && self.lsp_servers.is_empty()
    }
}

/// Resolve a manifest path inside the plugin, refusing anything that
/// climbs out of it.
fn inside(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let rel_path = Path::new(rel.trim());
    if rel_path.is_absolute()
        || rel_path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::Prefix(_)))
    {
        return Err(format!("`{rel}` points outside the plugin"));
    }
    Ok(root.join(rel_path))
}

/// Manifest path fields are a string or a list of strings.
fn paths(v: &Option<Value>) -> Vec<String> {
    match v {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect(),
        _ => Vec::new(),
    }
}

fn md_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            md_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "md") {
            out.push(p);
        }
    }
}

fn add_md(
    root: &Path,
    rel: Option<&str>,
    default: &str,
    out: &mut Vec<PathBuf>,
    problems: &mut Vec<String>,
) {
    let p = match rel {
        Some(r) => match inside(root, r) {
            Ok(p) => p,
            Err(e) => {
                problems.push(e);
                return;
            }
        },
        None => root.join(default),
    };
    if p.is_file() {
        out.push(p);
    } else {
        md_files(&p, out);
    }
}

pub fn discover(root: &Path, manifest: &PluginManifest) -> Components {
    let mut c = Components::default();

    add_md(root, None, "commands", &mut c.commands, &mut c.problems);
    for p in paths(&manifest.commands) {
        add_md(root, Some(&p), "", &mut c.commands, &mut c.problems);
    }
    add_md(root, None, "agents", &mut c.agents, &mut c.problems);
    for p in paths(&manifest.agents) {
        add_md(root, Some(&p), "", &mut c.agents, &mut c.problems);
    }
    c.commands.dedup();
    c.agents.dedup();

    let mut skill_dirs = vec![root.join("skills")];
    for p in paths(&manifest.skills) {
        match inside(root, &p) {
            Ok(dir) => skill_dirs.push(dir),
            Err(e) => c.problems.push(e),
        }
    }
    for dir in skill_dirs {
        // A path may name one skill directory or a directory of them.
        if dir.join("SKILL.md").is_file() {
            if let (Some(parent), Some(name)) = (dir.parent(), dir.file_name()) {
                c.skills.push(name.to_string_lossy().into_owned());
                if !c.skill_dirs.contains(&parent.to_path_buf()) {
                    c.skill_dirs.push(parent.to_path_buf());
                }
            }
            continue;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut found = false;
        let mut names: Vec<String> = rd
            .flatten()
            .filter(|e| e.path().join("SKILL.md").is_file())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        found |= !names.is_empty();
        c.skills.extend(names);
        if found && !c.skill_dirs.contains(&dir) {
            c.skill_dirs.push(dir);
        }
    }

    let hooks = match &manifest.hooks {
        Some(Value::Object(o)) => Some(Value::Object(o.clone())),
        Some(Value::String(p)) => match inside(root, p) {
            Ok(p) => read_json(&p),
            Err(e) => {
                c.problems.push(e);
                None
            }
        },
        _ => read_json(&root.join("hooks").join("hooks.json")),
    };
    if let Some(h) = hooks {
        let events = h.get("hooks").unwrap_or(&h);
        if let Some(o) = events.as_object() {
            c.hooks = o.keys().cloned().collect();
        }
        c.hooks_config = Some(events.clone());
    }

    let mcp_text = match &manifest.mcp_servers {
        Some(Value::Object(o)) => Some(Value::Object(o.clone()).to_string()),
        Some(Value::String(p)) => match inside(root, p) {
            Ok(p) => std::fs::read_to_string(p).ok(),
            Err(e) => {
                c.problems.push(e);
                None
            }
        },
        _ => std::fs::read_to_string(root.join(".mcp.json")).ok(),
    };
    if let Some(text) = mcp_text {
        match mira_mcp::parse_mcp_json(&text) {
            Ok(servers) => c.mcp_servers = servers,
            Err(e) => c.problems.push(format!("MCP servers: {e}")),
        }
    }
    c.mcp_server_names = c.mcp_servers.iter().map(|(n, _)| n.clone()).collect();

    let lsp = match &manifest.lsp_servers {
        Some(Value::Object(o)) => Some(Value::Object(o.clone())),
        Some(Value::String(p)) => inside(root, p).ok().and_then(|p| read_json(&p)),
        _ => read_json(&root.join(".lsp.json")),
    };
    if let Some(Value::Object(o)) = lsp {
        c.lsp_servers = o.keys().cloned().collect();
    }
    c
}

fn read_json(p: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()
}

/// MCP server definitions keyed for the manager, with the plugin's
/// directory for `${CLAUDE_PLUGIN_ROOT}`.
pub fn mcp_specs(
    plugin: &str,
    root: &Path,
    c: &Components,
) -> (Vec<mira_mcp::ServerSpec>, Vec<String>) {
    let mut specs = Vec::new();
    let mut problems = Vec::new();
    for (name, cfg) in &c.mcp_servers {
        match cfg {
            Ok(cfg) => {
                let mut spec = mira_mcp::ServerSpec::from_config(
                    &format!("plugin:{plugin}:{name}"),
                    mira_mcp::Scope::Plugin {
                        plugin: plugin.to_owned(),
                    },
                    cfg,
                )
                .with_source(root);
                spec.plugin_root = Some(root.to_path_buf());
                specs.push(spec);
            }
            Err(e) => problems.push(format!("MCP server `{name}`: {e}")),
        }
    }
    (specs, problems)
}

/// Commands, agents, etc. of several plugins, keyed by plugin id.
pub type ByPlugin<T> = BTreeMap<String, T>;

#[cfg(test)]
mod tests {
    use super::*;

    fn write(p: &Path, text: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    #[test]
    fn finds_default_and_custom_components() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        write(
            &r.join("commands/commit.md"),
            "---\ndescription: c\n---\nbody",
        );
        write(&r.join("commands/sub/deep.md"), "x");
        write(&r.join("extra/one.md"), "x");
        write(&r.join("agents/reviewer.md"), "x");
        write(&r.join("skills/pdf/SKILL.md"), "x");
        write(&r.join("more-skills/single/SKILL.md"), "x");
        write(
            &r.join("hooks/hooks.json"),
            r#"{"hooks": {"PreToolUse": [], "Stop": []}}"#,
        );
        write(
            &r.join(".mcp.json"),
            r#"{"mcpServers": {"db": {"command": "${CLAUDE_PLUGIN_ROOT}/db"}}}"#,
        );
        let manifest = PluginManifest {
            commands: Some(serde_json::json!(["./extra/one.md", "../escape"])),
            skills: Some(serde_json::json!("./more-skills/single")),
            ..Default::default()
        };
        let c = discover(r, &manifest);
        let names: Vec<_> = c
            .commands
            .iter()
            .map(|p| p.file_stem().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["commit", "deep", "one"]);
        assert_eq!(c.agents.len(), 1);
        assert_eq!(c.skills, ["pdf", "single"]);
        assert_eq!(c.skill_dirs.len(), 2);
        assert_eq!(c.hooks, ["PreToolUse", "Stop"]);
        assert_eq!(c.mcp_server_names, ["db"]);
        assert_eq!(c.problems.len(), 1, "{:?}", c.problems);

        let (specs, _) = mcp_specs("tools", r, &c);
        assert_eq!(specs[0].name, "plugin:tools:db");
        assert_eq!(
            specs[0].extra_vars()["CLAUDE_PLUGIN_ROOT"],
            r.display().to_string()
        );
    }
}
