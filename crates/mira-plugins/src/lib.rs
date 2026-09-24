//! Plugins for Mira, in Claude Code's format.
//!
//! A *marketplace* is a catalog (`.claude-plugin/marketplace.json`) in a
//! git repo, a local folder, or at a URL. A *plugin* bundles any of:
//! slash commands (`commands/`), subagents (`agents/`), skills
//! (`skills/`), MCP servers (`.mcp.json`), hooks and LSP servers. Mira
//! runs commands, agents, skills and MCP servers; hooks and LSP servers
//! are listed but not run yet.
//!
//! - [`manifest`]: the file formats.
//! - [`manager::PluginManager`]: add/update/remove marketplaces;
//!   install/uninstall/enable/disable plugins; what enabled plugins
//!   contribute.
//! - [`components`]: find a plugin's parts.
//! - [`commands`]: custom slash commands (user, project and plugin).

pub mod commands;
pub mod components;
mod git;
pub mod manager;
pub mod manifest;
pub mod runtime;

pub use commands::{CommandSource, SlashCommand};
pub use components::Components;
pub use manager::{
    CatalogEntry, Enabled, EnabledPlugin, InstalledPlugin, MarketplaceSource, MarketplaceView,
    PluginDetail, PluginManager,
};
