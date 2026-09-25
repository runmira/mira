//! Model Context Protocol client for Mira.
//!
//! - [`spec`]: what a server is (scope + transport), `.mcp.json` parsing,
//!   `${VAR}` expansion, tool naming.
//! - [`sources`]: gather definitions from the user config, the project
//!   (`.mcp.json`, `.mira/config.yaml`), local servers and plugins.
//! - [`state`]: approvals of project servers, disabled servers, local
//!   servers (`~/.mira/mcp/state.json`).
//! - [`manager`]: the live connections: parallel connect with timeouts,
//!   reconnect with backoff, live config changes, OAuth sign-in, status
//!   events, and the tools handed to sessions.
//!
//! - [`vars`]: saved values for `${VAR}`s (tokens pasted in the UI).
//!
//! Transports: stdio, streamable HTTP and the older HTTP+SSE.

pub mod auth;
pub mod manager;
pub mod paths;
pub mod sources;
pub mod spec;
pub mod sse;
pub mod state;
mod tool;
pub mod vars;

pub use manager::{
    McpEvent, McpManager, McpOptions, PromptArgView, PromptView, ResourceView, ServerView, Status,
    ToolLoading, ToolView,
};
pub use sources::{collect, Collected, ConfigProblem, SourceInputs};
pub use spec::{parse_mcp_json, sanitize, tool_name, Scope, ServerSpec, Transport};
pub use state::{Approval, McpState};
pub use tool::McpTool;
