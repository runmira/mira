//! Instance-routing provider pool.
//!
//! The type lives in `mira-engine` so the CLI TUI shares it; this module
//! keeps the historical `crate::provider::SwappableProvider` path stable.

pub use mira_engine::pool::SwappableProvider;
