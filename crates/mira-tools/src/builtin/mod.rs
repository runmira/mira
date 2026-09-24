//! Built-in tools.
//!
//! Each tool lives in its own module for readability and so downstream users
//! can pick and choose (they aren't forced into a "kitchen sink" registration).
//! [`register_default`] wires the whole set for a typical agent.

pub mod apply_patch;
pub mod ast_grep;
pub mod bash;
pub mod browser;
pub mod computer;
pub mod edit;
pub mod file_outline;
pub mod find_callers;
pub mod find_references;
pub mod find_symbol;
pub mod git;
pub mod glob;
pub mod grep;
pub mod memory;
pub mod read;
pub(crate) mod rg;
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
    reg.register(apply_patch::ApplyPatch);
    reg.register(bash::Bash);
    reg.register(grep::Grep);
    reg.register(glob::Glob);
    reg.register(find_symbol::FindSymbol);
    reg.register(find_references::FindReferences);
    reg.register(find_callers::FindCallers);
    reg.register(ast_grep::AstGrep);
    reg.register(file_outline::FileOutline);
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

/// What [`register_computer_use`] did, for the caller to surface.
#[derive(Debug, Default)]
pub struct ComputerUseReport {
    /// `computer` registered, with the backend name (`x11`, `macos`).
    pub computer: Option<&'static str>,
    pub browser: bool,
    /// Why a requested tool was skipped (unsupported platform, …).
    pub warnings: Vec<String>,
}

/// Register the `computer` and/or `browser` tools, per `enable_*`.
///
/// Callers register these *after* snapshotting the registry subagents
/// inherit: desktop and browser control stay with the top-level session,
/// where the user is watching and approving.
///
/// Never fails: an unavailable backend becomes a warning in the report,
/// so a missing `xdotool` can't stop Mira from starting.
pub async fn register_computer_use(
    reg: &mut Registry,
    enable_computer: bool,
    computer_cfg: &mira_config::ComputerUseConfig,
    enable_browser: bool,
    browser_cfg: &mira_config::BrowserConfig,
) -> ComputerUseReport {
    use std::sync::Arc;
    let mut report = ComputerUseReport::default();

    if enable_computer {
        match mira_computer::detect() {
            Ok(backend) => {
                let mut opts = mira_computer::ComputerOptions::default();
                if let Some(v) = computer_cfg.screenshot_after_action {
                    opts.screenshot_after_action = v;
                }
                if let Some(edge) = computer_cfg.max_long_edge {
                    opts.limits.max_long_edge = edge.clamp(256, 4096);
                }
                if let Some(ms) = computer_cfg.settle_ms {
                    opts.settle = std::time::Duration::from_millis(ms.min(10_000));
                }
                let name = backend.name();
                let computer = Arc::new(mira_computer::Computer::new(Arc::from(backend), opts));
                // Best effort: the size only improves the description.
                let display = match computer.display_size().await {
                    Ok(d) => Some(d),
                    Err(e) => {
                        report
                            .warnings
                            .push(format!("computer: couldn't read the display size ({e})"));
                        None
                    }
                };
                reg.register(computer::ComputerTool::new(computer, display));
                report.computer = Some(name);
            }
            Err(e) => report.warnings.push(format!("computer use disabled: {e}")),
        }
    }

    if enable_browser {
        let mut opts = mira_browser::BrowserOptions::default();
        if let Some(exe) = browser_cfg.executable_path() {
            opts.executable = Some(exe);
        }
        if let Some(h) = browser_cfg.headless {
            opts.headless = h;
        }
        if let Some(dir) = browser_cfg.profile_dir_path() {
            opts.profile_dir = dir;
        }
        // Resolve the executable now so a missing browser is reported at
        // startup rather than on the model's first call.
        match mira_browser::launch::find_executable(opts.executable.as_deref()) {
            Ok(_) => {
                reg.register(browser::BrowserTool::new(Arc::new(
                    mira_browser::Browser::new(opts),
                )));
                report.browser = true;
            }
            Err(e) => report.warnings.push(format!("browser tool disabled: {e}")),
        }
    }
    report
}
