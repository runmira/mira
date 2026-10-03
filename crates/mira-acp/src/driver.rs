//! Per-agent launch configuration.
//!
//! ACP is one protocol, but every agent reaches it differently: a different
//! binary, a different subcommand, different flags for the same permission
//! concept, and a different answer to "how do I authenticate?". That
//! knowledge is what a driver is — the thin, per-vendor layer the research
//! recommends extracting. Everything here is pure data: no process is
//! spawned, so the whole module is unit-testable without an agent installed.
//!
//! The generic runtime above this knows nothing about any specific agent.
//! If you find yourself adding a `if driver == "grok"` somewhere else, the
//! thing you want belongs in here.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// How to reach an agent.
///
/// `Auto` is the default because both paths are legitimate and only one may
/// be present: it takes native when the agent's own CLI is there and the ACP
/// adapter otherwise.
///
/// Serialized because it reaches the UI, which has to name the transport it
/// is looking at rather than implying one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Transport {
    /// Native CLI if the driver has one and it is present, else ACP.
    #[default]
    Auto,
    /// The agent's own headless interface. Fails if unavailable.
    Native,
    /// Always the ACP adapter, even when a native path exists.
    Acp,
}

/// How much latitude the agent has to act without asking.
///
/// `Default` is `Ask`, and deliberately so: the first variant is the
/// conservative one, so anything that reaches for `Default` (a probe, a
/// call-site that forgot to pick a mode) gets the safe behaviour rather than
/// auto-approval.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PermissionMode {
    /// Ask before every consequential action.
    #[default]
    Ask,
    /// Auto-approve file edits and commands that stay inside the workspace.
    AcceptEdits,
    /// Approve everything. Only reachable when the user explicitly opts in.
    Auto,
}

impl PermissionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            PermissionMode::Ask => "ask",
            PermissionMode::AcceptEdits => "accept_edits",
            PermissionMode::Auto => "auto",
        }
    }

    /// Whether this mode is safe to enter without an explicit
    /// acknowledgement. `Auto` runs arbitrary commands, so callers that
    /// expose a mode picker should gate it on this.
    pub fn requires_explicit_opt_in(self) -> bool {
        matches!(self, PermissionMode::Auto)
    }
}

/// Whether the agent can run without an interactive sign-in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthShape {
    /// Sign-in happens in the agent's own process, surfaced over ACP as
    /// `authMethods`. Needs a browser, so no headless path exists.
    InteractiveBrowser,
    /// An API key in the environment is enough; no ACP `authenticate` call.
    EnvKey,
    /// Both are available and the driver picks via `api_key_env`.
    Either,
}

/// One resolved launch: exactly what gets handed to `tokio::process::Command`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchConfig {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    /// Env keys whose values must never be logged or echoed back.
    pub secret_env: Vec<String>,
    /// Env key prefixes stripped from the child environment at spawn.
    /// Unlike `secret_env` (redaction for display), these never reach the
    /// process at all — for credentials the agent must not inherit from
    /// the ambient environment.
    pub env_deny: Vec<String>,
}

impl LaunchConfig {
    /// A safe-to-log rendering. Values for `secret_env` keys are redacted so
    /// this can go in a log line or an error message.
    pub fn redacted(&self) -> String {
        let env = self
            .env
            .iter()
            .map(|(k, v)| {
                if self.secret_env.iter().any(|s| s == k) {
                    (k.clone(), "«redacted»".to_string())
                } else {
                    (k.clone(), v.clone())
                }
            })
            .collect::<BTreeMap<_, _>>();
        format!(
            "{} {}{}{}",
            self.program.display(),
            self.args.join(" "),
            if env.is_empty() { "" } else { " " },
            env.iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(" ")
        )
    }
}

/// Settings a user can change per driver instance.
#[derive(Clone, Debug, Default)]
pub struct DriverConfig {
    /// Whether this instance is switched on. A disabled agent stays listed
    /// and still reports its health, but starting a session with it is
    /// refused — which is what lets the settings list show every agent,
    /// installed or not.
    pub enabled: bool,
    /// Overrides the driver's own display name, for users who run two
    /// instances of the same agent with different configs.
    pub display_name: Option<String>,
    /// Overrides the auto-detected binary. `None` means "find it on PATH".
    pub binary_path: Option<PathBuf>,
    /// Extra arguments appended after the driver's own, verbatim.
    pub launch_args: Vec<String>,
    /// Compact the session once its context passes this many tokens.
    /// `None` keeps the agent's own default. Only meaningful for agents whose
    /// context Mira manages.
    pub auto_compact_after: Option<u64>,
    /// Home/config directory override, exported as the driver's env var
    /// (`CODEX_HOME`-style). Used to give each instance its own credentials.
    pub home_path: Option<PathBuf>,
    pub env: BTreeMap<String, String>,
    /// API key, for drivers whose auth is key-based.
    pub api_key: Option<String>,
    /// Effort level for agents that take one (`claude --effort
    /// low|medium|high|xhigh|max`). Ignored by drivers without the flag.
    pub effort: Option<String>,
    /// Setting sources for agents that take them (`claude
    /// --setting-sources user,project,local`). Ignored otherwise.
    pub setting_sources: Option<String>,
}

impl DriverConfig {
    /// Keys whose values are secret in this config: the driver's API-key
    /// env vars (when a key is actually set), plus any user-supplied env var
    /// whose *name* looks like a credential.
    pub fn secret_keys<D: AcpDriver + ?Sized>(&self, driver: &D) -> Vec<String> {
        let mut keys: Vec<String> = if self.api_key.is_some() {
            driver
                .api_key_env_vars()
                .iter()
                .map(|k| (*k).to_string())
                .collect()
        } else {
            Vec::new()
        };
        keys.extend(
            self.env
                .keys()
                .filter(|k| {
                    let l = k.to_ascii_lowercase();
                    l.contains("token") || l.contains("key") || l.contains("secret")
                })
                .cloned(),
        );
        keys.sort();
        keys.dedup();
        keys
    }
}

/// Everything that varies between ACP-speaking agents.
pub trait AcpDriver: Send + Sync {
    /// Stable slug, used in config and as the instance's driver kind.
    fn kind(&self) -> &'static str;

    fn display_name(&self) -> &'static str;

    /// The base command, relative to the configured or detected binary.
    /// Every ACP agent here is a subcommand of its CLI.
    fn base_args(&self) -> &'static [&'static str];

    /// Filenames to look for on `PATH` when no path is configured.
    /// First match wins; `None` means "use `PATH` lookup by `binary_name`".
    fn binary_names(&self) -> &'static [&'static str];

    fn auth_shape(&self) -> AuthShape;

    /// Env vars that, when present, let the agent skip `authenticate`.
    fn api_key_env_vars(&self) -> &'static [&'static str] {
        &[]
    }

    /// The ACP `authMethods[].id` this driver knows how to satisfy, when the
    /// agent advertises one. Used to match an advertised method to our
    /// knowledge of how to complete it.
    fn known_auth_method_id(&self) -> Option<&'static str> {
        None
    }

    /// Extra arguments for a permission mode.
    ///
    /// The default implementation is the ACP-native route
    /// (`--permission-mode`), which Grok honors; drivers whose agent takes it
    /// elsewhere override this.
    fn permission_args(&self, mode: PermissionMode) -> Vec<String> {
        vec!["--permission-mode".to_string(), mode.as_str().to_string()]
    }

    /// Session modes that grant *more* authority than Mira would.
    ///
    /// These are vendor-defined ids, so they cannot be detected by shape —
    /// a driver has to name them. A listed mode needs explicit confirmation
    /// from the user before it is applied, because these change what the
    /// agent may do for the rest of the session rather than for one turn.
    ///
    /// The clearest case is Codex's `agent-full-access`, which disables
    /// Codex's own sandbox. That is a wider grant than Mira's own approval
    /// pipeline would ever allow, so it must not be one click away.
    fn privileged_mode_ids(&self) -> &'static [&'static str] {
        &[]
    }

    /// Why a mode is privileged, for the confirmation prompt. A driver that
    /// lists a mode without a reason here still gets confirmation; this
    /// only makes the prompt informative.
    fn privileged_mode_reason(&self, mode_id: &str) -> Option<&'static str> {
        let _ = mode_id;
        None
    }

    /// The underlying agent CLI, when it is a *separate* program from the
    /// binary we spawn.
    ///
    /// Adapter-based agents need two installs: the agent itself, and the
    /// adapter that exposes it over ACP. Claude Code is the clearest case —
    /// `claude` is the agent, `claude-agent-acp` is the adapter, and only the
    /// second speaks the protocol. Without this distinction a user who has
    /// Claude Code installed sees "Not installed" for Claude Code, which is
    /// both true and useless.
    fn underlying_cli_names(&self) -> &'static [&'static str] {
        &[]
    }

    /// How to install the binary we spawn, for the "adapter missing" case.
    fn install_hint(&self) -> &'static str {
        ""
    }

    /// The command that signs the agent's CLI in, for the "installed but
    /// signed out" case. Empty when there isn't one to suggest.
    fn login_command(&self) -> &'static str {
        ""
    }

    /// Arguments that make the underlying CLI print its version.
    fn cli_version_args(&self) -> &'static [&'static str] {
        &["--version"]
    }

    /// Arguments that make the underlying CLI print auth status as JSON.
    ///
    /// Empty means "this CLI has no such command", and auth is then not
    /// probed. Claude Code has `auth status`, which returns `loggedIn`,
    /// `authMethod` and `email` — enough to say "signed in via claude.ai"
    /// without inventing anything.
    fn cli_auth_args(&self) -> &'static [&'static str] {
        &[]
    }

    /// Extra CLI flags derived from [`DriverConfig`], after the driver's
    /// own argv. Empty by default: most drivers need nothing beyond their
    /// base args plus the user's verbatim `launch_args`.
    fn cli_extras(&self, _cfg: &DriverConfig) -> Vec<String> {
        Vec::new()
    }

    /// Environment variable the agent reads its config/credentials dir from.
    fn home_env_var(&self) -> Option<&'static str> {
        None
    }

    /// Default config/credentials dir when the user set no `home_path`.
    ///
    /// `None` means "leave the agent's own default alone". `Some` isolates
    /// the agent into our dir — used where the agent otherwise shares
    /// ambient credentials we cannot scope.
    fn default_home(&self) -> Option<std::path::PathBuf> {
        None
    }

    /// Env key prefixes the child process must never inherit.
    ///
    /// Enforced at every spawn site, for every transport: an agent that
    /// bills to ambient credentials cannot be scoped, audited, or run as a
    /// second identity.
    fn env_deny_prefixes(&self) -> &'static [&'static str] {
        &[]
    }

    /// How long a health probe may take. The default covers adapters that
    /// start in a second or two; agents whose binaries unpack on cold start
    /// override it rather than flaking.
    fn probe_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(12)
    }

    /// The agent's own headless interface, when it has one.
    ///
    /// `None` means this agent can only be driven through an ACP adapter. A
    /// `Some` is a claim that we can run the agent the user already has
    /// installed, with no second install and no third-party shim — which is
    /// both simpler and more reliably detected.
    fn native_flavor(&self) -> Option<crate::native::NativeFlavor> {
        None
    }

    /// Which transport to use.
    ///
    /// `Auto` prefers native and falls back to the adapter, so an agent with
    /// both paths available works either way and nothing regresses for
    /// someone who already installed an adapter.
    fn transport(&self) -> Transport {
        Transport::Auto
    }

    /// Client capabilities this driver actually needs.
    ///
    /// Derived per driver rather than configured globally, because ACP
    /// capabilities are a promise: advertising a capability the client will
    /// not honour makes the agent plan around something that does not exist.
    ///
    /// Codex is the concrete case. It runs terminals in **its own** process
    /// and streams the output bytes to the client, rather than asking the
    /// client to run commands — the opposite of most agents. Advertising
    /// `terminal` to it would invite requests we would then have to service
    /// for an agent that does not want them.
    fn client_caps(&self) -> crate::session::ClientCaps {
        crate::session::ClientCaps {
            fs_read: true,
            fs_write: true,
            terminal: true,
            elicitation_form: true,
        }
    }

    /// Environment that carries the permission mode, for agents configured
    /// that way rather than by flags. Explicit user `env` still wins.
    fn permission_env(&self, _mode: PermissionMode) -> Vec<(String, String)> {
        Vec::new()
    }

    /// Resolve a user config into a concrete launch.
    fn resolve(&self, cfg: &DriverConfig, mode: PermissionMode, program: PathBuf) -> LaunchConfig {
        let mut args: Vec<String> = self.base_args().iter().map(|s| (*s).to_string()).collect();
        args.extend(self.permission_args(mode));
        args.extend(cfg.launch_args.iter().cloned());

        // Derived values are only defaults: an explicit entry in `env` is a
        // more specific instruction from the user and must win. `home_path`
        // and `api_key` populate the same keys, so without this a per-
        // instance home set via `env` would be silently clobbered.
        let mut env: BTreeMap<String, String> = cfg.env.clone();
        // An explicit home always wins. Otherwise a driver-declared default
        // isolates the agent; otherwise the agent's own default stands.
        let home = cfg.home_path.clone().or_else(|| self.default_home());
        if let (Some(var), Some(home)) = (self.home_env_var(), home.as_ref()) {
            env.entry(var.to_string())
                .or_insert_with(|| home.display().to_string());
        }
        if let (Some(var), Some(key)) = (self.api_key_env_vars().first(), cfg.api_key.as_ref()) {
            env.entry((*var).to_string()).or_insert_with(|| key.clone());
        }
        for (k, v) in self.permission_env(mode) {
            env.entry(k).or_insert(v);
        }

        LaunchConfig {
            program,
            args,
            env,
            secret_env: cfg.secret_keys(self),
            env_deny: self
                .env_deny_prefixes()
                .iter()
                .map(|s| s.to_string())
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake;

    impl AcpDriver for Fake {
        fn kind(&self) -> &'static str {
            "fake"
        }
        fn display_name(&self) -> &'static str {
            "Fake"
        }
        fn base_args(&self) -> &'static [&'static str] {
            &["agent", "stdio"]
        }
        fn binary_names(&self) -> &'static [&'static str] {
            &["fake"]
        }
        fn auth_shape(&self) -> AuthShape {
            AuthShape::EnvKey
        }
        fn api_key_env_vars(&self) -> &'static [&'static str] {
            &["FAKE_API_KEY"]
        }
        fn home_env_var(&self) -> Option<&'static str> {
            Some("FAKE_HOME")
        }
    }

    fn base_cfg() -> DriverConfig {
        DriverConfig::default()
    }

    #[test]
    fn base_args_and_permission_mode_are_ordered_base_first() {
        let cfg = Fake.resolve(
            &base_cfg(),
            PermissionMode::AcceptEdits,
            PathBuf::from("/usr/local/bin/fake"),
        );
        assert_eq!(
            cfg.args,
            vec!["agent", "stdio", "--permission-mode", "accept_edits"]
        );
    }

    #[test]
    fn launch_args_come_after_our_own_so_users_can_override() {
        let mut c = base_cfg();
        c.launch_args = vec!["--profile".into(), "work".into()];
        let cfg = Fake.resolve(&c, PermissionMode::Ask, PathBuf::from("fake"));
        assert_eq!(
            cfg.args,
            vec![
                "agent",
                "stdio",
                "--permission-mode",
                "ask",
                "--profile",
                "work"
            ]
        );
    }

    #[test]
    fn api_key_reaches_the_env_the_driver_declares() {
        let mut c = base_cfg();
        c.api_key = Some("sk-secret".into());
        let cfg = Fake.resolve(&c, PermissionMode::Ask, PathBuf::from("fake"));
        assert_eq!(
            cfg.env.get("FAKE_API_KEY").map(String::as_str),
            Some("sk-secret")
        );
    }

    #[test]
    fn home_path_becomes_the_declared_env_var() {
        let mut c = base_cfg();
        c.home_path = Some(PathBuf::from("/tmp/fake-home"));
        let cfg = Fake.resolve(&c, PermissionMode::Ask, PathBuf::from("fake"));
        assert_eq!(
            cfg.env.get("FAKE_HOME").map(String::as_str),
            Some("/tmp/fake-home")
        );
    }

    #[test]
    fn explicit_env_wins_over_the_derived_default() {
        // `home_path` and `env` write the same key. The explicit `env` entry
        // is the more specific instruction, so it must survive.
        let mut c = base_cfg();
        c.env.insert("FAKE_HOME".into(), "/explicit".into());
        c.home_path = Some(PathBuf::from("/derived"));
        let cfg = Fake.resolve(&c, PermissionMode::Ask, PathBuf::from("fake"));
        assert_eq!(
            cfg.env.get("FAKE_HOME").map(String::as_str),
            Some("/explicit")
        );
    }

    #[test]
    fn redacted_hides_secrets_but_keeps_the_shape() {
        let mut c = base_cfg();
        c.api_key = Some("sk-super-secret".into());
        c.env.insert("EXTRA".into(), "visible".into());
        let cfg = Fake.resolve(&c, PermissionMode::Ask, PathBuf::from("/bin/fake"));
        let out = cfg.redacted();
        assert!(!out.contains("sk-super-secret"), "secret leaked: {out}");
        assert!(out.contains("«redacted»"));
        assert!(out.contains("EXTRA=visible"));
    }

    #[test]
    fn auto_mode_is_flagged_as_needing_explicit_opt_in() {
        assert!(PermissionMode::Auto.requires_explicit_opt_in());
        assert!(!PermissionMode::AcceptEdits.requires_explicit_opt_in());
        assert!(!PermissionMode::Ask.requires_explicit_opt_in());
    }
}
