//! Built-in tools.
//!
//! Each tool lives in its own module for readability and so downstream users
//! can pick and choose (they aren't forced into a "kitchen sink" registration).
//! [`register_default`] wires the whole set for a typical agent.

pub mod apply_patch;
pub mod ast_grep;
pub mod background;
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
pub mod repo_map;
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
    reg.register(repo_map::RepoMapTool);
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

/// Register the background-process tools (`run_background`, `read_output`,
/// `kill_background`). Callers wire this separately so the store can be
/// shared with the session's `ToolContext` before any tool calls fire.
pub fn register_background(reg: &mut Registry) {
    reg.register(background::RunBackground);
    reg.register(background::ReadOutput);
    reg.register(background::KillBackground);
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
    /// `computer` registered, with the backend name (`x11`, `macos`, `cua`).
    pub computer: Option<&'static str>,
    pub browser: bool,
    /// Why a requested tool was skipped (unsupported platform, …).
    pub warnings: Vec<String>,
}

/// Register the `computer` and/or `browser` tools, per `enable_*`.
///
/// `computer_cfg.cua` selects the remote backend: when it is set, the
/// `computer` tool drives that machine through cua-spacesd instead of the
/// desktop Mira runs on. Otherwise [`mira_computer::detect`] picks the local
/// one.
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
    cua_delivery: Option<mira_computer::Delivery>,
    enable_browser: bool,
    browser_cfg: &mira_config::BrowserConfig,
) -> ComputerUseReport {
    use std::sync::Arc;
    let mut report = ComputerUseReport::default();

    if enable_computer && computer_cfg.paired.is_some() {
        match computer_controller(computer_cfg, cua_delivery).await {
            Ok(controller) => {
                reg.register(computer::ComputerTool::new(controller, None));
                report.computer = Some("paired");
            }
            Err(e) => report.warnings.push(format!("computer use disabled: {e}")),
        }
    } else if enable_computer {
        match computer_backend(computer_cfg, cua_delivery).await {
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
                // The process-wide browser for this profile: the browser
                // pane and agents drive the same one, so they can never
                // fight over Chrome's one-process-per-profile lock.
                reg.register(browser::BrowserTool::new(mira_browser::Browser::shared(
                    opts,
                )));
                report.browser = true;
            }
            Err(e) => report.warnings.push(format!("browser tool disabled: {e}")),
        }
    }
    report
}

/// Pick the backend the `computer` tool drives.
///
/// `computer_cfg.cua` wins when present: it names a machine other than this
/// one, and asking the local platform what it can do would answer a question
/// nobody asked. A failure to reach the remote daemon is reported rather than
/// quietly falling back to the local desktop — silently clicking on the
/// user's real screen because a remote box was down is not a fallback anyone
/// wants.
pub async fn computer_backend(
    cfg: &mira_config::ComputerUseConfig,
    delivery: Option<mira_computer::Delivery>,
) -> Result<Box<dyn mira_computer::ComputerBackend>, mira_computer::ComputerError> {
    let delivery = resolved_delivery(cfg, delivery)?;
    let Some(opts) = cua_options(cfg, Some(delivery)) else {
        if delivery == mira_computer::Delivery::Background {
            return Err(mira_computer::ComputerError::Unsupported("The native desktop backend cannot promise background delivery. Use CUA or explicitly select Foreground.".into()));
        }
        return mira_computer::detect();
    };
    let endpoint = opts.endpoint.clone();
    let delivery = opts.delivery;
    let backend = mira_computer::CuaBackend::connect(&opts).await?;

    // Worth one round trip at startup: a guest that cannot take background
    // input should be said out loud before the agent starts clicking, not
    // discovered on the first click that moves the user's pointer.
    match backend.capabilities().await {
        Ok(caps) if caps.protocol_version != 1 => {
            return Err(mira_computer::ComputerError::Unsupported(
                "Unsupported CUA protocol version".into(),
            ))
        }
        Ok(caps) if !caps.background_input && delivery == mira_computer::Delivery::Background => {
            return Err(mira_computer::ComputerError::Unsupported(format!(
                "{endpoint} cannot deliver background input: {}",
                caps.background_input_limitation
            )))
        }
        Ok(_) => {}
        Err(e) => return Err(e),
    }

    Ok(Box::new(backend))
}

/// Options for the remote backend, or `None` when `computer.cua` does not
/// select it and the local backend should be used instead.
///
/// One place, so registration and `mira doctor` cannot disagree about which
/// machine the `computer` tool is pointed at.
pub fn cua_options(
    cfg: &mira_config::ComputerUseConfig,
    delivery: Option<mira_computer::Delivery>,
) -> Option<mira_computer::CuaOptions> {
    let cua = cfg.cua.as_ref().filter(|c| c.is_set())?;
    Some(
        mira_computer::CuaOptions {
            endpoint: cua.endpoint.clone()?,
            token: None,
            delivery: delivery
                .or_else(|| cfg.delivery.as_ref().and_then(|v| v.parse().ok()))
                .unwrap_or_else(delivery_from_env),
            window: cfg
                .window
                .as_ref()
                .map(|w| mira_computer::backend::cua::TargetWindow {
                    id: w.id.clone(),
                    epoch: w.epoch,
                    title: w.title.clone(),
                    app: w.app.clone(),
                }),
            ..Default::default()
        }
        .token_from_env(cua.token_env()),
    )
}

/// Readiness checks for whichever backend the `computer` tool would use.
pub async fn computer_preflight(
    cfg: &mira_config::ComputerUseConfig,
    delivery: Option<mira_computer::Delivery>,
) -> Vec<mira_computer::Check> {
    match cua_options(cfg, delivery) {
        Some(opts) => mira_computer::backend::cua::preflight(&opts).await,
        None => mira_computer::preflight().await,
    }
}

/// Delivery mode when neither `--cua-delivery` nor the caller said:
/// `MIRA_CUA_DELIVERY`, else [`mira_computer::Delivery::Auto`].
///
/// A run-time flag rather than a setting in `mira.yaml` because whether an
/// agent may take your pointer is a decision about right now, not about the
/// project.
fn delivery_from_env() -> mira_computer::Delivery {
    match std::env::var("MIRA_CUA_DELIVERY") {
        Ok(v) => match v.parse() {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!("MIRA_CUA_DELIVERY: {e}; using background only");
                mira_computer::Delivery::Background
            }
        },
        Err(_) => mira_computer::Delivery::Auto,
    }
}

/// Same target selection for CLI tools and the live Settings source. Never fall back from a paired target.
pub async fn computer_controller(
    cfg: &mira_config::ComputerUseConfig,
    delivery: Option<mira_computer::Delivery>,
) -> Result<std::sync::Arc<dyn mira_computer::ComputerController>, mira_computer::ComputerError> {
    let delivery = resolved_delivery(cfg, delivery)?;
    let window = cfg
        .window
        .as_ref()
        .map(|w| mira_computer::backend::cua::TargetWindow {
            id: w.id.clone(),
            epoch: w.epoch,
            title: w.title.clone(),
            app: w.app.clone(),
        });
    if let Some(id) = &cfg.paired {
        let connection = mira_computer::remote::load_connections()?
            .into_iter()
            .find(|c| &c.id == id)
            .ok_or_else(|| {
                mira_computer::ComputerError::Remote(
                    "The selected computer is no longer paired. Choose a computer in Settings."
                        .into(),
                )
            })?;
        return Ok(mira_computer::remote::RemoteComputer::new(
            connection, delivery, window,
        )?);
    }
    let backend = computer_backend(cfg, Some(delivery)).await?;
    let mut options = mira_computer::ComputerOptions {
        screenshot_after_action: cfg.screenshot_after_action.unwrap_or(true),
        settle: std::time::Duration::from_millis(cfg.settle_ms.unwrap_or(600).min(10_000)),
        ..Default::default()
    };
    options.limits.max_long_edge = cfg.max_long_edge.unwrap_or(1568).clamp(256, 4096);
    Ok(std::sync::Arc::new(mira_computer::Computer::new(
        std::sync::Arc::from(backend),
        options,
    )))
}

fn resolved_delivery(
    cfg: &mira_config::ComputerUseConfig,
    explicit: Option<mira_computer::Delivery>,
) -> Result<mira_computer::Delivery, mira_computer::ComputerError> {
    if let Some(delivery) = explicit {
        return Ok(delivery);
    }
    if let Some(delivery) = &cfg.delivery {
        return delivery
            .parse()
            .map_err(mira_computer::ComputerError::InvalidArgs);
    }
    Ok(delivery_from_env())
}
#[cfg(test)]
mod computer_settings_tests {
    use super::*;
    #[tokio::test]
    async fn background_delivery_never_silently_uses_native_input() {
        let result = computer_backend(
            &Default::default(),
            Some(mira_computer::Delivery::Background),
        )
        .await;
        assert!(result.is_err());
    }
    #[test]
    fn invalid_delivery_configuration_is_rejected() {
        let cfg = mira_config::ComputerUseConfig {
            delivery: Some("backgound".into()),
            ..Default::default()
        };
        assert!(resolved_delivery(&cfg, None).is_err());
    }
}
