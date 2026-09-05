//! Permission engine.
//!
//! Every tool invocation runs through [`Policy::evaluate`], which returns
//! [`Decision::Allow`], [`Decision::Ask`], or [`Decision::Deny`]. The harness
//! surfaces `Ask` to the UI (approve/reject) and treats `Deny` as terminal.
//!
//! The DSL is one line per rule:
//!
//! ```text
//! Bash(cargo test:*)   — allow any `cargo test ...`
//! Edit(src/**)         — file globs match against paths inside cwd
//! Read(.env*)          — always evaluated; deny wins over allow
//! ```
//!
//! Modes (`plan`, `manual`, `auto`, `edit`, `yolo`) are just presets over the
//! same rule engine.

pub mod mode;
pub mod rule;

use mira_tools::Action;
use serde::{Deserialize, Serialize};
use tracing::debug;

pub use mode::Mode;
pub use rule::{Rule, RuleParseError};

/// What the policy engine decides about a specific invocation.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Ask,
    Deny,
}

/// Semantic subject of a policy check. `Action` says "what kind of thing";
/// `target` is the specific path, argv, or empty string for pure tools.
#[derive(Clone, Debug)]
pub struct Request<'a> {
    pub action: Action,
    pub target: &'a str,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PolicyConfig {
    #[serde(default)]
    pub mode: Mode,
    /// Rules override the mode. Order within a list doesn't matter — the
    /// engine evaluates deny > ask > allow.
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub ask: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
}

/// Compiled policy: parsed rules plus a mode.
pub struct Policy {
    mode: Mode,
    allow: Vec<Rule>,
    ask: Vec<Rule>,
    deny: Vec<Rule>,
}

impl Policy {
    pub fn from_config(cfg: &PolicyConfig) -> Result<Self, RuleParseError> {
        Ok(Self {
            mode: cfg.mode,
            allow: cfg
                .allow
                .iter()
                .map(|s| s.parse())
                .collect::<Result<_, _>>()?,
            ask: cfg
                .ask
                .iter()
                .map(|s| s.parse())
                .collect::<Result<_, _>>()?,
            deny: cfg
                .deny
                .iter()
                .map(|s| s.parse())
                .collect::<Result<_, _>>()?,
        })
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
    }

    /// Evaluate a request. Precedence: deny > ask > mode default > allow.
    pub fn evaluate(&self, req: &Request<'_>) -> Decision {
        if self.deny.iter().any(|r| r.matches(req)) {
            debug!(?req, "policy: deny (explicit)");
            return Decision::Deny;
        }
        if self.ask.iter().any(|r| r.matches(req)) {
            debug!(?req, "policy: ask (explicit)");
            return Decision::Ask;
        }
        if self.allow.iter().any(|r| r.matches(req)) {
            debug!(?req, "policy: allow (explicit)");
            return Decision::Allow;
        }
        let d = self.mode.default_for(req.action);
        debug!(?req, ?d, mode = ?self.mode, "policy: mode default");
        d
    }
}
