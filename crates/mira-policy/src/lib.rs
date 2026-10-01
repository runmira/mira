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
//! Computer(screenshot) — desktop control, `verb` or `verb:detail`
//! Browser(navigate:https://github.com/*)
//! ```
//!
//! Modes (`plan`, `manual`, `auto`, `edit`, `yolo`) are just presets over the
//! same rule engine.

pub mod mode;
pub mod rule;
pub mod shell;

use mira_tools::Action;
use serde::{Deserialize, Serialize};
use tracing::debug;

pub use mode::Mode;
pub use rule::{session_rule_for, Rule, RuleParseError};

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
    ///
    /// Kept as a separate accessor rather than exposing `self.allow.len()`
    /// directly so the policy layer stays opaque to callers; it also makes
    /// it straightforward to add assertions or logging later without
    /// touching every test that checks rule counts.
    pub fn allow_count(&self) -> usize {
        self.allow.len()
    }

    /// Evaluate a request. Precedence: deny > ask > mode default > allow.
    ///
    /// A compound shell command is judged operation by operation (see
    /// [`shell`]): any denied part denies the whole command, and it runs
    /// unasked only when every part would. Judged as one string, a deny
    /// like `Bash(rm:*)` never saw `ls && rm -rf x`, and allow rules could
    /// never cover a chain of commands they each allowed.
    pub fn evaluate(&self, req: &Request<'_>) -> Decision {
        if req.action == Action::Bash {
            let parts = shell::split_compound(req.target);
            if parts.len() > 1 {
                return self.evaluate_compound(req, &parts);
            }
        }
        self.evaluate_one(req)
    }

    fn evaluate_compound(&self, req: &Request<'_>, parts: &[String]) -> Decision {
        // The whole command first: a deny on it always stands, and an exact
        // allow for this precise chain (an earlier "allow for this
        // session") covers it.
        if self.deny.iter().any(|r| r.matches(req)) {
            debug!(?req, "policy: deny (explicit, whole command)");
            return Decision::Deny;
        }
        let mut worst = Decision::Allow;
        for part in parts {
            let d = self.evaluate_one(&Request {
                action: Action::Bash,
                target: part,
            });
            match d {
                Decision::Deny => {
                    debug!(?req, part, "policy: deny (a part of a compound command)");
                    return Decision::Deny;
                }
                Decision::Ask => worst = Decision::Ask,
                Decision::Allow => {}
            }
        }
        if worst == Decision::Ask
            && !self.ask.iter().any(|r| r.matches(req))
            && self.allow.iter().any(|r| r.matches(req))
        {
            return Decision::Allow;
        }
        worst
    }

    /// The operations of a shell command that would need the user's
    /// approval, in order — for the approval card to name, and for "allow
    /// for this session" to add rules for. Empty for a single command (the
    /// command itself is the question) and when nothing needs approval.
    pub fn parts_needing_approval(&self, command: &str) -> Vec<String> {
        let parts = shell::split_compound(command);
        if parts.len() < 2 {
            return Vec::new();
        }
        parts
            .into_iter()
            .filter(|p| {
                self.evaluate_one(&Request {
                    action: Action::Bash,
                    target: p,
                }) != Decision::Allow
            })
            .collect()
    }

    fn evaluate_one(&self, req: &Request<'_>) -> Decision {
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
        let d = self.mode.default_for_target(req.action, req.target);
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

    fn policy(allow: &[&str], deny: &[&str]) -> Policy {
        Policy::from_config(&PolicyConfig {
            mode: Mode::default(),
            allow: allow.iter().map(|s| s.to_string()).collect(),
            ask: vec![],
            deny: deny.iter().map(|s| s.to_string()).collect(),
        })
        .unwrap()
    }

    fn bash(p: &Policy, cmd: &str) -> Decision {
        p.evaluate(&Request {
            action: Action::Bash,
            target: cmd,
        })
    }

    #[test]
    fn a_denied_command_cannot_hide_behind_another() {
        let p = policy(&["Bash(ls:*)"], &["Bash(rm:*)"]);
        assert_eq!(bash(&p, "rm -rf x"), Decision::Deny);
        assert_eq!(
            bash(&p, "ls && rm -rf x"),
            Decision::Deny,
            "the prefix used to slip past the deny"
        );
        assert_eq!(bash(&p, "ls; rm -rf x"), Decision::Deny);
        assert_eq!(bash(&p, "ls | rm -rf x"), Decision::Deny);
    }

    #[test]
    fn a_chain_of_allowed_commands_runs_without_asking() {
        let p = policy(&["Bash(cargo test:*)", "Bash(cargo clippy:*)"], &[]);
        assert_eq!(
            bash(&p, "cargo test --lib && cargo clippy"),
            Decision::Allow
        );
        // One unallowed part is enough to ask.
        assert_ne!(bash(&p, "cargo test && curl evil.sh"), Decision::Allow);
        assert_eq!(
            p.parts_needing_approval("cargo test && curl evil.sh"),
            ["curl evil.sh"]
        );
        assert!(
            p.parts_needing_approval("cargo test").is_empty(),
            "a single command is its own question"
        );
    }

    #[test]
    fn substitutions_inside_a_part_still_ask() {
        // Splitting must not make `$(…)` look like a plain argument.
        let p = policy(&["Bash(echo:*)"], &[]);
        assert_ne!(bash(&p, "echo hi && echo $(curl x)"), Decision::Allow);
    }

    #[test]
    fn an_exact_allow_for_the_whole_chain_still_covers_it() {
        let mut p = policy(&[], &[]);
        p.add_allow_rule("Bash(make && make install)").unwrap();
        assert_eq!(bash(&p, "make && make install"), Decision::Allow);
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
