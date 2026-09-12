//! Built-in tools.
//!
//! Each tool lives in its own module for readability and so downstream users
//! can pick and choose (they aren't forced into a "kitchen sink" registration).
//! [`register_default`] wires the whole set for a typical agent.

pub mod bash;
pub mod edit;
pub mod find_callers;
pub mod find_references;
pub mod find_symbol;
pub mod git;
pub mod glob;
pub mod grep;
pub mod memory;
pub mod read;
pub mod rustfmt;
pub mod skill;
pub mod task_tools;
pub mod web_fetch;
pub mod web_search;
pub mod write;

use crate::registry::Registry;

/// Register every built-in tool. Callers who want a subset should register
/// individually instead.
pub fn register_default(reg: &mut Registry) {
    register_core(reg);
    register_memory(reg);
}

/// Register just the non-memory built-ins. Callers wire memory separately
/// so they can honor a `memory.tools_enabled: false` opt-out without
/// forking the whole default list.
pub fn register_core(reg: &mut Registry) {
    reg.register(read::ReadFile);
    reg.register(write::WriteFile);
    reg.register(edit::EditFile);
    reg.register(bash::Bash);
    reg.register(grep::Grep);
    reg.register(glob::Glob);
    reg.register(find_symbol::FindSymbol);
    reg.register(find_references::FindReferences);
    reg.register(find_callers::FindCallers);
    reg.register(git::GitStatus);
    reg.register(git::GitDiff);
    reg.register(git::GitLog);
    reg.register(git::GitCommit);
    reg.register(web_search::WebSearch);
    reg.register(web_fetch::WebFetch);
    reg.register(rustfmt::RustFmt);
    reg.register(task_tools::TaskCreate);
    reg.register(task_tools::TaskList);
    reg.register(task_tools::TaskGet);
    reg.register(task_tools::TaskUpdateTool);
}

/// Register the memory tools. Split out so callers can gate on the
/// `memory.tools_enabled` config knob — useful as a bisect switch when
/// diagnosing whether the extra tools are distracting the model.
pub fn register_memory(reg: &mut Registry) {
    reg.register(memory::MemoryRead);
    reg.register(memory::MemorySearch);
    reg.register(memory::MemoryAppend);
    reg.register(memory::MemoryEdit);
    reg.register(memory::MemoryRemember);
}

/// Register the `skill` tool with a shared handle to the loaded skill
/// registry. Split out so callers that don't want the surface (e.g.
/// tests, headless CLI runs with no skill dir) can skip it. Requires
/// [`SkillHandle`] — see the `mira-server` boot path for how the
/// registry is loaded from `~/.mira/skills` + `<cwd>/.mira/skills`.
pub fn register_skills(reg: &mut Registry, skills: skill::SkillHandle) {
    reg.register(skill::SkillTool::new(skills));
}

/// Register the `memory_consolidate` tool. Constructed with a
/// [`ChatProvider`](mira_ai::ChatProvider) handle + model at boot —
/// the tool makes an out-of-band provider call, and the standard
/// [`ToolContext`](crate::ToolContext) doesn't carry a provider. Same
/// pattern the server-side `AgentTool` uses.
pub fn register_consolidate(
    reg: &mut Registry,
    provider: std::sync::Arc<dyn mira_ai::ChatProvider>,
    model: String,
) {
    reg.register(memory::MemoryConsolidate::new(provider, model));
}
