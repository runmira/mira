//! Tool trait, registry, and built-ins.
//!
//! A [`Tool`] is a named, self-describing capability the model can invoke.
//! Tools are grouped in a [`Registry`], which the harness consults on every
//! turn to (a) produce the `tools` array sent to the model and (b) dispatch
//! incoming tool calls.
//!
//! Extending Mira with a new capability is normally three lines: implement
//! `Tool`, register it, done. See `builtin::read::ReadFile` for the canonical
//! example.

pub mod builtin;
pub mod context;
pub mod guard;
pub mod mcp;
pub mod preview;
pub mod registry;
pub mod tasks;
pub mod tool;

pub use context::ToolContext;
pub use guard::{AppliedUndo, FileGuard, GuardError, UndoEntry, UndoOp};
pub use mcp::{connect as connect_mcp, McpConnection, McpTool};
pub use preview::{compute_preview, DiffKind, DiffLine, DiffPreview};
pub use registry::Registry;
pub use tasks::{TaskItem, TaskStatus, TaskStore, TaskUpdate};
pub use tool::{Action, Tool, ToolError};
