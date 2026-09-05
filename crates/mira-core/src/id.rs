use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Typed IDs. Newtypes over `String` because provider IDs (OpenAI, Anthropic)
/// are not necessarily UUIDs — we accept whatever shape the wire uses.
macro_rules! string_id {
    ($name:ident, $prefix:literal) => {
        #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            /// Generate a fresh ID with the crate's canonical prefix.
            pub fn new() -> Self {
                Self(format!("{}_{}", $prefix, Uuid::new_v4().simple()))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<String> for $name {
            fn from(v: String) -> Self {
                Self(v)
            }
        }

        impl From<&str> for $name {
            fn from(v: &str) -> Self {
                Self(v.to_owned())
            }
        }
    };
}

string_id!(SessionId, "sess");
string_id!(MessageId, "msg");
string_id!(ToolCallId, "call");
