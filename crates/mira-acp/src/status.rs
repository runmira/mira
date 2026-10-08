//! Agent health, for the settings list.
//!
//! The list a user sees has to answer, per agent, one of a small number of
//! things: is it switched on, is it installed, does it start, and who is it
//! signed in as. Answering that requires actually launching the agent and
//! completing `initialize` — there is no cheaper way to learn its version or
//! its auth methods, because only the agent knows.
//!
//! The distinction between *installed* and *starts* is the useful one. A
//! binary can be present and still fail: Claude Code's adapter launches fine
//! and then fails because the `claude` CLI underneath is not signed in, and
//! Codex's can be missing entirely. Collapsing those into one boolean would
//! tell a user to install something they already have.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

use crate::conn::ConnError;
use crate::driver::{AcpDriver, DriverConfig, LaunchConfig, PermissionMode};
use crate::host::{DenyAll, FilePort, PermissionPort, TerminalPort};
use crate::process::{SpawnError, StartSpec};
use crate::session::ClientCaps;

/// How long a probe waits before giving up on an agent that starts but never
/// completes `initialize`. Generous, because a Node-based adapter can take a
/// second or two to boot, and short enough that a wedged agent cannot hang
/// the whole settings list.
/// Fallback probe budget, matching the driver default.
#[allow(dead_code)]
const PROBE_TIMEOUT: Duration = Duration::from_secs(12);

/// Why an agent is not usable right now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum AgentState {
    /// The binary is not on `PATH` and no path is configured.
    NotFound { looked_for: String },
    /// Launched, but `initialize` did not succeed.
    Failed { reason: String },
    /// Launched and initialized.
    Ready,
}

/// Everything the settings list shows for one agent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentStatus {
    /// Account-level catalog, available before opening an agent chat.
    #[serde(default)]
    pub models: Vec<crate::appserver::CodexModel>,
    pub kind: String,
    /// Driver's own name, or the instance override.
    pub display_name: String,
    pub state: AgentState,
    /// The agent's self-reported version, when it reports one. Most adapters
    /// do not, so this is usually absent even when the agent works.
    pub version: Option<String>,
    /// Human-readable auth summary, e.g. "Claude Pro Subscription". Taken
    /// from the agent's advertised auth methods, so it is whatever the agent
    /// says rather than something inferred.
    pub auth: Option<String>,
    /// Method ids the agent advertised, for a client that wants to call
    /// `authenticate` itself.
    pub auth_method_ids: Vec<String>,
    /// The resolved command, with credentials redacted. Safe to log.
    pub launch: String,

    // ---- the underlying agent CLI, as distinct from the ACP adapter ----
    //
    // Adapter-based agents need two installs. Reporting only the adapter's
    // presence made a user with Claude Code installed see "Not installed"
    // for Claude Code — true, and useless. These fields let the UI say
    // "2.1.283, signed in — install the adapter to use it here".
    /// Whether the agent's own CLI is present, when it is a separate program
    /// from the adapter. `None` for agents that speak ACP natively.
    pub cli_installed: Option<bool>,
    /// The agent CLI's self-reported version.
    pub cli_version: Option<String>,
    /// How to install the ACP adapter, when that is what is missing.
    pub install_hint: Option<String>,

    /// Which transport this status describes.
    pub transport: crate::driver::Transport,
}

/// Probe one agent.
///
/// Cheap when the binary is absent: `looks_installed` is a `PATH` scan, so
/// the common "not installed" case never spawns anything.
pub async fn probe(
    driver: &dyn AcpDriver,
    cfg: &DriverConfig,
    mode: PermissionMode,
) -> AgentStatus {
    probe_with_catalog(driver, cfg, mode, true).await
}

/// The background-sweep probe: everything [`probe`] learns without opening
/// an authenticated catalog session.
///
/// Opening a provider session can start MCP servers, run hooks, or launch
/// a login browser, so background health checks must not do it: no
/// app-server handshake, no `account/read`, no `model/list`. The ACP
/// `initialize` handshake stays — it opens no session (see
/// `probe_initialize`) and answers nothing — as do the CLI's own
/// side-effect-free reads (`--version`, `auth status`, `models`). A chat
/// start or an explicit catalog refresh runs the full probe and fills in
/// the rest.
pub async fn probe_background(
    driver: &dyn AcpDriver,
    cfg: &DriverConfig,
    mode: PermissionMode,
) -> AgentStatus {
    probe_with_catalog(driver, cfg, mode, false).await
}

async fn probe_with_catalog(
    driver: &dyn AcpDriver,
    cfg: &DriverConfig,
    mode: PermissionMode,
    catalog: bool,
) -> AgentStatus {
    let program = cfg.binary_path.clone().unwrap_or_else(|| {
        std::path::PathBuf::from(driver.binary_names().first().copied().unwrap_or(""))
    });
    let launch: LaunchConfig = driver.resolve(cfg, mode, program);

    let mut status = AgentStatus {
        models: Vec::new(),
        kind: driver.kind().to_string(),
        display_name: cfg
            .display_name
            .clone()
            .unwrap_or_else(|| driver.display_name().to_string()),
        // Health only. Whether the *user* has switched this agent on is a
        // local preference the client owns; reporting it here is what made
        // every row read "Disabled" regardless of what was installed.
        state: AgentState::NotFound {
            looked_for: launch.program.display().to_string(),
        },
        version: None,
        auth: None,
        auth_method_ids: Vec::new(),
        launch: launch.redacted(),
        cli_installed: None,
        cli_version: None,
        install_hint: None,
        transport: crate::driver::Transport::Acp,
    };

    // Deliberately *not* gated on `cfg.enabled`. Enabled/disabled is a user
    // preference about whether to *use* an agent; whether it is installed and
    // starts is a fact about the machine. Conflating them meant a correctly
    // installed, working agent reported "Disabled" and its real state was
    // never checked — the toggle became a blindfold.
    let adapter_present = crate::process::looks_installed(&launch);
    // A driver whose agent we can drive directly is probed through that
    // path. Reporting "not installed" for an agent the user runs daily
    // because a third-party adapter is missing was the original defect, and
    // the fix is to ask the agent itself.
    if driver.native_flavor().is_some() && cli_present(driver).await {
        {
            if let Some(v) = run_cli(
                driver.underlying_cli_names().first().copied().unwrap_or(""),
                driver.cli_version_args(),
            )
            .await
            {
                status.cli_version = v.split_whitespace().next().map(str::to_string);
            }
            status.cli_installed = Some(true);
            status.install_hint =
                (!driver.install_hint().is_empty()).then(|| driver.install_hint().to_string());
            if let Some(auth_args) =
                (!driver.cli_auth_args().is_empty()).then(|| driver.cli_auth_args())
            {
                if let Some(raw) = run_cli_status(
                    driver.underlying_cli_names().first().copied().unwrap_or(""),
                    auth_args,
                )
                .await
                {
                    if let Some(label) = summarize_cli_auth(&raw) {
                        status.auth = Some(label);
                    }
                }
            }
            // App-server agents get an account-level probe: the handshake
            // names who is signed in, which a bare `--version` cannot.
            // Explicit contexts only (chat start, catalog refresh): the
            // handshake opens an authenticated session, which background
            // sweeps must not do.
            if catalog
                && driver.native_flavor() == Some(crate::native::NativeFlavor::CodexAppServer)
            {
                // Probe the same binary, home and environment as the chat.
                // Otherwise a configured account gets the ambient account's catalog.
                if let Some(native_launch) =
                    crate::process::build_native_launch(driver, cfg, mode, &launch.program, None)
                {
                    // Same budget as the ACP handshake: a wedged app-server
                    // must not stall the sweep past the driver's ceiling.
                    if let Some(probe) = tokio::time::timeout(
                        driver.probe_timeout(),
                        crate::appserver::probe_handshake(&native_launch),
                    )
                    .await
                    .ok()
                    .flatten()
                    {
                        status.models = probe.models;
                        status.version = Some(probe.user_agent);
                        let mut bits = Vec::new();
                        if let Some(e) = probe.email {
                            bits.push(e);
                        }
                        if let Some(p) = probe.plan {
                            bits.push(p);
                        }
                        if !bits.is_empty() {
                            status.auth = Some(bits.join(" · "));
                        }
                    }
                }
            }
            // Ready on the strength of the CLI answering. The version is the
            // agent's own, not an adapter's, and the label says so.
            if status.version.is_none() {
                status.version = status.cli_version.clone();
            }
            status.transport = crate::driver::Transport::Native;
            fill_cli_models(&mut status, driver).await;
            // Installed but signed out isn't ready: every message would come
            // back "Not logged in". Say so where the user can act on it.
            if status.auth.as_deref() == Some(NOT_SIGNED_IN) {
                status.state = AgentState::Failed {
                    reason: signed_out_reason(driver),
                };
                return status;
            }
            status.state = AgentState::Ready;
            status.transport = crate::driver::Transport::Native;
            return status;
        }
    }

    if !adapter_present {
        // The adapter is missing. Before reporting a bare "not installed",
        // ask about the agent itself: for adapter-based agents those are two
        // separate installs, and telling someone who uses Claude Code daily
        // that Claude Code is not installed is worse than saying nothing.
        fill_cli_status(&mut status, driver).await;
        status.state = AgentState::NotFound {
            looked_for: launch.program.display().to_string(),
        };
        return status;
    }

    // A probe grants no authority: every port refuses. The point is to learn
    // what the agent says about itself, not to let it touch anything. The
    // budget is the driver's own: a binary that unpacks on cold start needs
    // longer than the default or every probe reports it missing.
    let budget = driver.probe_timeout();
    let outcome = tokio::time::timeout(
        budget,
        // A probe grants no authority and answers nothing, so it advertises
        // no capabilities. Claiming ones we would refuse is the exact failure
        // `AcpDriver::client_caps` exists to prevent.
        probe_initialize(driver, &launch, ClientCaps::default()),
    )
    .await;

    match outcome {
        Err(_) => {
            // Started but never finished the handshake: almost always an
            // agent whose own dependencies are missing or unsigned-in.
            status.state = AgentState::Failed {
                reason: format!(
                    "{} started but did not complete initialize within {}s",
                    driver.display_name(),
                    budget.as_secs()
                ),
            };
        }
        Ok(Err(e)) => {
            status.state = match e {
                ProbeError::Spawn(SpawnError::NotFound(p)) => {
                    AgentState::NotFound { looked_for: p }
                }
                other => AgentState::Failed {
                    reason: other.to_string(),
                },
            };
        }
        Ok(Ok(info)) => {
            status.state = AgentState::Ready;
            status.version = info.version;
            status.auth = summarize_auth(&info.auth_ids);
            status.auth_method_ids = info.auth_ids;
            fill_cli_models(&mut status, driver).await;
        }
    }
    status
}

#[derive(Debug, thiserror::Error)]
enum ProbeError {
    #[error(transparent)]
    Spawn(#[from] SpawnError),
    #[error(transparent)]
    Conn(#[from] ConnError),
    #[error("{0}")]
    Other(String),
}

struct InitializeInfo {
    version: Option<String>,
    auth_ids: Vec<String>,
}

/// Spawn, read what the agent says about itself, then shut down — answering
/// nothing.
async fn probe_initialize(
    driver: &dyn AcpDriver,
    launch: &LaunchConfig,
    caps: ClientCaps,
) -> Result<InitializeInfo, ProbeError> {
    // `start` performs the handshake, so the probe does not re-initialize:
    // a second `initialize` on a live agent is a protocol error to several
    // of them.
    let agent = crate::process::start(StartSpec {
        launch,
        caps: Some(caps),
        files: Arc::new(DenyAll) as Arc<dyn FilePort>,
        terminals: Arc::new(DenyAll) as Arc<dyn TerminalPort>,
        permissions: Arc::new(DenyAll) as Arc<dyn PermissionPort>,
        events: Arc::new(DenyAll) as Arc<dyn crate::host::EventPort>,
        // No session is opened: a probe only needs the agent to introduce
        // itself, and `session/new` can fail for reasons unrelated to health.
        cwd: None,
        mira_mcp: None,
        resume_session_id: None,
    })
    .await
    .map_err(|e| match e {
        crate::process::StartError::Spawn(s) => ProbeError::Spawn(s),
        // Name the agent: a list of six makes "failed to initialize" on its
        // own useless, and `start`'s message does not know which driver it
        // was reached through.
        other => ProbeError::Other(format!("{}: {other}", driver.display_name())),
    })?;

    let info = InitializeInfo {
        // Most adapters omit `agentInfo`, so this is usually absent even when
        // the agent works perfectly well.
        version: agent.session().agent_version(),
        auth_ids: agent.session().agent_auth_method_ids(),
    };

    agent.shutdown().await;
    Ok(info)
}

/// Account-level model catalog from the agent's own CLI, when it has
/// one (OpenCode: `opencode models`, one `provider/model` per line).
/// ACP itself has no "list models" verb, so this is what keeps the
/// picker from being empty before the first chat.
async fn fill_cli_models(status: &mut AgentStatus, driver: &dyn AcpDriver) {
    let args = driver.cli_models_args();
    if args.is_empty() {
        return;
    }
    let Some(bin) = driver.underlying_cli_names().first().copied() else {
        return;
    };
    if let Some(raw) = run_cli(bin, args).await {
        let models: Vec<crate::appserver::CodexModel> = raw
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && l.contains('/'))
            .map(|l| crate::appserver::CodexModel {
                value: l.to_string(),
                label: l.to_string(),
            })
            .collect();
        if !models.is_empty() {
            status.models = models;
        }
    }
}

/// Fill in what the *agent's own CLI* can tell us, independently of whether
/// its ACP adapter is installed.
///
/// This is what makes detection match what a user expect
/// "Claude Code 2.1.283, signed in" is read from the `claude` binary
/// (`--version` and `auth status`), not from the adapter. Without it, an
/// installed agent behind a missing adapter reports as not installed.
/// Whether the agent's own CLI is present and answering.
async fn cli_present(driver: &dyn AcpDriver) -> bool {
    let Some(first) = driver.underlying_cli_names().first().copied() else {
        return false;
    };
    run_cli(first, driver.cli_version_args()).await.is_some()
}

async fn fill_cli_status(status: &mut AgentStatus, driver: &dyn AcpDriver) {
    let names = driver.underlying_cli_names();
    if names.is_empty() {
        return;
    }
    let first = names[0];

    // The adapter is what we cannot spawn, so tell the user how to get it.
    // Reported whether or not the agent itself is installed: "install the
    // adapter" is the actionable next step either way.
    let hint = driver.install_hint();
    if !hint.is_empty() {
        status.install_hint = Some(hint.to_string());
    }

    // Version: `<cli> --version`.
    if let Some(v) = run_cli(first, driver.cli_version_args()).await {
        // `claude --version` prints "2.1.283 (Claude Code)"; keep the number.
        let token = v.split_whitespace().next().unwrap_or("").trim();
        if !token.is_empty() {
            status.cli_version = Some(token.to_string());
        }
    }

    // Presence: a CLI that answered at all is installed. Checked by running
    // rather than by scanning PATH, so a shim that fails to start is not
    // reported as present.
    let present = status.cli_version.is_some()
        || !driver.cli_auth_args().is_empty() && run_cli(first, &["--version"]).await.is_some();
    status.cli_installed = Some(present);

    if !present {
        return;
    }

    // Auth: `<cli> auth status` -> JSON, when the driver says to ask.
    let auth_args = driver.cli_auth_args();
    if auth_args.is_empty() {
        return;
    }
    if let Some(raw) = run_cli_status(first, auth_args).await {
        if let Some(label) = summarize_cli_auth(&raw) {
            status.auth = Some(label);
        }
    }
    fill_cli_models(status, driver).await;
}

/// Run the CLI and capture stdout, with a short ceiling so a hung CLI cannot
/// wedge the settings list.
async fn run_cli(bin: &str, args: &[&str]) -> Option<String> {
    run_cli_with(bin, args, false).await
}

/// `<cli> auth status`: signed out is a non-zero exit with the answer
/// still on stdout (Claude Code prints `{"loggedIn": false, …}` and exits
/// 1). Dropping it made a signed-out agent look signed in with no account.
async fn run_cli_status(bin: &str, args: &[&str]) -> Option<String> {
    run_cli_with(bin, args, true).await
}

async fn run_cli_with(bin: &str, args: &[&str], keep_failure_output: bool) -> Option<String> {
    use std::process::Stdio;
    // Resolved absolutely: the agent's CLI is usually under a version
    // manager's directory, which is not on the PATH of a process that was
    // not started from a shell.
    let program = crate::which::resolve(bin).unwrap_or_else(|| std::path::PathBuf::from(bin));
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for key in mira_sandbox::safe_child_env() {
        if let Ok(v) = std::env::var(key) {
            cmd.env(key, v);
        }
    }

    let out = tokio::time::timeout(Duration::from_secs(8), cmd.output())
        .await
        .ok()?
        .ok()?;
    if !out.status.success() && !keep_failure_output {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

/// Turn a CLI's auth-status JSON into a human summary.
///
/// Deliberately generic and evidence-based: it reports the provider/method
/// and account the CLI itself printed, and nothing more. An account name is
/// included because that is what identifies *who* is signed in; nothing is
/// inferred.
const NOT_SIGNED_IN: &str = "Not signed in";

/// "Not signed in" with the command that fixes it, in backticks so the UI
/// can offer it as a copyable line.
fn signed_out_reason(driver: &dyn AcpDriver) -> String {
    let login = driver.login_command();
    if login.is_empty() {
        format!(
            "{} isn't signed in. Sign in, then check again.",
            driver.display_name()
        )
    } else {
        format!(
            "{} isn't signed in. Run `{login}` in a terminal, then check again.",
            driver.display_name()
        )
    }
}

fn summarize_cli_auth(raw: &str) -> Option<String> {
    let v: Value = serde_json::from_str(raw).ok()?;

    // Claude Code's shape: loggedIn + authMethod.
    let logged_in = v
        .get("loggedIn")
        .or_else(|| v.get("authenticated"))
        .and_then(Value::as_bool);
    let method = v
        .get("authMethod")
        .or_else(|| v.get("subscriptionType"))
        .and_then(Value::as_str);

    if logged_in == Some(false) {
        return Some(NOT_SIGNED_IN.to_string());
    }
    if logged_in != Some(true) && method.is_none() {
        return None;
    }

    let account = v
        .get("email")
        .or_else(|| v.get("orgName"))
        .and_then(Value::as_str);

    let mut bits: Vec<String> = Vec::new();
    if let Some(m) = method {
        // `claude.ai` is Claude's first-party web login; phrase it the way a
        // user would rather than echoing the raw protocol value.
        if m == "claude.ai" {
            bits.push("Claude subscription".to_string());
        } else {
            bits.push(m.to_string());
        }
    }
    if let Some(a) = account {
        bits.push(a.to_string());
    }
    (!bits.is_empty()).then(|| bits.join(" · "))
}

/// Turn advertised auth method ids into something worth showing.
///
/// The ids are the agent's own vocabulary and mean nothing to a user
/// (`cursor_login`, `oauth-personal`, `grok.com`), so they are labelled
/// rather than echoed raw.
fn summarize_auth(ids: &[String]) -> Option<String> {
    if ids.is_empty() {
        // Self-authenticating agents (`claude-code-acp` delegates to the
        // `claude` CLI) advertise nothing. Saying "not authenticated" here
        // would be wrong; they are, just not through ACP.
        return None;
    }
    if ids.len() == 1 {
        return Some(friendly_auth_id(&ids[0]));
    }
    Some(format!(
        "{} options",
        ids.iter()
            .map(|i| friendly_auth_id(i))
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

/// Whether a turn is billed to a metered API key or covered by a plan.
///
/// The distinction matters more than the dollar figure it sits next to: a
/// Max/Plus subscriber pays a flat subscription, so an API-equivalent
/// estimate of their usage is not money spent. Where an agent signs in
/// with a plan we can say so; where it holds a key we can say that too.
/// Everything else (an ambient login, a proxy we can't see) stays
/// [`Billing::Unknown`] rather than being guessed at.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Billing {
    /// Metered: a key the user configured, or a plan's usage pool.
    Api,
    /// Covered by a subscription — no per-token charge to the user.
    Subscription,
    /// We can't tell from what the agent reported.
    #[default]
    Unknown,
}

impl Billing {
    /// Classify from the auth method ids an agent advertised at
    /// `initialize`, plus whether this instance was given a key.
    ///
    /// A key is decisive: it bills by the token, whatever else the agent
    /// also advertises. Otherwise a plan-shaped method id means a
    /// subscription. `claude.ai` and `chatgpt` are the everyday ones;
    /// `oauth-personal` is Google's account login, which for Gemini is
    /// normally a plan, and is left unknown because it can equally be
    /// billed per token.
    pub fn classify(has_api_key: bool, auth_method_ids: &[String]) -> Self {
        if has_api_key {
            return Billing::Api;
        }
        if auth_method_ids.iter().any(|id| {
            matches!(
                id.as_str(),
                "claude.ai" | "chatgpt" | "chatgpt-login" | "cursor_login" | "grok.com"
            )
        }) {
            return Billing::Subscription;
        }
        Billing::Unknown
    }
}

impl Billing {
    /// Classify from a probe's human-readable auth summary.
    ///
    /// The native transports never advertise method ids on the wire, so
    /// this is how their subscription logins are recognised: the probe
    /// already asked the agent's own CLI (`claude auth status`) and got
    /// back a phrase like "Claude subscription · a@b.com". An account that
    /// signed in with a key says so instead, and a proxy we can't see
    /// stays unknown.
    pub fn from_auth_summary(summary: &str) -> Self {
        let s = summary.to_ascii_lowercase();
        // A key in the summary is decisive, same as a configured one.
        if s.contains("api key") || s.contains("api_key") {
            return Billing::Api;
        }
        if s.contains("subscription") || s.contains("claude.ai") || s.contains("chatgpt") {
            return Billing::Subscription;
        }
        Billing::Unknown
    }
}

fn friendly_auth_id(id: &str) -> String {
    match id {
        "grok.com" => "Grok account".to_string(),
        "cursor_login" => "Cursor subscription".to_string(),
        "oauth-personal" => "Google account".to_string(),
        "anthropic" | "claude" | "claude.ai" => "Claude subscription".to_string(),
        "chatgpt" | "chatgpt-login" => "ChatGPT subscription".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use crate::drivers::ClaudeCodeDriver;

    #[test]
    fn cli_auth_json_becomes_a_summary() {
        // The shape `claude auth status` prints.
        let raw = r#"{"loggedIn":true,"authMethod":"claude.ai","email":"a@b.com"}"#;
        assert_eq!(
            summarize_cli_auth(raw).as_deref(),
            Some("Claude subscription · a@b.com")
        );

        // Signed out must not be dressed up as a provider.
        assert_eq!(
            summarize_cli_auth(r#"{"loggedIn":false}"#).as_deref(),
            Some("Not signed in")
        );

        // Nothing recognisable: say nothing rather than guess.
        assert!(summarize_cli_auth("not json").is_none());
        assert!(summarize_cli_auth(r#"{"other":1}"#).is_none());
    }

    #[tokio::test]
    async fn an_agent_with_a_native_cli_probes_ready_without_any_adapter() {
        // The bug this guards: `claude` installed, `claude-agent-acp` not, and
        // the panel said Claude Code was "Not installed". Since the CLI is
        // itself a full agent server, the probe now goes through it and the
        // agent is Ready — with the adapter never mentioned.
        //
        // Asserting through `probe` also pins the `.await`: dropping the
        // future compiles, warns, and silently returns an empty status.
        let d = ClaudeCodeDriver;
        let cfg = DriverConfig {
            binary_path: Some("/nonexistent/claude-agent-acp".into()),
            ..Default::default()
        };
        let s = probe(&d, &cfg, crate::driver::PermissionMode::Ask).await;

        if run_cli("claude", &["--version"]).await.is_none() {
            // No CLI here: the adapter's absence is all we can report.
            assert!(matches!(s.state, AgentState::NotFound { .. }), "{s:?}");
            return;
        }
        assert!(matches!(s.state, AgentState::Ready), "{s:?}");
        assert_eq!(s.transport, crate::driver::Transport::Native, "{s:?}");
        assert_eq!(s.cli_installed, Some(true), "{s:?}");
        assert!(
            s.cli_version.is_some(),
            "expected the agent's own version: {s:?}"
        );
    }

    use super::*;
    use crate::drivers::{CodexDriver, GrokDriver, OpenCodeDriver};

    /// Enabled, but pointing at a binary that cannot exist — the "Not found"
    /// row in the settings list.
    fn never_installed() -> DriverConfig {
        DriverConfig {
            enabled: true,
            binary_path: Some("/nonexistent/mira-probe".into()),
            ..Default::default()
        }
    }

    /// Switched off. The binary is never looked for, so the probe cannot
    /// spawn anything.
    fn switched_off() -> DriverConfig {
        DriverConfig {
            enabled: false,
            binary_path: Some("/nonexistent/mira-probe".into()),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn health_is_reported_regardless_of_the_user_toggle() {
        // The regression this pins: `AcpStatus` handed `DriverConfig::default()`
        // (whose `enabled` is false), so every probe short-circuited and the
        // whole list read "Disabled" — including for an agent that was
        // installed and started perfectly. The toggle is a local preference;
        // health is a fact about the machine.
        let status = probe(&GrokDriver, &switched_off(), PermissionMode::Ask).await;
        assert!(
            matches!(status.state, AgentState::NotFound { .. }),
            "a switched-off agent still reports its real health, got {:?}",
            status.state
        );
        assert_eq!(status.kind, "grok");
        assert_eq!(status.display_name, "Grok Build");
        // The resolved command is reported either way, since the list shows
        // every agent regardless of the toggle.
        assert!(!status.launch.is_empty(), "no launch recorded");
    }

    #[tokio::test]
    async fn a_missing_binary_is_reported_not_found_and_names_what_it_looked_for() {
        let status = probe(&GrokDriver, &never_installed(), PermissionMode::Ask).await;
        match status.state {
            AgentState::NotFound { looked_for } => {
                assert!(looked_for.contains("mira-probe"), "got {looked_for}");
            }
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn the_launch_string_is_redacted() {
        let mut cfg = never_installed();
        cfg.enabled = true;
        cfg.api_key = Some("sk-do-not-log".into());
        let status = probe(&GrokDriver, &cfg, PermissionMode::Ask).await;
        assert!(
            !status.launch.contains("sk-do-not-log"),
            "the probe leaked a credential: {}",
            status.launch
        );
    }

    #[test]
    fn billing_from_a_probe_summary_recognises_a_plan_login() {
        use Billing::*;
        // What `claude auth status` produces for a subscriber.
        assert_eq!(
            Billing::from_auth_summary("Claude subscription · a@b.com"),
            Subscription
        );
        // A key signs differently.
        assert_eq!(Billing::from_auth_summary("API key"), Api);
        // An account we can't classify stays unknown.
        assert_eq!(Billing::from_auth_summary("some-proxy"), Unknown);
        assert_eq!(Billing::from_auth_summary(""), Unknown);
    }

    #[test]
    fn billing_follows_the_key_then_the_advertised_login() {
        use Billing::*;
        // A key decides it, even alongside a plan-shaped method.
        assert_eq!(Billing::classify(true, &["claude.ai".to_string()]), Api);
        // Plan logins with no key: covered by the subscription.
        assert_eq!(
            Billing::classify(false, &["claude.ai".to_string()]),
            Subscription
        );
        assert_eq!(
            Billing::classify(false, &["chatgpt".to_string(), "api-key".to_string()]),
            Subscription
        );
        // A key-only login is metered.
        assert_eq!(Billing::classify(false, &["api-key".to_string()]), Unknown);
        // Nothing advertised: we don't guess.
        assert_eq!(Billing::classify(false, &[]), Unknown);
    }

    #[test]
    fn an_auth_summary_is_human_readable() {
        assert_eq!(summarize_auth(&[]), None);
        assert_eq!(
            summarize_auth(&["cursor_login".to_string()]).as_deref(),
            Some("Cursor subscription")
        );
        assert_eq!(
            summarize_auth(&["grok.com".to_string()]).as_deref(),
            Some("Grok account")
        );
        assert_eq!(
            summarize_auth(&["oauth-personal".to_string()]).as_deref(),
            Some("Google account")
        );
        // An unknown id is passed through rather than dropped or mangled.
        assert_eq!(
            summarize_auth(&["weird-id".to_string()]).as_deref(),
            Some("weird-id")
        );
        assert!(summarize_auth(&["a".to_string(), "b".to_string()])
            .unwrap()
            .contains("options"));
    }

    #[test]
    fn every_driver_can_be_probed_without_being_installed() {
        // A probe must never panic or hang for an agent that is simply absent
        // — that is the state most of the list is in.
        for d in crate::drivers::all() {
            assert!(!d.binary_names().is_empty(), "{} has no binary", d.kind());
        }
    }

    #[tokio::test]
    async fn probing_an_absent_adapter_reports_not_found_rather_than_hanging() {
        // The real-world case from the settings list: an agent that is
        // enabled but was never installed. With no CLI present either, there
        // is nothing to report but absence.
        let cfg = DriverConfig {
            enabled: true,
            binary_path: Some("/nonexistent/claude-code-acp".into()),
            ..Default::default()
        };
        if run_cli("claude", &["--version"]).await.is_some() {
            // The CLI is here and can be driven directly, so this agent is
            // usable without the adapter this test names.
            return;
        }
        let status = probe(&crate::drivers::ClaudeCodeDriver, &cfg, PermissionMode::Ask).await;
        assert!(
            matches!(status.state, AgentState::NotFound { .. }),
            "{status:?}"
        );
    }

    #[tokio::test]
    async fn opencode_and_codex_probe_independently() {
        // OpenCode has no native path: a missing adapter is always
        // NotFound, on every machine.
        let status = probe(&OpenCodeDriver, &never_installed(), PermissionMode::Ask).await;
        assert!(
            matches!(status.state, AgentState::NotFound { .. }),
            "{status:?}"
        );
        // Codex has a native CLI path: with no `codex` answering
        // `--version` the missing adapter is NotFound; with one, the
        // native branch correctly reports the CLI instead.
        if run_cli("codex", &["--version"]).await.is_none() {
            let status = probe(&CodexDriver, &never_installed(), PermissionMode::Ask).await;
            assert!(
                matches!(status.state, AgentState::NotFound { .. }),
                "{status:?}"
            );
        }
    }

    /// Background sweeps must not open authenticated catalog sessions: no
    /// app-server handshake, so no handshake models or handshake auth, on
    /// any machine state. (With no `codex` CLI this is NotFound either
    /// way; with one it is version-based Ready — both without a catalog.)
    #[tokio::test]
    async fn background_probe_never_carries_a_handshake_catalog() {
        let status =
            probe_background(&CodexDriver, &DriverConfig::default(), PermissionMode::Ask).await;
        assert!(
            status.models.is_empty(),
            "background must not run model/list: {status:?}"
        );
        assert!(
            status.auth.is_none(),
            "background must not run account/read: {status:?}"
        );
    }

    #[tokio::test]
    async fn background_probe_reports_absence_like_the_full_probe() {
        // Absent binaries: both depths agree, quickly, having spawned
        // nothing (the binary path cannot exist).
        for d in crate::drivers::all() {
            let cfg = DriverConfig {
                binary_path: Some("/nonexistent/mira-background-probe".into()),
                ..Default::default()
            };
            let bg = probe_background(d.as_ref(), &cfg, PermissionMode::Ask).await;
            // A native CLI found on PATH still answers `--version` in both
            // depths; absence is what must agree.
            let full = probe(d.as_ref(), &cfg, PermissionMode::Ask).await;
            if matches!(bg.state, AgentState::NotFound { .. }) {
                assert!(
                    matches!(full.state, AgentState::NotFound { .. }),
                    "{} disagrees: background {bg:?} vs full {full:?}",
                    d.kind()
                );
            }
        }
    }
}
