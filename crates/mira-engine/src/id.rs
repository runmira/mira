//! Engine identifiers.
//!
//! Two distinct concepts that used to be conflated under "provider":
//!
//! - [`DriverKind`] names an *implementation* (`native`, `claude-code`,
//!   `codex`, a fork's custom driver). It selects how the backend is
//!   reached.
//! - [`EngineId`] names one *configured instance* — the routing key every
//!   persisted selection, settings entry, and WS frame refers to. Two
//!   instances can share a driver (`work` and `personal` both on
//!   `codex`) with independent config.
//!
//! Both are deliberately **open** slugs, not closed enums. Configuration
//! written by a newer build, a fork, or by hand must still parse; whether
//! a driver is actually loadable is discovered at runtime (see
//! [`crate::snapshot::EngineState::Unavailable`]), never at decode time.

use serde::{Deserialize, Serialize};
use std::borrow::Borrow;
use std::fmt;

/// A driver kind or engine id: starts with a letter, then letters,
/// digits, `-` or `_`; at most 64 characters.
pub fn is_valid_slug(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    s.len() <= 64 && chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

macro_rules! engine_slug {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String")]
        pub struct $name(String);

        impl $name {
            pub fn new(s: impl Into<String>) -> Option<Self> {
                let s = s.into();
                is_valid_slug(&s).then_some(Self(s))
            }

            /// The raw slug. Borrowed form for keys and comparisons.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = String;
            fn try_from(s: String) -> Result<Self, Self::Error> {
                if is_valid_slug(&s) {
                    Ok(Self(s))
                } else {
                    Err(format!(
                        "`{s}` is not a valid {}: letters, digits, `-` and `_` only, \
                         starting with a letter, at most 64 characters",
                        stringify!($name)
                    ))
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl Borrow<str> for $name {
            fn borrow(&self) -> &str {
                &self.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
    };
}

engine_slug!(
    /// One configured engine instance — the routing key for selections.
    EngineId
);
engine_slug!(
    /// Which driver implementation backs an instance.
    DriverKind
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_accept_the_ordinary_shapes() {
        assert!(is_valid_slug("native"));
        assert!(is_valid_slug("claude-code"));
        assert!(is_valid_slug("codex_personal"));
        assert!(is_valid_slug("MyFork2"));
        assert!(EngineId::new("work-codex").is_some());
    }

    #[test]
    fn slugs_reject_everything_else() {
        assert!(!is_valid_slug(""));
        assert!(!is_valid_slug("2fast"));
        assert!(!is_valid_slug("has space"));
        assert!(!is_valid_slug("../escape"));
        let long = "a".repeat(65);
        assert!(!is_valid_slug(&long));
        assert!(EngineId::new("../escape").is_none());
    }

    #[test]
    fn slugs_round_trip_through_serde() {
        let id: EngineId = serde_json::from_str("\"claude-code\"").unwrap();
        assert_eq!(id.as_str(), "claude-code");
        assert!(serde_json::from_str::<EngineId>("\"../bad\"").is_err());
    }
}
