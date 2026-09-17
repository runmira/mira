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
    /// Raw deny strings kept alongside the parsed rules so subagents can
    /// rebuild a policy that inherits the parent's explicit denies (see
    /// [`Self::deny_source`]). Without this, an autonomous read-only child
    /// would silently lose e.g. `Deny(Read("**/.env"))` when the parent
    /// spawns it on a fresh Auto policy.
    deny_source: Vec<String>,
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
            deny_source: cfg.deny.clone(),
        })
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
    }

    /// Parse `rule_str` and append it to the allow list. Used by the
    /// server when the user picks "Allow for this session" on an
    /// approval prompt: the specific target the model just asked about
    /// gets promoted from `Ask` to `Allow` for the rest of the session.
    ///
    /// No dedup — repeated identical calls grow the list linearly, but
    /// `Policy::evaluate` uses `.any()` so the observable behaviour is
    /// unchanged and the extra memory is negligible for a session.
    pub fn add_allow_rule(&mut self, rule_str: &str) -> Result<(), RuleParseError> {
        let rule: Rule = rule_str.parse()?;
        self.allow.push(rule);
        Ok(())
    }

    /// Original deny-rule strings, in the order they were parsed. Used
    /// by subagent spawn plumbing so a child that runs on a fresh Auto
    /// policy still inherits explicit denies the user (or the parent's
    /// config) set. Returning the raw form (rather than parsed `Rule`s)
    /// keeps the child free to rebuild them cleanly via `from_config`.
    pub fn deny_source(&self) -> &[String] {
        &self.deny_source
    }

    /// Number of allow rules — mainly for tests / diagnostics.
    pub fn allow_count(&self) -> usize {
        self.allow.len()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn base_ask_policy() -> Policy {
        Policy::from_config(&PolicyConfig {
            mode: Mode::default(),
            allow: vec![],
            ask: vec!["Bash(*)".into()],
            deny: vec![],
        })
        .unwrap()
    }

    #[test]
    fn add_allow_rule_promotes_a_specific_target_from_ask_to_allow() {
        let mut p = base_ask_policy();
        let cmd = "git status";
        assert_eq!(
            p.evaluate(&Request {
                action: Action::Bash,
                target: cmd
            }),
            Decision::Ask
        );
        p.add_allow_rule(&format!("Bash({cmd})")).unwrap();
        assert_eq!(
            p.evaluate(&Request {
                action: Action::Bash,
                target: cmd
            }),
            Decision::Allow
        );
        // A sibling command still asks — the rule was scoped, not blanket.
        assert_eq!(
            p.evaluate(&Request {
                action: Action::Bash,
                target: "git push"
            }),
            Decision::Ask
        );
    }

    #[test]
    fn add_allow_rule_rejects_a_malformed_string() {
        let mut p = base_ask_policy();
        assert!(p.add_allow_rule("not a rule").is_err());
    }
}
