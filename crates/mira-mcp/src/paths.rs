//! Where MCP keeps its files, next to `~/.mira/mira.yaml`.

use std::path::PathBuf;

fn mira_dir() -> PathBuf {
    mira_config::global_path()
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".mira"))
}

/// Approvals, disabled servers, local servers.
pub fn state_file() -> PathBuf {
    mira_dir().join("mcp").join("state.json")
}

/// OAuth tokens (mode 0600).
pub fn credentials_file() -> PathBuf {
    mira_dir().join("mcp").join("credentials.json")
}

/// Stdio servers' stderr, one file per server.
pub fn log_dir() -> PathBuf {
    mira_dir().join("logs").join("mcp")
}
