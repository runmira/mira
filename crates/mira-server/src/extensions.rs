//! MCP servers, plugins and custom slash commands, shared by the web
//! server and the terminal app.
//!
//! One [`Extensions`] per process owns the MCP manager (live
//! connections) and the plugin manager (marketplaces + installs). After
//! anything changes (a plugin installed, a server added, the project
//! switched) [`Extensions::reload`] recomputes what should run: MCP
//! servers reconnect or disconnect as needed, plugin skills are swapped
//! into the skill registry, and slash commands are re-read.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use mira_mcp::{ConfigProblem, McpManager, McpOptions, Status};
use mira_plugins::{Enabled, PluginManager, SlashCommand};
use mira_tools::builtin::skill::SkillHandle;

#[derive(Clone)]
pub struct Extensions {
    inner: Arc<Inner>,
}

struct Inner {
    mcp: McpManager,
    plugins: PluginManager,
    project: RwLock<Option<PathBuf>>,
    enabled: RwLock<Enabled>,
    commands: RwLock<Vec<SlashCommand>>,
    problems: RwLock<Vec<ConfigProblem>>,
    skills: RwLock<Option<SkillHandle>>,
}

/// A slash command the palette can offer.
#[derive(Clone, Debug, serde::Serialize)]
pub struct CommandInfo {
    /// Without the leading `/`.
    pub name: String,
    pub description: String,
    pub argument_hint: Option<String>,
    /// `user`, `project`, `plugin:<name>` or `mcp:<server>`.
    pub source: String,
}

impl Extensions {
    pub fn new(project: Option<PathBuf>) -> Self {
        Self::with_managers(
            project,
            McpManager::new(McpOptions::from_env()),
            PluginManager::new(PluginManager::default_root()),
        )
    }

    pub fn with_managers(
        project: Option<PathBuf>,
        mcp: McpManager,
        plugins: PluginManager,
    ) -> Self {
        let enabled = plugins.enabled();
        Self {
            inner: Arc::new(Inner {
                mcp,
                plugins,
                project: RwLock::new(project),
                enabled: RwLock::new(enabled),
                commands: RwLock::new(Vec::new()),
                problems: RwLock::new(Vec::new()),
                skills: RwLock::new(None),
            }),
        }
    }

    pub fn mcp(&self) -> &McpManager {
        &self.inner.mcp
    }

    pub fn plugins(&self) -> &PluginManager {
        &self.inner.plugins
    }

    pub fn project(&self) -> Option<PathBuf> {
        self.inner.project.read().unwrap().clone()
    }

    /// The skill registry to refresh with plugin skills on reload.
    pub fn attach_skills(&self, skills: SkillHandle) {
        *self.inner.skills.write().unwrap() = Some(skills);
    }

    /// What enabled plugins contribute right now.
    pub fn enabled(&self) -> Enabled {
        self.inner.enabled.read().unwrap().clone()
    }

    /// Skill directories of enabled plugins.
    pub fn plugin_skill_dirs(&self) -> Vec<PathBuf> {
        self.inner.enabled.read().unwrap().skill_dirs()
    }

    /// Agent files of enabled plugins.
    pub fn plugin_agent_files(&self) -> Vec<PathBuf> {
        mira_plugins::runtime::agent_files(&self.inner.enabled.read().unwrap())
    }

    /// Definitions that couldn't be used, from the last reload.
    pub fn problems(&self) -> Vec<ConfigProblem> {
        self.inner.problems.read().unwrap().clone()
    }

    pub async fn set_project(&self, project: Option<PathBuf>) {
        *self.inner.project.write().unwrap() = project;
        self.reload().await;
    }

    /// Recompute everything from disk: enabled plugins, MCP servers,
    /// commands, plugin skills.
    pub async fn reload(&self) {
        let enabled = self.inner.plugins.enabled();
        let project = self.project();
        let problems =
            mira_plugins::runtime::apply_mcp(&self.inner.mcp, project.as_deref(), &enabled);
        let commands = mira_plugins::runtime::slash_commands(project.as_deref(), &enabled);
        let skill_dirs = enabled.skill_dirs();
        *self.inner.enabled.write().unwrap() = enabled;
        *self.inner.problems.write().unwrap() = problems;
        *self.inner.commands.write().unwrap() = commands;
        let skills = self.inner.skills.read().unwrap().clone();
        if let Some(handle) = skills {
            let fresh = load_skills(project.as_deref(), &skill_dirs);
            *handle.write().await = Arc::new(fresh);
        }
    }

    /// Custom commands and MCP prompts, for slash palettes.
    pub fn commands(&self) -> Vec<CommandInfo> {
        let mut out: Vec<CommandInfo> = self
            .inner
            .commands
            .read()
            .unwrap()
            .iter()
            .map(|c| CommandInfo {
                name: c.name.clone(),
                description: c.description.clone(),
                argument_hint: c.argument_hint.clone(),
                source: match &c.source {
                    mira_plugins::CommandSource::User => "user".into(),
                    mira_plugins::CommandSource::Project => "project".into(),
                    mira_plugins::CommandSource::Plugin { plugin } => format!("plugin:{plugin}"),
                },
            })
            .collect();
        for (server, p) in self.inner.mcp.prompts() {
            out.push(CommandInfo {
                name: mira_mcp::tool_name(&server, &p.name),
                description: p.description.unwrap_or_default(),
                argument_hint: (!p.arguments.is_empty()).then(|| {
                    p.arguments
                        .iter()
                        .map(|a| format!("<{}>", a.name))
                        .collect::<Vec<_>>()
                        .join(" ")
                }),
                source: format!("mcp:{server}"),
            });
        }
        out
    }

    /// Expand `/name args` if `name` is a custom command or an MCP
    /// prompt. `None` means it isn't one of ours.
    pub async fn expand(&self, name: &str, args: &str) -> Option<Result<String, String>> {
        let cmd = self
            .inner
            .commands
            .read()
            .unwrap()
            .iter()
            .find(|c| c.matches(name))
            .cloned();
        if let Some(cmd) = cmd {
            let cwd = self.project().unwrap_or_else(|| PathBuf::from("."));
            return Some(Ok(mira_plugins::commands::render(&cmd, args, &cwd).await));
        }
        let rest = name.strip_prefix("mcp__")?;
        let prompts = self.inner.mcp.prompts();
        let (server, prompt) = prompts.iter().find(|(server, p)| {
            mira_mcp::tool_name(server, &p.name)
                .strip_prefix("mcp__")
                .is_some_and(|n| n == rest)
        })?;
        // Positional arguments fill the prompt's arguments in order; the
        // last one takes the rest of the line.
        let mut values = std::collections::BTreeMap::new();
        let mut remaining = args.trim();
        let n = prompt.arguments.len();
        for (i, a) in prompt.arguments.iter().enumerate() {
            if remaining.is_empty() {
                break;
            }
            let value = if i + 1 == n {
                std::mem::take(&mut remaining).to_owned()
            } else {
                let (v, r) = remaining
                    .split_once(char::is_whitespace)
                    .unwrap_or((remaining, ""));
                remaining = r.trim_start();
                v.to_owned()
            };
            values.insert(a.name.clone(), value);
        }
        if let Some(missing) = prompt
            .arguments
            .iter()
            .find(|a| a.required && !values.contains_key(&a.name))
        {
            return Some(Err(format!("`/{name}` needs <{}>", missing.name)));
        }
        Some(
            self.inner
                .mcp
                .get_prompt(server, &prompt.name, values)
                .await,
        )
    }

    /// One-line notices for startup: servers waiting on approval or
    /// sign-in, and definitions that couldn't be used.
    pub fn notices(&self) -> Vec<String> {
        let mut out = Vec::new();
        let servers = self.inner.mcp.servers();
        let approval: Vec<&str> = servers
            .iter()
            .filter(|s| s.status == Status::NeedsApproval)
            .map(|s| s.name.as_str())
            .collect();
        if !approval.is_empty() {
            out.push(format!(
                "This project defines MCP servers that need your approval before they run: {}. \
                 Review them with /mcp.",
                approval.join(", ")
            ));
        }
        for p in self.problems() {
            if p.message.starts_with("overridden") {
                continue;
            }
            out.push(format!(
                "MCP config {}{}: {}",
                p.source.display(),
                p.server.map(|s| format!(" ({s})")).unwrap_or_default(),
                p.message
            ));
        }
        out
    }
}

/// The skill registry for `project`, with enabled plugins' skills.
pub fn load_skills(project: Option<&Path>, plugin_dirs: &[PathBuf]) -> mira_skills::SkillRegistry {
    let project_dirs = project
        .map(mira_config::well_known_project_skills_dirs)
        .unwrap_or_default();
    mira_skills::SkillRegistry::load_layered_with_plugins(
        &mira_config::shared_skills_dir(),
        &mira_config::user_skills_dir(),
        &project_dirs,
        plugin_dirs,
    )
}
