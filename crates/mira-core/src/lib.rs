//! Shared vocabulary for the Mira harness.
//!
//! Everything downstream — providers, tools, the loop, the CLI — speaks the
//! types in this crate. Keep it small and free of runtime dependencies so it
//! stays cheap to depend on.

pub mod error;
pub mod id;
pub mod message;

pub use error::{Error, Result};
pub use id::{MessageId, SessionId, ToolCallId};
pub use message::{Message, Role, ToolCall, ToolResult};
