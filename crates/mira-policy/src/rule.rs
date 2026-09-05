use std::str::FromStr;

use glob::Pattern;
use mira_tools::Action;
use thiserror::Error;

use crate::Request;

/// A single parsed permission rule.
///
/// Two matcher flavors:
///
/// - **path glob**: `Read(src/**)`, `Edit(**/*.rs)` — matched with `glob::Pattern`.
/// - **argv prefix with wildcard tail**: `Bash(cargo test:*)` matches any command
///   that starts with `cargo test`. A trailing `:*` means "anything after";
///   otherwise the whole target must equal the pattern.
#[derive(Clone, Debug)]
pub struct Rule {
    action: Action,
    matcher: Matcher,
}

#[derive(Clone, Debug)]
enum Matcher {
    /// Filesystem-style glob for file paths.
    Glob(Pattern),
    /// Bash prefix (exact match or `prefix:*` wildcard).
    BashPrefix { prefix: String, wildcard: bool },
}

impl Rule {
    pub fn matches(&self, req: &Request<'_>) -> bool {
        if req.action != self.action {
            return false;
        }
        match &self.matcher {
            Matcher::Glob(pat) => pat.matches(req.target),
            Matcher::BashPrefix { prefix, wildcard } => {
                if *wildcard {
                    req.target == prefix.as_str() || req.target.starts_with(&format!("{prefix} "))
                } else {
                    req.target == prefix.as_str()
                }
            }
        }
    }
}

#[derive(Debug, Error)]
pub enum RuleParseError {
    #[error("rule `{0}` must look like `Action(pattern)`")]
    Shape(String),
    #[error("unknown action `{0}`; expected Read/Edit/Write/Bash")]
    UnknownAction(String),
    #[error("bad glob in `{rule}`: {source}")]
    BadGlob {
        rule: String,
        #[source]
        source: glob::PatternError,
    },
}

impl FromStr for Rule {
    type Err = RuleParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        let (head, tail) = s
            .split_once('(')
            .ok_or_else(|| RuleParseError::Shape(s.to_owned()))?;
        let pattern = tail
            .strip_suffix(')')
            .ok_or_else(|| RuleParseError::Shape(s.to_owned()))?;

        let action = match head.trim() {
            "Read" => Action::Read,
            "Edit" => Action::Edit,
            "Write" => Action::Write,
            "Bash" => Action::Bash,
            other => return Err(RuleParseError::UnknownAction(other.to_owned())),
        };

        let matcher =
            match action {
                Action::Bash => {
                    if let Some(prefix) = pattern.strip_suffix(":*") {
                        Matcher::BashPrefix {
                            prefix: prefix.trim().to_owned(),
                            wildcard: true,
                        }
                    } else {
                        Matcher::BashPrefix {
                            prefix: pattern.trim().to_owned(),
                            wildcard: false,
                        }
                    }
                }
                _ => Matcher::Glob(Pattern::new(pattern).map_err(|source| {
                    RuleParseError::BadGlob {
                        rule: s.to_owned(),
                        source,
                    }
                })?),
            };

        Ok(Self { action, matcher })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(action: Action, target: &str) -> Request<'_> {
        Request { action, target }
    }

    #[test]
    fn bash_wildcard() {
        let r: Rule = "Bash(cargo test:*)".parse().unwrap();
        assert!(r.matches(&req(Action::Bash, "cargo test")));
        assert!(r.matches(&req(Action::Bash, "cargo test --lib")));
        assert!(!r.matches(&req(Action::Bash, "cargo build")));
        assert!(!r.matches(&req(Action::Edit, "cargo test")));
    }

    #[test]
    fn bash_exact() {
        let r: Rule = "Bash(cargo fmt)".parse().unwrap();
        assert!(r.matches(&req(Action::Bash, "cargo fmt")));
        assert!(!r.matches(&req(Action::Bash, "cargo fmt --check")));
    }

    #[test]
    fn edit_glob() {
        let r: Rule = "Edit(src/**)".parse().unwrap();
        assert!(r.matches(&req(Action::Edit, "src/main.rs")));
        assert!(r.matches(&req(Action::Edit, "src/lib/foo.rs")));
        assert!(!r.matches(&req(Action::Edit, "tests/foo.rs")));
    }
}
