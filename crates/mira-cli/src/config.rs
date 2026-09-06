//! Re-exports of the shared `mira-config` types so existing
//! `crate::config::*` imports keep working while the on-disk format lives
//! in one crate that both the CLI and server can depend on.

pub use mira_config::{MiraConfig, ProviderConfig};
