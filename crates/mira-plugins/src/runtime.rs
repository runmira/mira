//! Glue shared by the terminal app and the web server: turn the current
//! configuration (user config, project files, local servers, enabled
//! plugins) into the MCP servers to run, and load slash commands.

use std::path::{Path, PathBuf};

use mira_mcp::{collect, ConfigProblem, McpManager, McpState, SourceInputs};

use crate::commands::SlashCommand;
use crate::manager::Enabled;

/// `~/.mira/commands`.
pub fn user_commands_dir() -> PathBuf {
    mira_config::global_path()
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".mira"))
        .join("commands")
}

/// Compute the MCP servers for `project` and hand them to `mcp`, which
/// connects, reconnects or disconnects as needed. Returns definitions
/// that couldn't be used.
pub fn apply_mcp(
    mcp: &McpManager,
    project: Option<&Path>,
    enabled: &Enabled,
) -> Vec<ConfigProblem> {
    let user = match mira_config::MiraConfig::load_global() {
        Ok(c) => c.mcp_servers,
        Err(e) => {
            return vec![ConfigProblem {
                source: mira_config::global_path(),
                server: None,
                message: format!("{e:#}"),
            }]
        }
    };
    let state = McpState::load(&mcp.options().state_file).unwrap_or_default();
    let (plugin_specs, plugin_problems) = enabled.mcp_specs();
    let collected = collect(SourceInputs {
        user: &user,
        user_path: &mira_config::global_path(),
        project,
        state: &state,
        plugins: plugin_specs,
    });
    mcp.apply(collected.servers, project.map(Path::to_path_buf));
    let mut problems = collected.problems;
    problems.extend(plugin_problems.into_iter().map(|message| ConfigProblem {
        source: PathBuf::new(),
        server: None,
        message,
    }));
    problems
}

/// Slash commands for `project`: project, user, then plugin commands.
pub fn slash_commands(project: Option<&Path>, enabled: &Enabled) -> Vec<SlashCommand> {
    crate::commands::load(
        Some(&user_commands_dir()),
        project,
        &enabled.command_files(),
    )
}

/// Agent files of enabled plugins, for `mira_agents::load_with_plugins`.
pub fn agent_files(enabled: &Enabled) -> Vec<PathBuf> {
    enabled.agent_files().into_iter().map(|(_, p)| p).collect()
}
