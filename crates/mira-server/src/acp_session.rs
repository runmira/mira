//! Bringing an external ACP agent up inside a Mira session.
//!
//! This is the piece that turns the standalone `mira-acp` runtime into
//! something a chat can actually use: it resolves a driver, binds the host
//! ports to *this* session's sandbox, policy and approver, spawns the agent,
//! and keeps it alive for the life of the slot.
//!
//! Ownership is the important part. An agent process is long-lived and holds
//! real authority — filesystem, terminals, permission prompts — so it is
//! owned by the [`SessionSlot`] and torn down with it. A dropped session
//! takes its agent and its terminals with it, rather than leaving a
//! third-party process running with no owner.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use mira_acp::driver::{AcpDriver, DriverConfig, LaunchConfig, PermissionMode};
use mira_acp::native::AgentHandle;
use mira_acp::process::{start_agent as start_acp_agent, StartError};
use mira_pty::PtyRegistry;
use tokio::sync::RwLock;

use crate::acp_host::{AcpEventPort, AcpFiles, AcpPermissions, AcpTerminals};
use crate::protocol::ServerMsg;
use crate::slot::SessionSlot;
use crate::state::AppState;
use mira_acp::driver::Transport;
use mira_policy::Decision;

/// How large a single ACP file read may be.
const MAX_ACP_READ_BYTES: usize = 1024 * 1024;

/// Claude Code's switch for loading MCP tool definitions on demand.
const CLAUDE_TOOL_SEARCH_ENV: &str = "ENABLE_TOOL_SEARCH";

/// The agent a slot is currently driving, if any.
pub struct SlotAgent {
    pub opencode_control: Option<crate::opencode_control::OpenCodeControl>,
    lease: std::sync::Mutex<Option<crate::runtime_admission::RuntimeLease>>,
    /// The driver it was started from, for diagnostics and restart.
    pub driver_kind: String,
    /// The engine instance it runs as. Cursor records, spend rows and
    /// engine frames route by this, not by kind.
    pub instance: String,
    /// The driver's own name, for user-facing messages.
    pub display_name: String,
    /// The resolved launch, with secrets redacted — safe to log.
    pub launch: String,
    /// Where this agent's prompt attachments are staged, when the transport
    /// stages one. Images sent with a prompt are written here and named in
    /// the prompt text, because the agent can only open what it may read.
    pub agent_files_dir: Option<std::path::PathBuf>,
    agent: RwLock<Option<AgentHandle>>,
    terminals: Arc<AcpTerminals>,
    /// Set before an intentional stop, so the event pump can tell a
    /// restart (mode change, model change, switch to a provider) from a
    /// crash. Without it every restart announced "the external agent
    /// stopped unexpectedly" and ended a turn that was never running —
    /// which the client read as the agent dying and dropped out of agent
    /// mode, racing the replacement's own start.
    stopping: Arc<AtomicBool>,
}

impl SlotAgent {
    /// Why `mode_id` is privileged, or `None` if it is not.
    ///
    /// The driver is re-resolved from its kind rather than stored: every
    /// driver is a stateless unit struct, so a fresh one costs nothing and
    /// avoids holding a trait object for the life of the session.
    pub fn privileged_mode_reason(&self, mode_id: &str) -> Option<&'static str> {
        let driver = mira_acp::drivers::by_kind(&self.driver_kind)?;
        if !driver.privileged_mode_ids().contains(&mode_id) {
            return None;
        }
        Some(
            driver
                .privileged_mode_reason(mode_id)
                .unwrap_or("This mode grants the agent more access than Mira would."),
        )
    }

    pub async fn agent(&self) -> Option<AgentHandle> {
        self.agent.read().await.clone()
    }

    /// Best-effort stop of the agent's *current turn* — not the process.
    /// The agent announces the interrupted end itself through its event
    /// pump, so callers must not also emit a turn-end frame.
    pub async fn cancel_current_turn(&self) {
        if let Some(agent) = self.agent().await {
            if self.driver_kind == "grok" {
                // Grok's soft interrupt leaves background subagents alive.
                // Explicit Stop owns the entire runtime and its terminals.
                self.stop().await;
            } else {
                let _ = agent.cancel().await;
            }
        }
    }

    pub async fn stop(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        // Answer any permission the agent is blocked on *before* killing the
        // process, so it is not left waiting on a request that will never
        // arrive. The spec makes this a MUST on cancel.
        if let Some(agent) = self.agent.write().await.take() {
            agent.shutdown().await;
        }
        // And take its terminals down: they are separate processes.
        self.terminals.shutdown();
        self.lease.lock().unwrap_or_else(|e| e.into_inner()).take();
    }
}

/// The slot's agent, held behind a lock so it can be swapped or stopped.
pub type AgentSlot = RwLock<Option<Arc<SlotAgent>>>;

/// Start the agent `params` describes for `slot`, replacing any agent
/// already running.
///
/// Callers hold the slot's engine start lock (see
/// [`crate::session_engine::ensure_agent`]); this does not take it, so a
/// restart can run under a lock its caller already holds.
///
/// Fails closed in two ways worth stating: the driver must resolve to a
/// binary that exists (so the user gets "grok isn't installed" rather than a
/// hang), and the ports are built from *this* slot's `ToolContext` and
/// approver, so the agent inherits the session's existing confinement rather
/// than a fresh one.
pub async fn start_agent(
    state: &AppState,
    slot: &Arc<SessionSlot>,
    params: &AcpLaunchParams,
    native: Option<mira_acp::native::NativeOverrides>,
) -> Result<Arc<SlotAgent>, String> {
    let driver = mira_acp::drivers::by_kind(&params.driver_kind)
        .ok_or_else(|| format!("unknown agent: {}", params.driver_kind))?;
    let driver = driver.as_ref();
    let mut driver_cfg = params.cfg.clone();
    // Secrets are never written to a session record, so a chat restored
    // after a restart comes back without its key. The engine's config in
    // mira.yaml is the one place they live; fill in what's missing from it.
    if let Some(current) = state
        .engines
        .current()
        .external_driver_config(&params.instance)
    {
        if driver_cfg.api_key.is_none() {
            driver_cfg.api_key = current.api_key;
        }
        for (k, v) in current.env {
            driver_cfg.env.entry(k).or_insert(v);
        }
    }
    let mode = params.mode;
    // Where the binary lives. `binary_path` in the driver config wins;
    // otherwise the driver's own name on PATH.
    let program: PathBuf = driver_cfg
        .binary_path
        .clone()
        .unwrap_or_else(|| PathBuf::from(driver.binary_names().first().copied().unwrap_or("")));

    // The binary that will actually be run, which is not necessarily the ACP
    // one: with a native transport there is no adapter involved, and checking
    // for one produced `Claude Code is not installed (looked for
    // claude-agent-acp)` for a user whose Claude Code was installed and
    // working. Decide the transport first, then validate what it will spawn.
    let transport = mira_acp::process::choose_transport(driver, &driver_cfg, &program);
    let mut native = native.unwrap_or_default();
    if matches!(transport, Transport::Native) {
        // The session's standing choices apply to every launch, so a mode
        // change does not quietly drop the model picked earlier, and vice
        // versa. Explicit overrides still win.
        if native.model.is_none() {
            native.model = params.model.clone();
        }
        if native.permission_mode.is_none() {
            native.permission_mode = params.mode_id.clone();
        }
    }
    // A slot with a recorded cursor resumes it: the agent session
    // outlives Mira restarts this way instead of starting blank each
    // time. Explicit overrides (import resume, fork) always win. This is
    // transport-agnostic: native CLIs take `--resume`, ACP agents take
    // `session/resume` (OpenCode advertises it) via the same field.
    if native.resume.is_none() {
        if let Some(cur) = mira_acp::agent_sessions::read(&slot.id.to_string(), &params.instance) {
            native.resume = Some(cur.agent_session_id);
        }
    }
    // Mira's browser, as an MCP server: Claude Code drives the same browser
    // the pane shows (and Mira's own model uses), instead of having none or
    // starting its own. Calls still go through the agent's permission gate,
    // which is Mira's approval card.
    if matches!(transport, Transport::Native) && driver.kind() == "claude-code" {
        driver_cfg.launch_args.push("--mcp-config".into());
        driver_cfg.launch_args.push(
            crate::browser::agent_mcp_config(state.local_port, &slot.id.to_string()).to_string(),
        );
        // Looking (snapshots, screenshots, process output) doesn't need an
        // approval each time; anything that acts still asks.
        driver_cfg.launch_args.push(format!(
            "--allowedTools={}",
            crate::browser::claude_allowed_tools()
        ));
    }
    // Claude Code's tool search: MCP tool definitions (Mira's browser,
    // process and thread tools, and the user's own servers) load when a
    // task needs them instead of riding on every request. Left alone if
    // the user set it either way.
    let claude_tool_search = matches!(transport, Transport::Native)
        && driver.kind() == "claude-code"
        && std::env::var_os(CLAUDE_TOOL_SEARCH_ENV).is_none();
    let mut launch: LaunchConfig = match transport {
        Transport::Native => {
            let mut l =
                mira_acp::process::build_native_launch(driver, &driver_cfg, mode, &program, None)
                    .expect("native transport implies a launch");
            // Keep the driver's own env and args; only the program differs.
            let acp = driver.resolve(&driver_cfg, mode, program.clone());
            l.env = acp.env;
            l.secret_env = acp.secret_env;
            l.args.extend(driver_cfg.launch_args.clone());
            if claude_tool_search && !l.env.contains_key(CLAUDE_TOOL_SEARCH_ENV) {
                l.env.insert(CLAUDE_TOOL_SEARCH_ENV.into(), "true".into());
            }
            l
        }
        _ => driver.resolve(&driver_cfg, mode, program.clone()),
    };
    let opencode_control = if driver.kind() == "opencode"
        && matches!(transport, Transport::Acp)
        && crate::opencode_control::OpenCodeControl::supported(&launch).await
    {
        Some(crate::opencode_control::OpenCodeControl::configure(
            &mut launch,
        )?)
    } else {
        None
    };
    if !mira_acp::process::looks_installed(&launch) {
        return Err(format!(
            "{} is not installed (looked for `{}`)",
            driver.display_name(),
            launch.program.display()
        ));
    }

    // The session's own root and context. `cwd` is read here rather than
    // captured earlier so a session that changed folder gets a matching
    // boundary.
    let repo_root = slot.cwd.read().await.clone();
    // The session's own context and the process-wide sandbox, so the agent
    // inherits the confinement Mira already applies to its own tools.
    let tool_ctx = slot.make_tool_ctx(state.sandbox.clone()).await;

    let registry = Arc::new(PtyRegistry::new());
    let terminals = AcpTerminals::new(
        slot.id.to_string(),
        repo_root.clone(),
        registry.clone(),
        /* sandboxed */ true,
    );
    let files = AcpFiles::new(tool_ctx, slot.approver.clone(), MAX_ACP_READ_BYTES);
    let permissions = AcpPermissions::for_slot(slot);
    // Spend reports are booked to the user's ledger, against this chat, so
    // the Usage page counts agent turns next to Mira's own.
    let events = match crate::agent_spend::SpendLedger::user() {
        Some(ledger) => AcpEventPort::with_spend(
            slot.events_tx.clone(),
            ledger,
            crate::agent_spend::Spender {
                session_id: slot.id.to_string(),
                cwd: repo_root.to_string_lossy().into_owned(),
                driver: params.driver_kind.clone(),
                instance: params.instance.clone(),
                fallback_model: params
                    .model
                    .clone()
                    .unwrap_or_else(|| params.driver_kind.clone()),
                // Decided now, from what we know before the agent starts.
                // A configured key is decisive; otherwise the plan the
                // agent advertises over ACP decides, and anything we can't
                // read stays unknown rather than being assumed.
                billing: mira_acp::status::Billing::classify(params.cfg.api_key.is_some(), &[]),
            },
        ),
        None => AcpEventPort::new(slot.events_tx.clone()),
    };
    events.set_driver(&params.driver_kind);

    // Restored native modes also configure the host gate: older sessions
    // persisted the runtime picker independently of agent_approval_mode.
    if params.driver_kind == "codex" && matches!(transport, Transport::Native) {
        let runtime = native
            .permission_mode
            .as_deref()
            .unwrap_or_else(|| mira_acp::appserver::mode_for_permission(params.mode));
        if let Some(mode) = codex_approval_mode(runtime) {
            set_agent_approval_mode(slot, mode).await;
        }
    }

    // A native agent has no ACP ports to bind — it runs in its own process and
    // asks the host its questions through one gate. Tool permissions answer
    // through the same approver as ACP, so the approval UI is identical
    // either way; the agent's own questions and plans get Mira's cards.
    let gate: mira_acp::native::PermissionGate = {
        let weak = Arc::downgrade(slot);
        Arc::new(move |p| {
            let weak = weak.clone();
            Box::pin(async move {
                match weak.upgrade() {
                    Some(slot) => decide_native_permission(&slot, p).await,
                    None => mira_acp::native::PermissionDecision {
                        allow: false,
                        message: Some("the Mira session closed".into()),
                        ..Default::default()
                    },
                }
            })
        })
    };

    // Stop whatever runs now *before* spawning the replacement, so two
    // processes never hold this session's permissions at once and the old
    // one's exit is recorded as intentional.
    stop_agent(slot).await;
    let lease = crate::runtime_admission::AGENT_ADMISSION.acquire(&params.driver_kind)?;
    events.bind_runtime(slot, &params.instance, &params.driver_kind);

    // Native transports always need their overrides (model, mode, resume);
    // ACP transports only need the resume cursor — `session/resume` rather
    // than `session/new`. Pass it through whenever one is set so an idle
    // reaper restart or a Mira restart re-adopts the same agent session.
    let native_overrides = if matches!(transport, Transport::Native) || native.resume.is_some() {
        Some(native)
    } else {
        None
    };
    let agent = match start_acp_agent_with_ports(
        driver,
        &driver_cfg,
        mode,
        &program,
        files,
        terminals.clone(),
        permissions,
        events.clone(),
        &repo_root,
        gate,
        native_overrides,
        // Mira's tools for ACP agents (OpenCode, Gemini…): they may call MCP
        // tools without asking, so Mira gates the ones that run commands.
        Some(crate::browser::agent_mcp(
            state.local_port,
            &slot.id.to_string(),
            crate::browser::McpGate::Mira,
        )),
    )
    .await
    {
        Ok(a) => a,
        Err(e) => return Err(describe(&e)),
    };

    let stopping = Arc::new(AtomicBool::new(false));

    // How this agent's spend is billed, now that the agent is up and has
    // told us how it authenticates. ACP agents advertise their methods;
    // the native transports don't, so those fall back to the last probe's
    // summary of who the CLI is signed in as. A configured key outranks
    // both, and `refine_billing` won't downgrade past it.
    let has_key = driver_cfg.api_key.as_ref().is_some_and(|k| !k.is_empty());
    let advertised = agent.advertised_auth_method_ids();
    let billing = if !advertised.is_empty() {
        mira_acp::status::Billing::classify(has_key, &advertised)
    } else {
        crate::engines_api::cached_auth_summary(&params.instance)
            .map(|summary| {
                if has_key {
                    mira_acp::status::Billing::Api
                } else {
                    mira_acp::status::Billing::from_auth_summary(&summary)
                }
            })
            .unwrap_or(mira_acp::status::Billing::classify(has_key, &[]))
    };
    events.refine_billing(billing);

    // Native agents deliver events on their own channels rather than through
    // the ACP host, so forward both onto the same wire the client already
    // reads. Without this the transcript stays empty and the turn never ends.
    // Both native transports expose the same pair, so the pump does not care
    // which one is live.
    if let Some((mut evs, mut ends)) = agent.take_event_channels() {
        let port = events.clone();
        let warn = slot.events_tx.clone();
        let agent_handle = agent.clone();
        let slot_id = slot.id.to_string();
        let instance = params.instance.clone();
        let stopping = stopping.clone();
        let weak_slot = Arc::downgrade(slot);
        let state_c = state.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    biased;
                    Some(e) = evs.recv() => {
                        if stopping.load(Ordering::SeqCst) { break; }
                        if let Some(slot) = weak_slot.upgrade() {
                            if let mira_acp::events::MiraEvent::Limits { windows } = &e.event {
                                *slot.message_queue.limit_reset.lock().unwrap_or_else(|e| e.into_inner()) = windows.iter().filter(|w| w.utilization >= 1.0).filter_map(|w| w.resets_at).filter(|at| *at > 0).max().map(|at| (at as u64).saturating_mul(1000));
                            }
                        }
                        port.emit(e).await;
                    },
                    Some(end) = ends.recv() => {
                        if stopping.load(Ordering::SeqCst) { break; }
                        let end_owner = weak_slot.upgrade();
                        let _end_dispatch = match &end_owner { Some(slot) => Some(slot.engine.input_dispatch.lock().await), None => None };
                        if let Some(slot) = &end_owner {
                            slot.engine.agent_in_turn.store(false, Ordering::SeqCst);
                            slot.publish_activity();
                        }
                        // A usage limit is a real outcome, not a crash. Its
                        // context rides on the turn-end frame itself: sending
                        // a warning as well produced two transcript lines for
                        // one fact, and the pair read as if two things failed.
                        if end.stop_reason == "rate_limited" {
                            if let Some(slot) = weak_slot.upgrade() { crate::message_queue::offer_recovery(&slot, end.rate_limited.as_deref()).await; }
                        }
                        match end.rate_limited.clone() {
                            Some(reset) => {
                                let _ = warn.send(ServerMsg::acp_turn_end_with(
                                    &end.stop_reason,
                                    format!("resets {reset}"),
                                ));
                            }
                            None => port.turn_ended(&end.stop_reason),
                        }
                        // The turn ended with the agent alive, so its session
                        // id is a valid resume cursor. Recorded every turn —
                        // cheap, and always current when Mira restarts.
                        if let Some(sid) = agent_handle.session_id().await {
                            mira_acp::agent_sessions::record(
                                &slot_id,
                                &instance,
                                &sid,
                            );
                        }
                    }
                    else => {
                        // Stopped on purpose (restart, switch, delete): the
                        // replacement or the new engine speaks for itself.
                        if stopping.load(Ordering::SeqCst) {
                            break;
                        }
                        // The agent's stream closed on its own. If it closed
                        // without a `result` the turn will never be
                        // announced, so end it rather than leave the
                        // composer spinning — and drop the dead handle so the
                        // next prompt restarts the agent (resuming its
                        // session) instead of writing into a closed pipe.
                        let _ = warn.send(ServerMsg::Warning {
                            text: "the external agent exited — your next message restarts it"
                                .to_string(),
                        });
                        port.turn_ended("agent_exited");
                        if let Some(slot) = weak_slot.upgrade() {
                            let mut current = slot.acp_agent.write().await;
                            let is_ours = current
                                .as_ref()
                                .is_some_and(|h| Arc::ptr_eq(&h.stopping, &stopping));
                            if is_ours {
                                if let Some(h) = current.take() {
                                    h.terminals.shutdown();
                                }
                            }
                            drop(current);
                            crate::session_engine::publish(&state_c, &slot).await;
                        }
                        break;
                    }
                }
            }
        });
    }

    let agent_files_dir = agent.attachments_dir();
    let handle = Arc::new(SlotAgent {
        opencode_control,
        lease: std::sync::Mutex::new(Some(lease)),
        driver_kind: driver.kind().to_string(),
        instance: params.instance.clone(),
        display_name: params.display_name(),
        launch: launch.redacted(),
        agent_files_dir,
        agent: RwLock::new(Some(agent)),
        terminals,
        stopping,
    });

    // Install it. Anything that slipped in since the stop above is stopped
    // too, so two agents never share a session's permissions.
    if let Some(prev) = slot.acp_agent.write().await.replace(handle.clone()) {
        prev.stop().await;
    }
    // ACP agents report their session id synchronously at `session/new`
    // (or keep it across `session/resume`), so record the cursor now:
    // the next start — after the idle reaper or a Mira restart — resumes
    // this same agent session instead of opening a blank one. Native
    // agents record per-turn in their event pump; ACP ids never change
    // mid-process, so once per start is enough.
    if let Some(agent) = handle.agent().await {
        if let Some(sid) = agent.session_id().await {
            mira_acp::agent_sessions::record(&slot.id.to_string(), &handle.instance, &sid);
        }
    }
    ensure_transcript_logger(state, slot);
    Ok(handle)
}

/// Subscribe the slot's transcript logger, once per slot.
///
/// A second subscriber on the session broadcast persists transcript-safe
/// frames to the sidecar. Native pumps and the ACP host both publish there,
/// so one logger covers every transport. It used to be spawned on every
/// agent start, so after a mode or model change each frame was written
/// twice and replay doubled every reply. It also watches the agent's
/// reported model, so the engine view names what is actually running.
fn ensure_transcript_logger(state: &AppState, slot: &Arc<SessionSlot>) {
    if slot.engine.logger_started.swap(true, Ordering::SeqCst) {
        return;
    }
    let path = state
        .store
        .as_ref()
        .and_then(|store| store.agent_log_path(&slot.id));
    let mut rx = slot.events_tx.subscribe();
    let weak_slot = Arc::downgrade(slot);
    let state = state.clone();
    tokio::spawn(async move {
        loop {
            let msg = match rx.recv().await {
                Ok(m) => m,
                // Falling behind loses frames, not the logger: before, one
                // burst ended persistence for the rest of the session.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(skipped = n, "agent transcript logger lagged");
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            let Some(slot) = weak_slot.upgrade() else {
                break;
            };
            if slot.engine.retired.load(Ordering::SeqCst) {
                slot.engine.logger_started.store(false, Ordering::SeqCst);
                break;
            }
            if let ServerMsg::Error { text } = &msg {
                *slot
                    .engine
                    .sidebar_failure
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = Some(text.chars().take(160).collect());
            }
            // A title the agent reports for its session becomes the row's
            // name — it knows what the conversation is about.
            if let ServerMsg::AcpSessionInfo {
                title: Some(title), ..
            } = &msg
            {
                let title = title.trim();
                if !title.is_empty() {
                    let session = slot.session.read().await.clone();
                    if session.title().await.as_deref() != Some(title) {
                        session.set_title(title).await;
                        state
                            .broadcast_all(ServerMsg::SessionTitleUpdated {
                                session_id: session.id.to_string(),
                                title: title.to_string(),
                            })
                            .await;
                    }
                }
            }
            let model_changed = {
                let launch = slot.acp_launch.lock().await;
                crate::session_engine::observe_frame(
                    &slot.engine,
                    &msg,
                    launch.as_ref().map(|params| params.driver_kind.as_str()),
                )
            };
            if model_changed {
                crate::session_engine::publish(&state, &slot).await;
            }
            if !is_transcript_frame(&msg) {
                continue;
            }
            slot.engine.touch();
            // Only while an agent is this session's engine: provider turns
            // live in the harness history, not the sidecar.
            let driver_kind = match slot.acp_launch.lock().await.as_ref() {
                Some(p) => p.driver_kind.clone(),
                None => continue,
            };
            let Some(path) = path.as_ref() else { continue };
            // Frames serialize to exactly what the client parses, so replay
            // feeds the same handler as live traffic.
            if let Ok(frame) = serde_json::to_value(&msg) {
                let line = serde_json::json!({
                    "t": mira_harness::persist::now_ms(),
                    "driver": driver_kind,
                    "frame": frame,
                });
                mira_acp::agent_sessions::append_line_to(path, &line);
            }
        }
    });
}

fn native_permission_call(
    p: &mira_acp::native::NativePermission,
    id: mira_core::ToolCallId,
) -> mira_core::ToolCall {
    let (name, arguments) = native_permission_tool_view(p);
    mira_core::ToolCall {
        id,
        kind: mira_core::ToolCallKind::Function,
        function: mira_core::ToolCallFunction {
            name,
            arguments: arguments.to_string(),
        },
    }
}

fn native_permission_tool_view(
    p: &mira_acp::native::NativePermission,
) -> (String, serde_json::Value) {
    let lower = p.tool_name.to_ascii_lowercase();
    if lower == "shell" || lower == "bash" {
        let command = p
            .input
            .get("command")
            .and_then(|v| v.as_str())
            .or_else(|| p.input.as_str())
            .or(p.reason.as_deref())
            .unwrap_or("")
            .trim()
            .to_string();
        if !command.is_empty() {
            return (
                "bash".to_string(),
                serde_json::json!({ "command": command }),
            );
        }
    }

    if lower == "apply_patch" || lower == "patch" || lower == "file_change" {
        let path = p
            .input
            .get("path")
            .or_else(|| p.input.get("file"))
            .or_else(|| p.input.get("filePath"))
            .or_else(|| p.input.get("file_path"))
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let patch = p
            .input
            .get("patch")
            .or_else(|| p.input.get("diff"))
            .or_else(|| p.input.get("changes"))
            .and_then(|v| v.as_str())
            .or(p.reason.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        if let Some(patch) = patch {
            let mut args = serde_json::Map::new();
            if let Some(path) = path {
                args.insert("path".to_string(), serde_json::Value::String(path));
            }
            args.insert("patch".to_string(), serde_json::Value::String(patch));
            return ("apply_patch".to_string(), serde_json::Value::Object(args));
        }
        if let Some(path) = path {
            return (
                "apply_patch".to_string(),
                serde_json::json!({ "path": path }),
            );
        }
    }

    // An MCP server's confirmation through Codex: say who asks and what.
    if p.tool_name == "mcp_elicitation" {
        let server = p
            .input
            .get("serverName")
            .and_then(|v| v.as_str())
            .unwrap_or("MCP server");
        let message = p
            .input
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        return (
            format!("mcp: {server}"),
            serde_json::json!({
                "tool_call_id": p.tool_use_id.clone().unwrap_or_else(|| p.request_id.clone()),
                "title": if message.is_empty() { server.to_string() } else { format!("{server}: {message}") },
                "input": { "server": server, "message": message },
                "reason": p.reason,
                "source": "native",
            }),
        );
    }

    let display_input = if p.input.as_object().is_some_and(|o| !o.is_empty()) {
        p.input.clone()
    } else if let Some(text) = p.input.as_str().map(str::trim).filter(|s| !s.is_empty()) {
        serde_json::json!({ "value": text })
    } else if let Some(reason) = p.reason.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        serde_json::json!({ "reason": reason })
    } else {
        serde_json::json!({})
    };

    (
        format!("{}: {}", p.tool_name, p.tool_name),
        serde_json::json!({
            "tool_call_id": p.tool_use_id.clone().unwrap_or_else(|| p.request_id.clone()),
            "title": p.tool_name,
            "input": display_input,
            "reason": p.reason,
            "source": "native",
        }),
    )
}

/// Record a session-scoped standing grant, de-duplicated. Mira's own rule
/// for the call shape — what stops the repeat asks without ever handing
/// the agent a persistent grant.
fn record_standing_approval(
    slot: &Arc<SessionSlot>,
    rule: &crate::session_engine::StandingApproval,
) {
    if let Ok(mut m) = slot.engine.standing_approvals.lock() {
        if !m.contains(rule) {
            m.push(rule.clone());
        }
    }
}

/// Codex's native picker and Mira's host gate must describe one posture.
pub(crate) fn codex_approval_mode(mode: &str) -> Option<mira_policy::Mode> {
    match mode {
        "plan" => Some(mira_policy::Mode::Plan),
        "approval-required" => Some(mira_policy::Mode::Manual),
        "auto-accept-edits" => Some(mira_policy::Mode::Auto),
        "auto" => Some(mira_policy::Mode::Edit),
        "full-access" => Some(mira_policy::Mode::Yolo),
        _ => None,
    }
}

/// Apply the persisted chat posture and wake permissions that were already
/// waiting. A native runtime restart must not leave the current turn parked.
pub(crate) async fn set_agent_approval_mode(slot: &Arc<SessionSlot>, mode: mira_policy::Mode) {
    slot.session
        .read()
        .await
        .set_agent_approval_mode(mode)
        .await;
    slot.engine.approval_mode_changed.notify_waiters();
}

async fn approve_native_tool(
    slot: &Arc<SessionSlot>,
    tool: &str,
    call: &mira_core::ToolCall,
) -> bool {
    let (allow, automatic) = wait_native_approval(
        slot.approver.approve(call, Decision::Ask),
        &slot.engine.approval_mode_changed,
        || async { slot.session.read().await.config().await.agent_approval_mode },
        tool,
    )
    .await;
    if automatic
        && crate::approver::resolve(&slot.pending, &call.id.to_string(), true)
            .await
            .is_some()
    {
        let _ = slot.events_tx.send(ServerMsg::ToolEnd {
            result: mira_core::ToolResult::ok(call.id.clone(), "Allowed by the chat approval mode"),
        });
    }
    allow
}

async fn wait_native_approval<F, C, M>(
    approval: F,
    changes: &tokio::sync::Notify,
    mut current_mode: C,
    tool: &str,
) -> (bool, bool)
where
    F: std::future::Future<Output = bool>,
    C: FnMut() -> M,
    M: std::future::Future<Output = mira_policy::Mode>,
{
    tokio::pin!(approval);
    loop {
        // Register before reading state: a mode change between the check and
        // entering the wait must not strand an already-pending permission.
        let changed = changes.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();
        if auto_approve_native_permission(current_mode().await, tool) {
            return (true, true);
        }
        tokio::select! {
            allow = &mut approval => return (allow, false),
            _ = &mut changed => {},
        }
    }
}

/// Native permissions use the same per-chat posture as ACP, without a
/// persistent agent grant. Questions and plan decisions always remain interactive.
fn auto_approve_native_permission(mode: mira_policy::Mode, tool: &str) -> bool {
    let tool = tool.to_ascii_lowercase();
    if matches!(
        tool.as_str(),
        "askuserquestion" | "request_user_input" | "exitplanmode"
    ) {
        return false;
    }
    match mode {
        mira_policy::Mode::Edit | mira_policy::Mode::Yolo => true,
        mira_policy::Mode::Auto => matches!(
            tool.as_str(),
            "apply_patch"
                | "patch"
                | "file_change"
                | "edit"
                | "write"
                | "multiedit"
                | "notebookedit"
        ),
        _ => false,
    }
}

/// Answer one request from a native agent.
///
/// Three kinds of request arrive through the same gate, and each gets the
/// surface Mira already has for it:
/// * `AskUserQuestion` — the agent's clarifying questions, shown as Mira's
///   question card; the answers go back as the tool's input.
/// * `ExitPlanMode` — the agent's plan, shown as Mira's plan card. Approving
///   lets the agent leave plan mode and build; rejecting says why.
/// * anything else — a tool permission, through the session's approver.
///   "Allow for this session" becomes Mira's own standing rule for the
///   call shape (the agent keeps getting one-shot answers); "Always" is
///   session-scoped the same way — Mira never writes the agent's settings
///   files to make a grant stick.
async fn decide_native_permission(
    slot: &Arc<SessionSlot>,
    p: mira_acp::native::NativePermission,
) -> mira_acp::native::PermissionDecision {
    use mira_acp::native::PermissionDecision;
    slot.engine.touch();
    match p.tool_name.as_str() {
        "AskUserQuestion" | "request_user_input" => ask_agent_questions(slot, &p).await,
        "ExitPlanMode" => review_agent_plan(slot, &p).await,
        "mcp_elicitation" if elicitation_has_form(&p.input) || is_url_elicitation(&p.input) => {
            codex_elicitation(slot, &p).await
        }
        _ => {
            // Re-read for every request: selecting Auto everything takes
            // effect immediately, and returning to Ask restores the gate.
            let mode = slot.session.read().await.config().await.agent_approval_mode;
            if auto_approve_native_permission(mode, &p.tool_name) {
                return PermissionDecision {
                    allow: true,
                    ..Default::default()
                };
            }
            let call_id = mira_core::ToolCallId::new();
            let call = native_permission_call(&p, call_id.clone());
            let (view_name, view_args) = native_permission_tool_view(&p);
            // A standing session grant answers before anyone is asked —
            // same rule book as the ACP path, same one-shot reply.
            if let Ok(m) = slot.engine.standing_approvals.lock() {
                if crate::session_engine::standing_allows(&m, &view_name, &view_args) {
                    return PermissionDecision {
                        allow: true,
                        updated_permissions: Vec::new(),
                        ..Default::default()
                    };
                }
            }
            let allow = approve_native_tool(slot, &p.tool_name, &call).await;
            let scope = slot
                .engine
                .approval_scopes
                .lock()
                .ok()
                .and_then(|mut m| m.remove(&call_id.to_string()));
            if !allow {
                return PermissionDecision {
                    allow: false,
                    message: Some("The user denied this in Mira. Don't retry it as-is; ask or take another approach.".into()),
                    ..Default::default()
                };
            }
            let standing = || crate::session_engine::StandingApproval {
                tool: view_name.clone(),
                target: crate::session_engine::standing_target(&view_name, &view_args),
            };
            let updated_permissions = match scope {
                Some(crate::protocol::ApprovalScope::Session) => {
                    record_standing_approval(slot, &standing());
                    mira_acp::native::session_permission_updates(
                        &p.tool_name,
                        &p.suggestions,
                        false,
                    )
                }
                Some(crate::protocol::ApprovalScope::Always) => {
                    // Session-scoped like everything else here: persisting
                    // (`localSettings`) would write the agent's own config
                    // files, giving one chat's approval a life beyond Mira.
                    record_standing_approval(slot, &standing());
                    let _ = slot.events_tx.send(ServerMsg::Warning {
                        text: "allowed for this session: Mira won't ask again, and the agent's own settings are untouched".into(),
                    });
                    mira_acp::native::session_permission_updates(
                        &p.tool_name,
                        &p.suggestions,
                        false,
                    )
                }
                _ => Vec::new(),
            };
            PermissionDecision {
                allow: true,
                updated_permissions,
                ..Default::default()
            }
        }
    }
}

/// Wrap an interactive prompt in a synthetic tool call, so the client's
/// existing cards (which attach to a tool entry by id) render it unchanged.
async fn with_prompt_card<F, Fut, T>(
    slot: &SessionSlot,
    name: &str,
    args: serde_json::Value,
    ask: F,
) -> T
where
    F: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = (T, String)>,
{
    let call_id = mira_core::ToolCallId::new();
    let call = mira_core::ToolCall {
        id: call_id.clone(),
        kind: mira_core::ToolCallKind::Function,
        function: mira_core::ToolCallFunction {
            name: name.to_string(),
            arguments: args.to_string(),
        },
    };
    let _ = slot.events_tx.send(ServerMsg::ToolStart { call });
    let (out, summary) = ask(call_id.to_string()).await;
    let _ = slot.events_tx.send(ServerMsg::ToolEnd {
        result: mira_core::ToolResult {
            call_id,
            content: summary,
            is_error: false,
            data: None,
            images: Vec::new(),
        },
    });
    out
}

/// `AskUserQuestion` → Mira's question card → the agent's answers.
///
/// Answers are keyed by the full question text: that is how Claude Code
/// looks them up when it builds the tool result.
async fn ask_agent_questions(
    slot: &SessionSlot,
    p: &mira_acp::native::NativePermission,
) -> mira_acp::native::PermissionDecision {
    use mira_acp::native::PermissionDecision;
    use mira_tools::prompt::{
        AskUserOption, AskUserProposal, AskUserQuestion, PromptRequest, PromptResponse,
    };
    let raw = p
        .input
        .get("questions")
        .and_then(|q| q.as_array())
        .cloned()
        .unwrap_or_default();
    let questions: Vec<AskUserQuestion> = raw
        .iter()
        .map(|q| AskUserQuestion {
            question: q
                .get("question")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            header: q.get("header").and_then(|v| v.as_str()).map(str::to_string),
            options: q
                .get("options")
                .and_then(|o| o.as_array())
                .map(|opts| {
                    opts.iter()
                        .map(|o| AskUserOption {
                            label: o
                                .get("label")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            description: o
                                .get("description")
                                .and_then(|v| v.as_str())
                                .map(str::to_string),
                            recommended: false,
                        })
                        .collect()
                })
                .unwrap_or_default(),
            multi_select: q
                .get("multiSelect")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        })
        .collect();
    if questions.is_empty() {
        return PermissionDecision::from(true);
    }
    let proposal = AskUserProposal {
        questions: questions.clone(),
    };
    let channel = slot.prompt_channel.clone();
    let args = serde_json::json!({ "questions": raw });
    with_prompt_card(slot, "ask_user", args, |id| async move {
        let resp = channel.ask(id, PromptRequest::AskUser(proposal)).await;
        let answers = match resp {
            Some(PromptResponse::AskUser(r)) if !r.cancelled && !r.answers.is_empty() => r.answers,
            _ => {
                return (
                    PermissionDecision {
                        allow: false,
                        message: Some("The user dismissed the questions without answering.".into()),
                        ..Default::default()
                    },
                    "dismissed".to_string(),
                )
            }
        };
        let mut map = serde_json::Map::new();
        let mut summary = Vec::new();
        for ((q, raw_question), a) in questions.iter().zip(raw.iter()).zip(answers.iter()) {
            let text = match &a.custom {
                Some(c) if !c.trim().is_empty() => c.trim().to_string(),
                _ => a.picked.join(", "),
            };
            summary.push(format!("{}: {}", q.question, text));
            if p.tool_name == "request_user_input" {
                if let Some(id) = raw_question.get("id").and_then(|v| v.as_str()) {
                    let values = match &a.custom {
                        Some(c) if !c.trim().is_empty() => vec![c.trim().to_string()],
                        _ => a.picked.clone(),
                    };
                    map.insert(id.to_string(), serde_json::json!({ "answers": values }));
                }
            } else {
                map.insert(q.question.clone(), serde_json::Value::String(text));
            }
        }
        (
            PermissionDecision {
                allow: true,
                updated_input: Some(serde_json::json!({ "questions": raw, "answers": map })),
                ..Default::default()
            },
            summary.join("\n"),
        )
    })
    .await
}

/// One field of an agent's elicitation form, as a question on the card.
struct FormField {
    key: String,
    /// `{key}_custom`, when the form has a free-text twin for this field
    /// (OpenCode's "Type your own answer").
    custom_key: Option<String>,
    kind: FieldKind,
    /// `(value sent back, label shown)`.
    choices: Vec<(serde_json::Value, String)>,
    question: mira_tools::prompt::AskUserQuestion,
}

#[derive(Clone, Copy, PartialEq)]
enum FieldKind {
    Text,
    Number,
    Boolean,
    Choice,
    Multi,
}

fn form_fields(message: &str, schema: &serde_json::Value) -> Vec<FormField> {
    use mira_tools::prompt::{AskUserOption, AskUserQuestion};
    let Some(props) = schema.get("properties").and_then(|p| p.as_object()) else {
        return Vec::new();
    };
    let s = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
    let choices_of =
        |list: Option<&serde_json::Value>| -> Vec<(serde_json::Value, String, Option<String>)> {
            list.and_then(|l| l.as_array())
                .map(|l| {
                    l.iter()
                        .filter_map(|o| match o {
                            serde_json::Value::String(v) => Some((o.clone(), v.clone(), None)),
                            _ => {
                                let value = o.get("const")?.clone();
                                let label = s(o, "title").unwrap_or_else(|| {
                                    value
                                        .as_str()
                                        .map(str::to_string)
                                        .unwrap_or_else(|| value.to_string())
                                });
                                Some((value, label, s(o, "description")))
                            }
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
    let mut out = Vec::new();
    for (key, prop) in props {
        // A free-text twin is folded into the field it belongs to.
        if let Some(base) = key.strip_suffix("_custom") {
            if props.contains_key(base) {
                continue;
            }
        }
        let ty = s(prop, "type").unwrap_or_default();
        let (kind, choices) = if ty == "array" {
            let items = prop.get("items").cloned().unwrap_or_default();
            let c = choices_of(
                items
                    .get("anyOf")
                    .or(items.get("oneOf"))
                    .or(items.get("enum")),
            );
            (FieldKind::Multi, c)
        } else if prop.get("oneOf").is_some() || prop.get("enum").is_some() {
            (
                FieldKind::Choice,
                choices_of(prop.get("oneOf").or(prop.get("enum"))),
            )
        } else if ty == "boolean" {
            (
                FieldKind::Boolean,
                vec![
                    (serde_json::Value::Bool(true), "Yes".to_string(), None),
                    (serde_json::Value::Bool(false), "No".to_string(), None),
                ],
            )
        } else if ty == "number" || ty == "integer" {
            (FieldKind::Number, Vec::new())
        } else {
            (FieldKind::Text, Vec::new())
        };
        let title = s(prop, "title");
        let description = s(prop, "description");
        let question = description
            .clone()
            .or_else(|| title.clone())
            .unwrap_or_else(|| {
                if message.is_empty() {
                    key.clone()
                } else {
                    message.to_string()
                }
            });
        let custom_key = format!("{key}_custom");
        out.push(FormField {
            key: key.clone(),
            custom_key: props.contains_key(&custom_key).then_some(custom_key),
            kind,
            choices: choices
                .iter()
                .map(|(v, l, _)| (v.clone(), l.clone()))
                .collect(),
            question: AskUserQuestion {
                question,
                // The title is the short topic when there's a sentence too.
                header: description.and(title),
                options: choices
                    .into_iter()
                    .map(|(_, label, description)| AskUserOption {
                        label,
                        description,
                        recommended: false,
                    })
                    .collect(),
                multi_select: kind == FieldKind::Multi,
            },
        });
    }
    out
}

/// The card's answers, as the content object the form asked for.
fn form_content(
    fields: &[FormField],
    answers: &[mira_tools::prompt::AskUserAnswer],
) -> serde_json::Map<String, serde_json::Value> {
    use serde_json::Value;
    let mut content = serde_json::Map::new();
    for (f, a) in fields.iter().zip(answers) {
        let picked: Vec<Value> = a
            .picked
            .iter()
            .filter_map(|l| {
                f.choices
                    .iter()
                    .find(|(_, label)| label == l)
                    .map(|(v, _)| v.clone())
            })
            .collect();
        let typed = a.custom.as_deref().map(str::trim).filter(|t| !t.is_empty());
        match f.kind {
            FieldKind::Multi => {
                if !picked.is_empty() {
                    content.insert(f.key.clone(), Value::Array(picked));
                }
            }
            FieldKind::Choice | FieldKind::Boolean => {
                if let Some(v) = picked.into_iter().next() {
                    content.insert(f.key.clone(), v);
                } else if f.kind == FieldKind::Boolean {
                    if let Some(t) = typed {
                        let yes = matches!(t.to_lowercase().as_str(), "yes" | "y" | "true");
                        content.insert(f.key.clone(), Value::Bool(yes));
                    }
                }
            }
            FieldKind::Number => {
                if let Some(n) = typed.and_then(|t| t.parse::<f64>().ok()) {
                    content.insert(f.key.clone(), serde_json::json!(n));
                }
            }
            FieldKind::Text => {
                if let Some(t) = typed {
                    content.insert(f.key.clone(), Value::String(t.to_string()));
                }
            }
        }
        if let (Some(t), Some(ck)) = (typed, &f.custom_key) {
            if f.kind != FieldKind::Text {
                content.insert(ck.clone(), Value::String(t.to_string()));
            }
        }
    }
    content
}

/// An agent's form (ACP `elicitation/create`) → Mira's question card → the
/// filled-in form. OpenCode's `question` tool and its plan prompts arrive
/// this way.
pub(crate) async fn ask_elicitation(
    slot: &SessionSlot,
    req: &mira_acp::host::ElicitationRequest,
) -> mira_acp::host::ElicitationReply {
    use mira_acp::host::ElicitationReply;
    use mira_tools::prompt::{AskUserProposal, PromptRequest, PromptResponse};
    slot.engine.touch();
    let fields = form_fields(&req.message, &req.schema);
    if fields.is_empty() {
        // Nothing to fill in: a bare confirmation.
        return ElicitationReply::Accept(serde_json::Map::new());
    }
    let proposal = AskUserProposal {
        questions: fields.iter().map(|f| f.question.clone()).collect(),
    };
    let args = serde_json::json!({
        "questions": fields.iter().map(|f| serde_json::json!({
            "question": f.question.question,
            "header": f.question.header,
            "multiSelect": f.question.multi_select,
            "options": f.question.options.iter().map(|o| serde_json::json!({
                "label": o.label,
                "description": o.description,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    });
    let channel = slot.prompt_channel.clone();
    with_prompt_card(slot, "ask_user", args, |id| async move {
        match channel.ask(id, PromptRequest::AskUser(proposal)).await {
            Some(PromptResponse::AskUser(r)) if !r.cancelled && !r.answers.is_empty() => {
                let summary = fields
                    .iter()
                    .zip(&r.answers)
                    .map(|(f, a)| {
                        let text = match a.custom.as_deref().map(str::trim) {
                            Some(c) if !c.is_empty() => c.to_string(),
                            _ => a.picked.join(", "),
                        };
                        format!("{}: {}", f.question.question, text)
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                (
                    ElicitationReply::Accept(form_content(&fields, &r.answers)),
                    summary,
                )
            }
            _ => (ElicitationReply::Cancel, "dismissed".to_string()),
        }
    })
    .await
}

fn is_url_elicitation(params: &serde_json::Value) -> bool {
    params.get("mode").and_then(|m| m.as_str()) == Some("url")
}

/// Whether an MCP elicitation asks for fields, rather than a bare yes/no.
fn elicitation_has_form(params: &serde_json::Value) -> bool {
    params
        .get("requestedSchema")
        .and_then(|s| s.get("properties"))
        .and_then(|p| p.as_object())
        .is_some_and(|p| !p.is_empty())
}

/// An MCP server's request through Codex, answered on Mira's form card. A
/// bare confirmation never gets here: it goes through the normal approval
/// path, so it is asked (or auto-approved by posture) like any tool call
/// rather than accepted silently.
async fn codex_elicitation(
    slot: &SessionSlot,
    p: &mira_acp::native::NativePermission,
) -> mira_acp::native::PermissionDecision {
    use mira_acp::host::{ElicitationReply, ElicitationRequest};
    use mira_acp::native::PermissionDecision;
    if is_url_elicitation(&p.input) {
        return PermissionDecision {
            allow: false,
            message: Some("Opening a link for an MCP server isn't supported in Mira yet.".into()),
            ..Default::default()
        };
    }
    let server = p
        .input
        .get("serverName")
        .and_then(|v| v.as_str())
        .unwrap_or("An MCP server");
    let message = p
        .input
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let req = ElicitationRequest {
        session_id: String::new(),
        tool_call_id: p.tool_use_id.clone(),
        message: if message.is_empty() {
            format!("{server} needs some details")
        } else {
            format!("{server}: {message}")
        },
        schema: p.input.get("requestedSchema").cloned().unwrap_or_default(),
    };
    match ask_elicitation(slot, &req).await {
        ElicitationReply::Accept(content) => PermissionDecision {
            allow: true,
            updated_input: Some(serde_json::json!({ "content": content })),
            ..Default::default()
        },
        ElicitationReply::Decline | ElicitationReply::Cancel => PermissionDecision {
            allow: false,
            message: Some("The user declined.".into()),
            ..Default::default()
        },
    }
}

/// `ExitPlanMode` → Mira's plan card.
///
/// Approving lets the agent leave plan mode and start; an edited plan goes
/// back as the plan it should follow. Rejecting keeps it planning, with the
/// user's note as the reason.
async fn review_agent_plan(
    slot: &SessionSlot,
    p: &mira_acp::native::NativePermission,
) -> mira_acp::native::PermissionDecision {
    use mira_acp::native::PermissionDecision;
    use mira_tools::prompt::{PromptRequest, PromptResponse};
    let markdown = p
        .input
        .get("plan")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let proposal = plan_from_markdown(&markdown);
    let channel = slot.prompt_channel.clone();
    let args = serde_json::json!({ "title": proposal.title, "steps": proposal.steps });
    with_prompt_card(slot, "plan", args, |id| async move {
        match channel.ask(id, PromptRequest::Plan(proposal)).await {
            Some(PromptResponse::Plan(r)) if r.approved => {
                let updated_input = r.steps.map(|steps| {
                    let plan = steps
                        .iter()
                        .enumerate()
                        .map(|(i, s)| match &s.why {
                            Some(w) => format!("{}. {} — {}", i + 1, s.description, w),
                            None => format!("{}. {}", i + 1, s.description),
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    serde_json::json!({ "plan": plan })
                });
                (
                    PermissionDecision {
                        allow: true,
                        updated_input,
                        ..Default::default()
                    },
                    "approved".to_string(),
                )
            }
            Some(PromptResponse::Plan(r)) => (
                PermissionDecision {
                    allow: false,
                    message: Some(match r.note {
                        Some(n) if !n.trim().is_empty() => {
                            format!("The user rejected the plan: {n}. Revise it and propose again.")
                        }
                        _ => {
                            "The user rejected the plan. Stay in plan mode and ask what to change."
                                .into()
                        }
                    }),
                    ..Default::default()
                },
                "rejected".to_string(),
            ),
            _ => (
                PermissionDecision {
                    allow: false,
                    message: Some("The plan was not reviewed. Wait for the user.".into()),
                    ..Default::default()
                },
                "dismissed".to_string(),
            ),
        }
    })
    .await
}

/// Turn an agent's markdown plan into Mira's plan steps: the first heading
/// is the title, list items are steps; a plan with no list becomes one
/// step per paragraph, so nothing the agent wrote is dropped.
pub(crate) fn plan_from_markdown(md: &str) -> mira_tools::prompt::PlanProposal {
    use mira_tools::prompt::{PlanProposal, PlanStep};
    let mut title = None;
    let mut steps = Vec::new();
    for line in md.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if let Some(h) = t.strip_prefix('#') {
            if title.is_none() {
                title = Some(h.trim_start_matches('#').trim().to_string());
            }
            continue;
        }
        let item = t
            .strip_prefix("- ")
            .or_else(|| t.strip_prefix("* "))
            .or_else(|| {
                let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
                (digits > 0)
                    .then(|| {
                        t[digits..]
                            .strip_prefix(". ")
                            .or_else(|| t[digits..].strip_prefix(") "))
                    })
                    .flatten()
            });
        if let Some(item) = item {
            steps.push(PlanStep {
                description: item.trim().to_string(),
                why: None,
            });
        }
    }
    if steps.is_empty() {
        steps = md
            .split("\n\n")
            .map(str::trim)
            .filter(|p| !p.is_empty() && !p.starts_with('#'))
            .map(|p| PlanStep {
                description: p.to_string(),
                why: None,
            })
            .collect();
    }
    PlanProposal {
        title: title
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| "Agent plan".to_string()),
        steps,
    }
}

/// Bridge the port set into `start_driver`.
#[allow(clippy::too_many_arguments)]
async fn start_acp_agent_with_ports(
    driver: &dyn AcpDriver,
    cfg: &DriverConfig,
    mode: PermissionMode,
    program: &std::path::Path,
    files: Arc<AcpFiles>,
    terminals: Arc<AcpTerminals>,
    permissions: Arc<AcpPermissions>,
    events: Arc<AcpEventPort>,
    cwd: &std::path::Path,
    gate: mira_acp::native::PermissionGate,
    native: Option<mira_acp::native::NativeOverrides>,
    mira_mcp: Option<mira_acp::session::MiraMcp>,
) -> Result<AgentHandle, StartError> {
    start_acp_agent(
        driver,
        cfg,
        mode,
        program,
        mira_acp::process::HostPorts {
            files,
            terminals,
            permissions,
            events,
            mira_mcp,
        },
        Some(cwd),
        gate,
        native,
    )
    .await
}

fn describe(e: &StartError) -> String {
    match e {
        StartError::Spawn(mira_acp::process::SpawnError::NotFound(p)) => {
            format!("agent binary not found: {p}")
        }
        other => other.to_string(),
    }
}

/// Everything it takes to (re)start a session's agent. Also what a *new*
/// session inherits (`new_session` copies it forward, so the next chat
/// starts configured the same way) and, secrets removed, what the session
/// record keeps so a reload routes back to the same agent.
#[derive(Clone, Debug)]
pub struct AcpLaunchParams {
    /// Engine instance this session runs on (`codex-work`, or the driver
    /// kind for the default instance). Routing is by instance, never by
    /// kind: two instances of one driver must not share resume cursors,
    /// spend rows, or processes even when the rest matches.
    pub instance: String,
    pub driver_kind: String,
    pub cfg: DriverConfig,
    pub mode: PermissionMode,
    /// The agent model picked for this session; applied on every launch.
    pub model: Option<String>,
    /// The agent's own permission-mode id picked for this session (native
    /// transports take it at launch); applied on every launch.
    pub mode_id: Option<String>,
}

impl AcpLaunchParams {
    pub fn new(driver_kind: impl Into<String>, cfg: DriverConfig) -> Self {
        let driver_kind = driver_kind.into();
        // The default instance of a driver *is* its kind — the same rule
        // the registry uses — so kind-only callers keep routing exactly
        // where they always did.
        let instance = driver_kind.clone();
        AcpLaunchParams {
            instance,
            driver_kind,
            cfg,
            mode: PermissionMode::Ask,
            model: None,
            mode_id: None,
        }
    }

    /// Params for a registry instance: the id travels with the launch so
    /// everything downstream (cursors, spend, engine frames, reload)
    /// routes by instance.
    pub fn for_instance(
        instance: impl Into<String>,
        driver_kind: impl Into<String>,
        cfg: DriverConfig,
    ) -> Self {
        let mut p = AcpLaunchParams::new(driver_kind, cfg);
        p.instance = instance.into();
        p
    }

    /// The name to show: the user's override, else the driver's own.
    pub fn display_name(&self) -> String {
        self.cfg.display_name.clone().unwrap_or_else(|| {
            mira_acp::drivers::by_kind(&self.driver_kind)
                .map(|d| d.display_name().to_string())
                .unwrap_or_else(|| self.driver_kind.clone())
        })
    }

    /// Same agent, launched the same way — a running process for `self`
    /// can serve `other` without a restart. The model is left out: it is
    /// applied separately and must not cost the user their agent process.
    /// The instance is in: two instances may share every other field while
    /// billing different accounts, and then they must not share a process.
    pub fn same_agent(&self, other: &AcpLaunchParams) -> bool {
        self.instance == other.instance
            && self.driver_kind == other.driver_kind
            && self.cfg.binary_path == other.cfg.binary_path
            && self.cfg.home_path == other.cfg.home_path
            && self.cfg.launch_args == other.cfg.launch_args
            && self.cfg.env == other.cfg.env
            && self.cfg.effort == other.cfg.effort
            && self.cfg.setting_sources == other.cfg.setting_sources
    }

    /// The persisted form: everything needed to relaunch, minus secrets.
    /// The API key is never written, nor any env var whose name marks it as
    /// a credential — a record on disk must not become a key store.
    pub fn to_persisted(&self) -> serde_json::Value {
        let secret: std::collections::HashSet<String> =
            match mira_acp::drivers::by_kind(&self.driver_kind) {
                Some(d) => self.cfg.secret_keys(d.as_ref()).into_iter().collect(),
                None => Default::default(),
            };
        let env: std::collections::BTreeMap<&String, &String> = self
            .cfg
            .env
            .iter()
            .filter(|(k, _)| !secret.contains(*k))
            .collect();
        serde_json::json!({
            "instance": self.instance,
            "display_name": self.cfg.display_name,
            "binary_path": self.cfg.binary_path,
            "home_path": self.cfg.home_path,
            "launch_args": self.cfg.launch_args,
            "env": env,
            "effort": self.cfg.effort,
            "setting_sources": self.cfg.setting_sources,
            "model": self.model,
            "mode_id": self.mode_id,
        })
    }

    /// Rebuild launch params from a session record's agent meta.
    pub fn from_meta(meta: &mira_harness::persist::AgentSessionMeta) -> Self {
        let v = meta.launch.clone().unwrap_or(serde_json::Value::Null);
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
        let instance = s("instance")
            .or_else(|| meta.instance.clone())
            .unwrap_or_else(|| meta.driver_kind.clone());
        let cfg = DriverConfig {
            enabled: true,
            display_name: s("display_name"),
            binary_path: s("binary_path").map(PathBuf::from),
            home_path: s("home_path").map(PathBuf::from),
            launch_args: v
                .get("launch_args")
                .and_then(|a| a.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            env: v
                .get("env")
                .and_then(|e| e.as_object())
                .map(|o| {
                    o.iter()
                        .filter_map(|(k, x)| x.as_str().map(|x| (k.clone(), x.to_string())))
                        .collect()
                })
                .unwrap_or_default(),
            effort: s("effort"),
            setting_sources: s("setting_sources"),
            ..Default::default()
        };
        AcpLaunchParams {
            instance,
            driver_kind: meta.driver_kind.clone(),
            cfg,
            mode: PermissionMode::Ask,
            model: s("model").or_else(|| meta.model.clone()),
            mode_id: s("mode_id"),
        }
    }
}

/// Resolve what an `AcpStart` means, before any process is spawned.
///
/// Precedence, most specific wins:
/// 1. `instance` — an engine registry entry (`mira.yaml`'s `engines:`),
///    handed in already resolved.
/// 2. The slot's recorded launch params — the settings the previous
///    chat ran with — when the client named that same driver or named
///    none at all. This is what makes "new chat, same agent" work.
/// 3. A bare driver kind with default config — the legacy path, routed
///    at that kind's default instance.
///
/// `Err` when there is nothing to start from.
pub fn resolve_start_params(
    recorded: Option<AcpLaunchParams>,
    instance: Option<InstanceStart>,
    driver: Option<&str>,
) -> Result<ResolvedAgentStart, String> {
    if let Some(inst) = instance {
        return Ok(ResolvedAgentStart {
            instance: inst.instance,
            kind: inst.kind,
            cfg: inst.cfg,
        });
    }
    if let Some(rec) = &recorded {
        let same = driver.is_none_or(|d| d == rec.driver_kind);
        if same {
            return Ok(ResolvedAgentStart {
                instance: rec.instance.clone(),
                kind: rec.driver_kind.clone(),
                cfg: rec.cfg.clone(),
            });
        }
    }
    match driver {
        Some(d) => Ok(ResolvedAgentStart {
            instance: d.to_string(),
            kind: d.to_string(),
            cfg: DriverConfig::default(),
        }),
        None => Err("no agent is configured for this session — pick one in the \
                     model picker's Agent tab"
            .to_string()),
    }
}

/// A registry instance handed to [`resolve_start_params`] already
/// resolved: its id, its driver kind, and its launch config.
pub struct InstanceStart {
    pub instance: String,
    pub kind: String,
    pub cfg: DriverConfig,
}

/// What an `AcpStart` resolved to: the instance to route by, the driver
/// to launch, and the config to launch it with.
#[derive(Debug)]
pub struct ResolvedAgentStart {
    pub instance: String,
    pub kind: String,
    pub cfg: DriverConfig,
}

/// One recorded turn, for the revert picker: number, time, opening words,
/// and whether files can actually be restored (a snapshot exists).
#[derive(Clone, Debug, serde::Serialize)]
pub struct AgentTurnSummary {
    pub turn: u64,
    pub t: u64,
    pub first_text: String,
    pub snapshot: bool,
}

/// List recorded turns oldest-first. Snapshot lines precede their turn's
/// user line in the file, so one pass with a pending flag pairs them.
pub fn agent_turns(
    slot_id: &str,
    store: Option<&std::sync::Arc<dyn mira_harness::SessionStore>>,
) -> Vec<AgentTurnSummary> {
    let Some(path) = store.and_then(|s| s.agent_log_path(&mira_core::SessionId::from(slot_id)))
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut pending_snapshot = false;
    for line in mira_acp::agent_sessions::read_lines(&path) {
        if line
            .get("snapshot")
            .and_then(|s| s.get("hash"))
            .and_then(|h| h.as_str())
            .is_some()
        {
            pending_snapshot = true;
            continue;
        }
        if let Some(u) = line.get("user") {
            let text = u.get("text").and_then(|t| t.as_str()).unwrap_or("");
            out.push(AgentTurnSummary {
                turn: out.len() as u64 + 1,
                t: line.get("t").and_then(|t| t.as_u64()).unwrap_or(0),
                first_text: text.chars().take(60).collect(),
                snapshot: pending_snapshot,
            });
            pending_snapshot = false;
        }
    }
    out
}

/// Relaunch the slot's native agent with overrides, keeping the session.
///
/// Native launch flags (`--model`, `--permission-mode`) only take effect at
/// spawn, so changing either means a new process. `--resume` carries the
/// session across, which is why this is a settings change and not data loss.
/// Not offered for ACP agents: they apply both live.
///
/// A model or mode given here becomes the session's standing choice, so
/// the next restart — for any reason — keeps it.
pub async fn restart_native_agent(
    state: &AppState,
    slot: &Arc<SessionSlot>,
    overrides: mira_acp::native::NativeOverrides,
) -> Result<Arc<SlotAgent>, String> {
    let _dispatch = slot.engine.dispatch_lock.lock().await;
    restart_native_agent_locked(state, slot, overrides).await
}

async fn restart_native_agent_locked(
    state: &AppState,
    slot: &Arc<SessionSlot>,
    overrides: mira_acp::native::NativeOverrides,
) -> Result<Arc<SlotAgent>, String> {
    let _guard = slot.engine.start_lock.lock().await;
    let params = {
        let mut launch = slot.acp_launch.lock().await;
        let p = launch
            .as_mut()
            .ok_or_else(|| "no external agent is set up for this session".to_string())?;
        if overrides.model.is_some() {
            p.model = overrides.model.clone();
        }
        if overrides.permission_mode.is_some() {
            p.mode_id = overrides.permission_mode.clone();
        }
        p.clone()
    };
    // The session id before stopping: the handle owns it, and stopping drops
    // the handle. Without this the relaunched agent starts a blank session
    // and the conversation is silently forked.
    let sid = match slot.acp_agent.read().await.clone() {
        Some(h) => match h.agent().await {
            Some(a) => a.session_id().await,
            None => None,
        },
        None => None,
    };
    let mut overrides = overrides;
    if overrides.resume.is_none() {
        overrides.resume = sid;
    }
    // Work the old process owns dies with it; tell the agent once it's back.
    let doomed: Vec<_> = slot
        .engine
        .activity
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .work_snapshot()
        .into_iter()
        .filter(|w| w.status.is_active())
        .collect();
    *slot.engine.phase.lock().await = crate::session_engine::AgentPhase::Starting;
    crate::session_engine::publish(state, slot).await;
    let result = start_agent(state, slot, &params, Some(overrides)).await;
    if result.is_ok() {
        if let Some(note) = crate::session_engine::restart_note(&doomed) {
            crate::session_engine::note_for_agent(slot, note).await;
        }
    }
    *slot.engine.phase.lock().await = match &result {
        Ok(_) => crate::session_engine::AgentPhase::Idle,
        Err(e) => crate::session_engine::AgentPhase::Failed(e.clone()),
    };
    crate::session_engine::publish(state, slot).await;
    result
}

/// Frames safe to persist and replay through the client's live handler.
///
/// Everything here renders transcript content. Approval requests,
/// confirmations, agent-started banners and health snapshots are session
/// ephemera: replaying them would re-trigger dialogs or lie about the
/// present, so they stay live-only.
fn is_transcript_frame(msg: &ServerMsg) -> bool {
    use ServerMsg::*;
    matches!(
        msg,
        ToolStart { .. }
            | ToolEnd { .. }
            | StreamActivity { .. }
            | AcpMessageMetadata { .. }
            | AcpText { .. }
            | AcpTextSnapshot { .. }
            | AcpToolOutputDelta { .. }
            | AcpThought { .. }
            | AcpToolCall { .. }
            | AcpToolCallUpdate { .. }
            | AcpPlan { .. }
            | AcpModes { .. }
            | AcpConfigOptions { .. }
            | AcpCommands { .. }
            | AcpUsage { .. }
            | AcpTurnUsage { .. }
            | AcpLimits { .. }
            | AcpSessionInfo { .. }
            | AcpTurnEnd { .. }
            | AcpUnmodelled { .. }
            | AcpModeChanged { .. }
            | HtmlRender { .. }
            | Warning { .. }
            | Error { .. }
    )
}

/// Revert an agent session to before a turn ran.
///
/// Files go back to that turn's snapshot, the sidecar drops the turn and
/// everything after it, and the agent restarts fresh — there is no API for
/// rewinding a CLI's memory, so continuing the old process would act on
/// knowledge the transcript no longer shows.
///
/// Turns without a snapshot (non-repo, clean tree) still truncate the
/// transcript; only the file restore is skipped, and the message says so.
pub async fn revert_agent_turn(
    state: &AppState,
    slot: &std::sync::Arc<SessionSlot>,
    turn: u64,
) -> Result<String, String> {
    let _dispatch = slot.engine.dispatch_lock.lock().await;
    if turn < 1 {
        return Err("turn numbers start at 1".to_string());
    }
    let Some(path) = state
        .store
        .as_ref()
        .and_then(|s| s.agent_log_path(&slot.id))
    else {
        return Err("persistence is disabled — nothing recorded to revert".to_string());
    };
    let lines = mira_acp::agent_sessions::read_lines(&path);
    let hash = lines
        .iter()
        .filter_map(|l| {
            let s = l.get("snapshot")?;
            (s.get("turn").and_then(|t| t.as_u64()) == Some(turn))
                .then(|| s.get("hash").and_then(|h| h.as_str()))
                .flatten()
        })
        .next()
        .map(str::to_string);

    // An agent that can rewind its own conversation (Codex) forgets the
    // reverted prompts and keeps the rest; anything else restarts fresh.
    // Prompts to undo: this agent's user messages from `turn` on, minus
    // steers (those joined a running turn rather than starting one).
    let driver = slot
        .acp_launch
        .lock()
        .await
        .as_ref()
        .map(|p| p.driver_kind.clone());
    let prompts = lines
        .iter()
        .filter(|l| l.get("user").is_some())
        .skip(turn as usize - 1)
        .filter(|l| l.pointer("/user/input_intent").and_then(|v| v.as_str()) != Some("steer"))
        .filter(|l| {
            driver.is_none() || l.get("driver").and_then(|d| d.as_str()) == driver.as_deref()
        })
        .count();
    let live = match slot.acp_agent.read().await.clone() {
        Some(h) => h.agent().await,
        None => None,
    };
    let rewind = match live {
        Some(agent) => agent.rewind(prompts).await,
        None => None,
    };

    let mut notes = Vec::new();
    if let Some(hash) = hash.as_deref() {
        let cwd = slot.cwd.read().await.clone();
        mira_acp::snapshot::restore_snapshot(&cwd, hash)
            .map_err(|e| format!("could not restore files: {e}"))?;
        notes.push(format!("files restored to before turn {turn}"));
    } else {
        notes.push(format!(
            "no file snapshot for turn {turn} — transcript only"
        ));
    }

    mira_acp::snapshot::truncate_from_turn(&path, turn)
        .map_err(|e| format!("could not truncate transcript: {e}"))?;
    match rewind {
        Some(Ok(())) => {
            notes.push(format!(
                "rewound the agent's own thread by {prompts} message{}",
                if prompts == 1 { "" } else { "s" }
            ));
            return Ok(notes.join("; "));
        }
        Some(Err(e)) => notes.push(format!("couldn't rewind the agent ({e})")),
        None => {}
    }
    // The cursor points at the pre-revert session; keeping it would resume
    // exactly what was just left behind.
    if let Some(params) = slot.acp_launch.lock().await.clone() {
        if let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) {
            mira_acp::agent_sessions::remove_in(&home, &slot.id.to_string(), &params.instance);
        }
    }

    // Fresh process, no resume: the old agent remembers the reverted
    // turns, so continuing it would act on knowledge the transcript no
    // longer shows. Restarting is free until the next prompt, and keeps the
    // revert to one user action instead of two.
    stop_agent(slot).await;
    let handle =
        restart_native_agent_locked(state, slot, mira_acp::native::NativeOverrides::default())
            .await?;
    Ok(format!(
        "{}; restarted {}",
        notes.join("; "),
        handle.display_name
    ))
}

/// Stop whatever agent `slot` is running.
pub async fn stop_agent(slot: &Arc<SessionSlot>) -> bool {
    // The chat's tool-server token stays: the next agent here reuses it
    // (it ends when the chat is deleted, or goes unused for 12 h).
    slot.engine.generation.fetch_add(1, Ordering::SeqCst);
    slot.engine.agent_in_turn.store(false, Ordering::SeqCst);
    let cancelled = slot
        .engine
        .activity
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .cancel_work();
    for work in cancelled {
        let _ = slot
            .events_tx
            .send(crate::protocol::ServerMsg::RuntimeWorkUpdated { work });
    }
    *slot
        .engine
        .activity
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Default::default();
    slot.publish_activity();
    match slot.acp_agent.write().await.take() {
        Some(prev) => {
            prev.stop().await;
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod privilege_tests {
    use super::*;
    use mira_acp::driver::PermissionMode;

    /// A `SlotAgent` with only the fields the mode check reads.
    fn agent(kind: &str) -> Arc<SlotAgent> {
        Arc::new(SlotAgent {
            opencode_control: None,
            lease: std::sync::Mutex::new(None),
            driver_kind: kind.to_string(),
            instance: kind.to_string(),
            display_name: kind.to_string(),
            launch: String::new(),
            agent_files_dir: None,
            agent: RwLock::new(None),
            terminals: AcpTerminals::new(
                "s1",
                std::env::temp_dir(),
                Arc::new(PtyRegistry::new()),
                false,
            ),
            stopping: Arc::new(AtomicBool::new(false)),
        })
    }

    #[test]
    fn the_mode_that_disables_the_sandbox_is_flagged() {
        let a = agent("codex");
        assert!(
            a.privileged_mode_reason("agent-full-access").is_some(),
            "the mode that turns off Codex's sandbox must be gated"
        );
    }

    #[test]
    fn narrower_modes_are_not_gated() {
        // Gating these too would train the user to click through the prompt,
        // which is how a confirmation stops meaning anything.
        let a = agent("codex");
        for mode in ["read-only", "agent"] {
            assert!(
                a.privileged_mode_reason(mode).is_none(),
                "{mode} should not require confirmation"
            );
        }
    }

    #[test]
    fn an_unknown_mode_is_not_gated() {
        // A mode id we have never heard of is not assumed privileged. If a
        // vendor ships a new unrestricted mode, the safe default would be to
        // gate everything unknown — but that would break agents whose modes
        // are all innocuous, so this stays permissive and the audit trail in
        // `AcpModeChanged` is the backstop.
        let a = agent("codex");
        assert!(a.privileged_mode_reason("some-future-mode").is_none());
    }

    #[test]
    fn drivers_with_no_privileged_modes_never_gate() {
        for kind in ["claude-code", "opencode", "grok", "cursor", "antigravity"] {
            let a = agent(kind);
            for mode in ["code", "ask", "architect", "auto", "read-only"] {
                assert!(
                    a.privileged_mode_reason(mode).is_none(),
                    "{kind}/{mode} should not require confirmation"
                );
            }
        }
    }

    #[test]
    fn an_unknown_driver_gates_nothing_rather_than_panicking() {
        let a = agent("not-a-real-agent");
        assert!(a.privileged_mode_reason("agent-full-access").is_none());
    }

    #[test]
    fn every_gated_mode_explains_itself() {
        // An empty confirmation prompt is worse than none: the user cannot
        // decide, so they either click through or the feature is unusable.
        for kind in [
            "claude-code",
            "opencode",
            "codex",
            "grok",
            "cursor",
            "antigravity",
        ] {
            let a = agent(kind);
            for mode in ["agent-full-access", "code", "ask", "auto"] {
                if let Some(reason) = a.privileged_mode_reason(mode) {
                    assert!(!reason.trim().is_empty(), "{kind}/{mode} has no reason");
                }
            }
        }
    }

    #[test]
    fn the_default_permission_mode_is_the_conservative_one() {
        // Anything reaching for `Default` (a probe, a call-site that forgot
        // to pick) must get `Ask`, never auto-approval.
        assert_eq!(PermissionMode::default(), PermissionMode::Ask);
    }

    /// OpenCode's question form: a single choice with a free-text twin,
    /// and a multi-select.
    fn opencode_form() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "q0": {
                    "type": "string", "title": "Approach", "description": "Which approach?",
                    "oneOf": [
                        {"const": "Rewrite", "title": "Rewrite", "description": "Start over"},
                        {"const": "Patch", "title": "Patch"}
                    ]
                },
                "q0_custom": {"type": "string", "title": "Approach (other)", "description": "Type your own answer"},
                "q1": {
                    "type": "array", "title": "Targets", "description": "Which targets?",
                    "items": {"anyOf": [{"const": "web", "title": "Web"}, {"const": "ios", "title": "iOS"}]}
                }
            },
            "required": []
        })
    }

    #[test]
    fn an_elicitation_form_becomes_card_questions() {
        let f = form_fields("Questions", &opencode_form());
        assert_eq!(f.len(), 2, "the free-text twin folds into its field");
        assert_eq!(f[0].question.question, "Which approach?");
        assert_eq!(f[0].question.header.as_deref(), Some("Approach"));
        assert_eq!(f[0].question.options.len(), 2);
        assert_eq!(f[0].custom_key.as_deref(), Some("q0_custom"));
        assert!(f[1].question.multi_select);
        assert_eq!(f[1].question.options[1].label, "iOS");
    }

    #[test]
    fn card_answers_fill_the_form() {
        use mira_tools::prompt::AskUserAnswer;
        let f = form_fields("Questions", &opencode_form());
        let content = form_content(
            &f,
            &[
                AskUserAnswer {
                    picked: vec![],
                    custom: Some("Both, carefully".into()),
                },
                AskUserAnswer {
                    picked: vec!["Web".into(), "iOS".into()],
                    custom: None,
                },
            ],
        );
        assert_eq!(content["q0_custom"], "Both, carefully");
        assert!(content.get("q0").is_none());
        assert_eq!(content["q1"], serde_json::json!(["web", "ios"]));
        let picked = form_content(
            &f,
            &[
                AskUserAnswer {
                    picked: vec!["Patch".into()],
                    custom: None,
                },
                AskUserAnswer {
                    picked: vec![],
                    custom: None,
                },
            ],
        );
        assert_eq!(picked["q0"], "Patch");
        assert!(picked.get("q1").is_none());
    }
}

#[cfg(test)]
mod plan_tests {
    use super::plan_from_markdown;

    #[test]
    fn an_agents_markdown_plan_becomes_plan_steps() {
        let p = plan_from_markdown(
            "# Fix login\n\nContext line.\n\n1. Read auth.rs\n2. Patch the check\n- Run tests\n",
        );
        assert_eq!(p.title, "Fix login");
        let steps: Vec<_> = p.steps.iter().map(|s| s.description.as_str()).collect();
        assert_eq!(steps, ["Read auth.rs", "Patch the check", "Run tests"]);
    }

    #[test]
    fn a_plan_without_a_list_keeps_every_paragraph() {
        let p = plan_from_markdown("First do this.\n\nThen that.");
        assert_eq!(p.title, "Agent plan");
        assert_eq!(p.steps.len(), 2, "nothing the agent wrote is dropped");
    }
}

#[cfg(test)]
mod resolve_start_tests {
    use super::*;

    fn recorded(kind: &str) -> AcpLaunchParams {
        AcpLaunchParams::for_instance(
            kind,
            kind,
            DriverConfig {
                display_name: Some("Codex (work)".to_string()),
                home_path: Some(std::path::PathBuf::from("/tmp/codex-home")),
                ..Default::default()
            },
        )
    }

    fn instance_start(instance: &str, kind: &str, cfg: DriverConfig) -> InstanceStart {
        InstanceStart {
            instance: instance.to_string(),
            kind: kind.to_string(),
            cfg,
        }
    }

    #[test]
    fn launch_settings_survive_a_reload_without_their_secrets() {
        let mut p = AcpLaunchParams::new(
            "claude-code",
            DriverConfig {
                display_name: Some("Claude (work)".into()),
                binary_path: Some("/opt/claude".into()),
                api_key: Some("sk-secret".into()),
                env: [
                    ("ANTHROPIC_API_KEY".to_string(), "sk-env".to_string()),
                    ("CLAUDE_PROFILE".to_string(), "work".to_string()),
                ]
                .into_iter()
                .collect(),
                ..Default::default()
            },
        );
        p.model = Some("opus".into());
        p.mode_id = Some("acceptEdits".into());
        let persisted = p.to_persisted();
        let text = persisted.to_string();
        assert!(!text.contains("sk-secret"), "the api key is never written");
        assert!(
            !text.contains("sk-env"),
            "credential-named env vars are never written"
        );

        let back = AcpLaunchParams::from_meta(&mira_harness::persist::AgentSessionMeta {
            driver_kind: "claude-code".into(),
            instance: None,
            model: None,
            active: true,
            launch: Some(persisted),
        });
        assert_eq!(back.driver_kind, "claude-code");
        assert_eq!(back.display_name(), "Claude (work)");
        assert_eq!(
            back.cfg.binary_path.as_deref(),
            Some(std::path::Path::new("/opt/claude"))
        );
        assert_eq!(
            back.cfg.env.get("CLAUDE_PROFILE").map(String::as_str),
            Some("work")
        );
        assert_eq!(back.model.as_deref(), Some("opus"));
        assert_eq!(back.mode_id.as_deref(), Some("acceptEdits"));
        assert!(
            back.same_agent(&p) || p.cfg.api_key.is_some(),
            "only the secrets differ"
        );
    }

    #[test]
    fn a_model_change_is_not_a_different_agent() {
        let a = recorded("codex");
        let mut b = recorded("codex");
        b.model = Some("gpt-5".into());
        assert!(
            a.same_agent(&b),
            "picking a model must not cost the running process"
        );
        assert!(!a.same_agent(&recorded("claude-code")));
    }

    #[test]
    fn a_new_session_inherits_the_previous_sessions_settings() {
        // No driver, no instance: the bare `AcpStart` the UI sends for a
        // fresh chat resolves to exactly what the last chat ran.
        let r = resolve_start_params(Some(recorded("codex")), None, None).unwrap();
        assert_eq!(r.kind, "codex");
        assert_eq!(r.instance, "codex");
        assert_eq!(r.cfg.display_name.as_deref(), Some("Codex (work)"));
        assert_eq!(
            r.cfg.home_path.as_deref(),
            Some(std::path::Path::new("/tmp/codex-home"))
        );
    }

    #[test]
    fn naming_the_same_driver_still_uses_the_recorded_settings() {
        let r = resolve_start_params(Some(recorded("codex")), None, Some("codex")).unwrap();
        assert_eq!(r.kind, "codex");
        assert_eq!(r.instance, "codex");
        assert_eq!(r.cfg.display_name.as_deref(), Some("Codex (work)"));
    }

    #[test]
    fn naming_a_different_driver_starts_that_driver_fresh() {
        // The user picked a different agent in the picker: last chat's
        // config must not leak into it.
        let r = resolve_start_params(Some(recorded("codex")), None, Some("claude-code")).unwrap();
        assert_eq!(r.kind, "claude-code");
        assert_eq!(r.instance, "claude-code");
        assert_eq!(
            r.cfg.display_name, None,
            "fresh driver gets a default config"
        );
    }

    #[test]
    fn an_engine_instance_wins_over_everything() {
        let cfg = DriverConfig {
            display_name: Some("From registry".to_string()),
            ..Default::default()
        };
        let r = resolve_start_params(
            Some(recorded("codex")),
            Some(instance_start("codex-work", "codex", cfg)),
            Some("codex"),
        )
        .unwrap();
        assert_eq!(r.kind, "codex");
        assert_eq!(r.instance, "codex-work");
        assert_eq!(r.cfg.display_name.as_deref(), Some("From registry"));
    }

    #[test]
    fn a_bare_driver_routes_at_its_default_instance() {
        // Legacy path: no instance id anywhere, so the instance *is* the
        // kind. Cursors and spend keyed by instance keep working because
        // the key is unchanged.
        let r = resolve_start_params(None, None, Some("grok")).unwrap();
        assert_eq!(r.kind, "grok");
        assert_eq!(r.instance, "grok");
    }

    #[test]
    fn nothing_recorded_and_nothing_named_is_an_honest_error() {
        let e = resolve_start_params(None, None, None).unwrap_err();
        assert!(e.contains("no agent is configured"), "{e}");
        // And a bare driver still works the legacy way.
        let r = resolve_start_params(None, None, Some("grok")).unwrap();
        assert_eq!(r.kind, "grok");
    }

    #[test]
    fn same_agent_compares_instances_not_just_config() {
        // Two instances may share every launch field while billing
        // different accounts: sharing a process would cross the streams.
        let mut a = recorded("codex");
        a.instance = "codex".into();
        let mut b = recorded("codex");
        b.instance = "codex-work".into();
        assert!(
            !a.same_agent(&b),
            "different instances must not share a process"
        );
        b.instance = "codex".into();
        assert!(a.same_agent(&b), "same instance still reuses the process");
    }

    #[test]
    fn instance_survives_a_reload_without_its_secrets() {
        let mut p = AcpLaunchParams::for_instance(
            "codex-work",
            "codex",
            DriverConfig {
                home_path: Some(std::path::PathBuf::from("/tmp/codex-home")),
                api_key: Some("sk-secret".into()),
                ..Default::default()
            },
        );
        p.model = Some("gpt-5".into());
        let back = AcpLaunchParams::from_meta(&mira_harness::persist::AgentSessionMeta {
            driver_kind: "codex".into(),
            instance: None,
            model: None,
            active: true,
            launch: Some(p.to_persisted()),
        });
        assert_eq!(back.instance, "codex-work");
        assert_eq!(back.driver_kind, "codex");
        // And a record from before instances existed reloads at the
        // default instance, which is the kind.
        let legacy = AcpLaunchParams::from_meta(&mira_harness::persist::AgentSessionMeta {
            driver_kind: "codex".into(),
            instance: None,
            model: None,
            active: true,
            launch: None,
        });
        assert_eq!(legacy.instance, "codex");
    }
}

#[cfg(test)]
mod native_permission_view_tests {
    use super::*;

    fn perm(tool: &str, input: serde_json::Value) -> mira_acp::native::NativePermission {
        mira_acp::native::NativePermission {
            request_id: "r".into(),
            tool_name: tool.into(),
            input,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn pending_command_resumes_when_chat_switches_to_auto_everything() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let automatic = AtomicBool::new(false);
        let changes = tokio::sync::Notify::new();
        let wait = wait_native_approval(
            std::future::pending(),
            &changes,
            || {
                std::future::ready(if automatic.load(Ordering::SeqCst) {
                    mira_policy::Mode::Edit
                } else {
                    mira_policy::Mode::Auto
                })
            },
            "Bash",
        );
        tokio::pin!(wait);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), &mut wait)
                .await
                .is_err()
        );
        automatic.store(true, Ordering::SeqCst);
        changes.notify_waiters();
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(1), wait)
                .await
                .unwrap(),
            (true, true)
        );
    }

    #[tokio::test]
    async fn automatic_posture_keeps_questions_interactive() {
        let changes = tokio::sync::Notify::new();
        assert_eq!(
            wait_native_approval(
                std::future::ready(false),
                &changes,
                || std::future::ready(mira_policy::Mode::Edit),
                "request_user_input"
            )
            .await,
            (false, false)
        );
    }

    #[test]
    fn codex_picker_and_host_gate_have_the_same_posture() {
        use mira_policy::Mode;
        assert_eq!(codex_approval_mode("auto"), Some(Mode::Edit));
        assert_eq!(codex_approval_mode("auto-accept-edits"), Some(Mode::Auto));
        assert_eq!(codex_approval_mode("approval-required"), Some(Mode::Manual));
        assert_eq!(codex_approval_mode("plan"), Some(Mode::Plan));
        assert_eq!(codex_approval_mode("full-access"), Some(Mode::Yolo));
        assert_eq!(codex_approval_mode("unknown"), None);
    }

    #[test]
    fn native_modes_gate_actions_but_never_answer_questions_or_plans() {
        use mira_policy::Mode;
        for tool in [
            "shell",
            "Bash",
            "apply_patch",
            "Edit",
            "Write",
            "permission",
            "unknown",
        ] {
            assert!(!auto_approve_native_permission(Mode::Manual, tool));
            assert!(!auto_approve_native_permission(Mode::Plan, tool));
            assert!(auto_approve_native_permission(Mode::Edit, tool));
            assert!(auto_approve_native_permission(Mode::Yolo, tool));
        }
        for tool in [
            "apply_patch",
            "patch",
            "file_change",
            "Edit",
            "Write",
            "MultiEdit",
            "NotebookEdit",
        ] {
            assert!(auto_approve_native_permission(Mode::Auto, tool));
        }
        for tool in ["shell", "Bash", "permission", "unknown"] {
            assert!(!auto_approve_native_permission(Mode::Auto, tool));
        }
        for mode in [Mode::Manual, Mode::Auto, Mode::Edit, Mode::Yolo] {
            for tool in ["AskUserQuestion", "request_user_input", "ExitPlanMode"] {
                assert!(!auto_approve_native_permission(mode, tool));
            }
        }
    }

    #[test]
    fn shell_string_input_renders_as_command() {
        let (_name, args) =
            native_permission_tool_view(&perm("shell", serde_json::json!("ls -la")));
        assert_eq!(args, serde_json::json!({ "command": "ls -la" }));
    }

    #[test]
    fn patch_metadata_only_does_not_render_protocol_json() {
        let p = mira_acp::native::NativePermission {
            request_id: "r".into(),
            tool_name: "apply_patch".into(),
            input: serde_json::json!({ "threadId": "t", "turnId": "u", "itemId": "i" }),
            reason: Some("*** Begin Patch\n*** Update File: README.md\n*** End Patch".into()),
            ..Default::default()
        };
        let (name, args) = native_permission_tool_view(&p);
        assert_eq!(name, "apply_patch");
        assert!(args.get("threadId").is_none());
        assert!(args
            .get("patch")
            .and_then(|v| v.as_str())
            .unwrap()
            .contains("README.md"));
    }
}
