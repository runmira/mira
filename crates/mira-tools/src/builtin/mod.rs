//! Built-in tools.
//!
//! Each tool lives in its own module for readability and so downstream users
//! can pick and choose (they aren't forced into a "kitchen sink" registration).
//! [`register_default`] wires the whole set for a typical agent.

pub mod bash;
pub mod edit;
pub mod git;
pub mod glob;
pub mod grep;
pub mod read;
pub mod rustfmt;
pub mod write;

use crate::registry::Registry;

/// Register every built-in tool. Callers who want a subset should register
/// individually instead.
pub fn register_default(reg: &mut Registry) {
    reg.register(read::ReadFile);
    reg.register(write::WriteFile);
    reg.register(edit::EditFile);
    reg.register(bash::Bash);
    reg.register(grep::Grep);
    reg.register(glob::Glob);
    reg.register(git::GitStatus);
    reg.register(git::GitDiff);
    reg.register(git::GitLog);
    reg.register(git::GitCommit);
    reg.register(rustfmt::RustFmt);
}
