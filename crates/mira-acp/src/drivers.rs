//! The ACP-speaking agents Mira ships support for.
//!
//! Facts here come from the ACP registry (`registry/*/agent.json`) and each
//! vendor's own docs. Two are worth flagging because they contradict the
//! obvious guess:
//!
//! * **Antigravity is not the `agy` CLI.** Google ships a *separate*
//!   server binary, `agy_acp_server.par` / `.exe`; the `agy` CLI has no ACP
//!   flag at all (their tracking issue is still open). Its cold start is
//!   15–25s because the binary unpacks itself first.
//! * **Cursor's `webviewTag`-era extension methods are `cursor/*`**, not
//!   `_`-prefixed, which is a deliberate deviation from the spec's
//!   extension-namespace rule. We surface them but never dispatch on them by
//!   name, only log-and-continue.

use super::driver::{AcpDriver, AuthShape, DriverConfig, PermissionMode};

/// Grok Build — `grok agent stdio`.
#[derive(Debug, Default, Clone, Copy)]
pub struct GrokDriver;

impl AcpDriver for GrokDriver {
    fn kind(&self) -> &'static str {
        "grok"
    }
    fn display_name(&self) -> &'static str {
        "Grok Build"
    }
    fn base_args(&self) -> &'static [&'static str] {
        &["agent", "stdio"]
    }
    fn binary_names(&self) -> &'static [&'static str] {
        &["grok"]
    }
    /// Grok's browser OAuth has no headless path, so a key is the only way
    /// to run without a person clicking through.
    fn auth_shape(&self) -> AuthShape {
        AuthShape::Either
    }
    fn api_key_env_vars(&self) -> &'static [&'static str] {
        &["XAI_API_KEY"]
    }
    fn known_auth_method_id(&self) -> Option<&'static str> {
        Some("grok.com")
    }
    /// Grok spells it `--permission-mode` with hyphenated values, which is
    /// not ACP's own `accept_edits` spelling.
    fn permission_args(&self, mode: PermissionMode) -> Vec<String> {
        let value = match mode {
            PermissionMode::Ask => "default",
            PermissionMode::AcceptEdits => "acceptEdits",
            PermissionMode::Auto => "auto",
        };
        vec!["--permission-mode".into(), value.into()]
    }
    fn home_env_var(&self) -> Option<&'static str> {
        Some("GROK_HOME")
    }
}

/// Cursor — `agent acp` (the CLI installs as `agent`).
#[derive(Debug, Default, Clone, Copy)]
pub struct CursorDriver;

impl AcpDriver for CursorDriver {
    fn kind(&self) -> &'static str {
        "cursor"
    }
    fn display_name(&self) -> &'static str {
        "Cursor"
    }
    fn base_args(&self) -> &'static [&'static str] {
        &["acp"]
    }
    /// The CLI installs as `agent` (default `~/.local/bin/agent`); the
    /// older `cursor-agent` name is what third-party adapters look for, so it
    /// is listed second rather than dropped.
    fn binary_names(&self) -> &'static [&'static str] {
        &["agent", "cursor-agent"]
    }
    /// `agent login` pre-auths, but over ACP the method is `cursor_login`.
    fn auth_shape(&self) -> AuthShape {
        AuthShape::Either
    }
    fn api_key_env_vars(&self) -> &'static [&'static str] {
        &["CURSOR_API_KEY", "CURSOR_AUTH_TOKEN"]
    }
    fn known_auth_method_id(&self) -> Option<&'static str> {
        Some("cursor_login")
    }
    /// Cursor takes no permission flag — the session mode carries it, which
    /// is the ACP-native path.
    fn permission_args(&self, _mode: PermissionMode) -> Vec<String> {
        Vec::new()
    }
    fn home_env_var(&self) -> Option<&'static str> {
        Some("CURSOR_HOME")
    }
}

/// Google's Antigravity — the standalone ACP server binary, not `agy`.
#[derive(Debug, Default, Clone, Copy)]
pub struct AntigravityDriver;

impl AcpDriver for AntigravityDriver {
    fn kind(&self) -> &'static str {
        "antigravity"
    }
    fn display_name(&self) -> &'static str {
        "Antigravity"
    }
    fn base_args(&self) -> &'static [&'static str] {
        &[]
    }
    /// Platform-specific, so detection has to try both.
    fn binary_names(&self) -> &'static [&'static str] {
        &["agy_acp_server.par", "agy_acp_server", "agy_acp_server.exe"]
    }
    fn auth_shape(&self) -> AuthShape {
        AuthShape::Either
    }
    fn api_key_env_vars(&self) -> &'static [&'static str] {
        &["GEMINI_API_KEY"]
    }
    /// Four advertised methods; OAuth-personal is the everyday one.
    fn known_auth_method_id(&self) -> Option<&'static str> {
        Some("oauth-personal")
    }
    /// Modes are `configOptions` with `category: "mode"`, so there is no
    /// launch flag to set.
    fn permission_args(&self, _mode: PermissionMode) -> Vec<String> {
        Vec::new()
    }
    fn home_env_var(&self) -> Option<&'static str> {
        Some("ANTIGRAVITY_HOME")
    }
    /// Isolated profile by default: everything the binary reads or writes
    /// (tokens, settings) lands under our dir, never the ambient Google
    /// config. First start needs a fresh sign-in — a one-time cost for not
    /// sharing credentials with whatever else is on the machine.
    fn default_home(&self) -> Option<std::path::PathBuf> {
        crate::which::home_dir().map(|h| h.join(".mira/antigravity-acp"))
    }
    /// Antigravity bills to ambient Google credentials. Inheriting them
    /// would silently scope the agent to whoever happens to be logged in,
    /// so they are stripped at every spawn, on every transport.
    fn env_deny_prefixes(&self) -> &'static [&'static str] {
        &["GOOGLE_"]
    }
    /// The binary unpacks itself on cold start (15–25s), longer than the
    /// default probe budget. Probing with the default reports a healthy
    /// install as missing.
    fn probe_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(60)
    }
    /// The agent runs terminals in its own process and does not want ours;
    /// advertising `terminal` would invite requests it never makes.
    fn client_caps(&self) -> crate::session::ClientCaps {
        crate::session::ClientCaps {
            fs_read: true,
            fs_write: true,
            terminal: false,
        }
    }
}

/// Every driver Mira ships, in the order they should be offered.
pub fn all() -> Vec<Box<dyn AcpDriver>> {
    vec![
        Box::new(ClaudeCodeDriver),
        Box::new(OpenCodeDriver),
        Box::new(CodexDriver),
        Box::new(GrokDriver),
        Box::new(CursorDriver),
        Box::new(AntigravityDriver),
    ]
}

pub fn by_kind(kind: &str) -> Option<Box<dyn AcpDriver>> {
    all().into_iter().find(|d| d.kind() == kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_extras_pass_effort_and_setting_sources() {
        let d = ClaudeCodeDriver;
        assert!(d.cli_extras(&DriverConfig::default()).is_empty());
        let cfg = DriverConfig {
            effort: Some("high".into()),
            setting_sources: Some("project,local".into()),
            ..Default::default()
        };
        let ex = d.cli_extras(&cfg);
        assert!(ex.windows(2).any(|w| w == ["--effort", "high"]));
        assert!(ex
            .windows(2)
            .any(|w| w == ["--setting-sources", "project,local"]));
    }

    use super::super::driver::{DriverConfig, LaunchConfig};
    use std::path::PathBuf;

    fn resolve(d: &dyn AcpDriver, mode: PermissionMode) -> LaunchConfig {
        d.resolve(&DriverConfig::default(), mode, PathBuf::from("bin"))
    }

    #[test]
    fn grok_uses_hyphenated_permission_values_not_acp_spellings() {
        // ACP spells it `accept_edits`; Grok wants `acceptEdits`. Getting this
        // wrong is a silent no-op, so it is pinned by test.
        let args = resolve(&GrokDriver, PermissionMode::AcceptEdits).args;
        assert_eq!(
            args,
            vec!["agent", "stdio", "--permission-mode", "acceptEdits"]
        );

        let auto = resolve(&GrokDriver, PermissionMode::Auto).args;
        assert!(auto.contains(&"auto".to_string()));

        let ask = resolve(&GrokDriver, PermissionMode::Ask).args;
        assert!(ask.contains(&"default".to_string()));
    }

    #[test]
    fn cursor_takes_no_permission_flag_because_modes_do_it_over_acp() {
        let args = resolve(&CursorDriver, PermissionMode::Auto).args;
        assert_eq!(args, vec!["acp"]);
    }

    #[test]
    fn antigravity_is_the_standalone_server_not_the_agy_cli() {
        let d = AntigravityDriver;
        assert!(d.base_args().is_empty());
        assert!(d.binary_names().contains(&"agy_acp_server.par"));
        assert!(
            !d.binary_names().contains(&"agy"),
            "the agy CLI has no ACP support; the server binary is separate"
        );
    }

    #[test]
    fn every_driver_exposes_a_binary() {
        // Deliberately no assertion about `base_args`. The old version of this
        // test required every agent to be a subcommand of its CLI, which is
        // exactly the assumption that was wrong for Antigravity — and wrong
        // again for the two adapter-based agents, whose binary *is* the ACP
        // server. Only the binary is genuinely required.
        for d in all() {
            assert!(
                !d.binary_names().is_empty(),
                "{} has no binary name",
                d.kind()
            );
            assert!(
                !d.display_name().is_empty(),
                "{} has no display name",
                d.kind()
            );
        }
    }

    #[test]
    fn by_kind_round_trips_and_rejects_unknown() {
        assert_eq!(
            by_kind("grok").map(|d| d.kind().to_string()),
            Some("grok".into())
        );
        assert!(by_kind("not-an-agent").is_none());
    }

    #[test]
    fn user_launch_args_never_precede_the_subcommand() {
        // If a user's extra args landed before `agent stdio`, Grok would treat
        // them as unknown subcommands and refuse to start.
        let cfg = DriverConfig {
            launch_args: vec!["--model".into(), "grok-4".into()],
            ..Default::default()
        };
        let out = GrokDriver.resolve(&cfg, PermissionMode::Ask, PathBuf::from("grok"));
        assert_eq!(out.args[0], "agent");
        assert_eq!(out.args[1], "stdio");
        assert_eq!(out.args.last().unwrap(), "grok-4");
    }

    #[test]
    fn user_env_is_passed_through_verbatim() {
        let mut cfg = DriverConfig::default();
        cfg.env.insert("CURSOR_AUTH_TOKEN".into(), "tok".into());
        let out = CursorDriver.resolve(&cfg, PermissionMode::Ask, PathBuf::from("cursor-agent"));
        assert_eq!(
            out.env.get("CURSOR_AUTH_TOKEN").map(String::as_str),
            Some("tok")
        );
        // A token-shaped name must be treated as a secret so logs redact it.
        assert!(out.secret_env.contains(&"CURSOR_AUTH_TOKEN".to_string()));
    }

    #[test]
    fn no_secret_leaks_into_the_redacted_rendering() {
        let mut cfg = DriverConfig::default();
        cfg.env.insert("SOME_TOKEN".into(), "supersecret".into());
        let out = GrokDriver.resolve(&cfg, PermissionMode::Ask, PathBuf::from("grok"));
        let rendered = out.redacted();
        assert!(!rendered.contains("supersecret"));
    }

    #[test]
    fn a_stable_ordering_so_settings_lists_dont_jitter() {
        let kinds: Vec<&str> = all().iter().map(|d| d.kind()).collect();
        assert_eq!(
            kinds,
            vec![
                "claude-code",
                "opencode",
                "codex",
                "grok",
                "cursor",
                "antigravity"
            ]
        );
    }
}

// ---------------------------------------------------------------------
// Claude Code — via the official ACP adapter.
//
// Claude Code has no native ACP server. Anthropic exposes a stable SDK, and
// the ACP adapter is built over it by the agentclientprotocol project.
//
//     npm install -g @agentclientprotocol/claude-agent-acp
//
// The binary was **renamed**: the package moved from
// `@zed-industries/claude-code-acp` to
// `@agentclientprotocol/claude-agent-acp`, and the executable with it, from
// `claude-code-acp` to `claude-agent-acp`. The old name is still listed
// second so an install that predates the rename keeps working — npm marks
// the old package deprecated rather than removing it.
//
// Either way the adapter is a Node program that takes no subcommand: it
// speaks ACP on stdio as soon as it is spawned. Verified against
// `@agentclientprotocol/claude-agent-acp` 0.81.2, which advertises
// `authMethods: []` (it authenticates through the `claude` CLI) and reports
// its version in `agentInfo`.
// ---------------------------------------------------------------------

/// Zed's adapter for Claude Code.
pub struct ClaudeCodeDriver;

impl AcpDriver for ClaudeCodeDriver {
    fn kind(&self) -> &'static str {
        "claude-code"
    }

    fn display_name(&self) -> &'static str {
        "Claude Code"
    }

    /// No subcommand: the adapter *is* the ACP server.
    fn base_args(&self) -> &'static [&'static str] {
        &[]
    }

    /// Current name first, deprecated name second so a pre-rename install
    /// still resolves. `looks_installed` takes the first match.
    fn binary_names(&self) -> &'static [&'static str] {
        &["claude-agent-acp", "claude-code-acp"]
    }

    /// `claude` is the agent; the adapter is a separate install. Both are
    /// needed, and telling the user which one is missing is the whole point.
    fn underlying_cli_names(&self) -> &'static [&'static str] {
        &["claude"]
    }

    fn install_hint(&self) -> &'static str {
        "npm install -g @agentclientprotocol/claude-agent-acp"
    }

    /// `claude auth status` prints JSON describing the session.
    fn cli_auth_args(&self) -> &'static [&'static str] {
        &["auth", "status"]
    }

    /// First-class `--effort` and `--setting-sources`, both verified against
    /// `claude --help`. Users could already pass these via verbatim launch
    /// args; the fields exist so the picker can offer them without quoting.
    fn cli_extras(&self, cfg: &DriverConfig) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(e) = cfg.effort.as_ref().filter(|e| !e.is_empty()) {
            out.push("--effort".to_string());
            out.push(e.clone());
        }
        if let Some(ss) = cfg.setting_sources.as_ref().filter(|e| !e.is_empty()) {
            out.push("--setting-sources".to_string());
            out.push(ss.clone());
        }
        out
    }

    /// The CLI is a full agent server: streaming, sessions, permissions,
    /// resume. Preferring it removes the adapter install entirely.
    fn native_flavor(&self) -> Option<crate::native::NativeFlavor> {
        Some(crate::native::NativeFlavor::Claude)
    }

    fn auth_shape(&self) -> AuthShape {
        // The `claude` CLI holds the subscription login; the API key is the
        // pay-per-token path. Either way the key alone is enough to start, so
        // no ACP `authenticate` call is required of us.
        AuthShape::EnvKey
    }

    fn api_key_env_vars(&self) -> &'static [&'static str] {
        &["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"]
    }

    /// The adapter treats `authenticate` as a no-op — credentials come from
    /// Claude Code's own login — so there is no method id to satisfy here.
    fn known_auth_method_id(&self) -> Option<&'static str> {
        None
    }

    /// Claude Code expresses permission as a *mode*, not a `--permission-mode`
    /// flag. Mapping the user's choice onto the adapter's mode ids is the
    /// adapter's job, so no permission flag is passed at launch.
    fn permission_args(&self, _mode: PermissionMode) -> Vec<String> {
        Vec::new()
    }

    fn home_env_var(&self) -> Option<&'static str> {
        Some("CLAUDE_CONFIG_DIR")
    }
}

// ---------------------------------------------------------------------
// Codex CLI — via the ACP adapter.
//
//     npm install -g @agentclientprotocol/codex-acp
//
// This package also moved — `@zed-industries/codex-acp` is deprecated in
// favour of `@agentclientprotocol/codex-acp` — but unlike the Claude adapter
// the **executable name did not change**: both ship a `codex-acp` bin. So
// only the install guidance here was ever at risk of going stale, not the
// binary lookup.
//
// The adapter bundles a compatible Codex and is a plain stdio server with no
// subcommand.
// ---------------------------------------------------------------------

/// The ACP adapter for OpenAI's Codex CLI.
pub struct CodexDriver;

impl AcpDriver for CodexDriver {
    fn kind(&self) -> &'static str {
        "codex"
    }

    fn display_name(&self) -> &'static str {
        "Codex"
    }

    fn base_args(&self) -> &'static [&'static str] {
        &[]
    }

    fn binary_names(&self) -> &'static [&'static str] {
        &["codex-acp"]
    }

    /// The adapter bundles a compatible Codex, so there is no separate CLI
    /// to install — but the package name moved, and the old one is
    /// deprecated.
    fn install_hint(&self) -> &'static str {
        "npm install -g @agentclientprotocol/codex-acp"
    }

    /// `codex` is the agent; the adapter is a separate install. The CLI
    /// is required for the native path and detected independently of it.
    fn underlying_cli_names(&self) -> &'static [&'static str] {
        &["codex"]
    }

    /// `codex app-server` holds multi-turn threads in one process, with an
    /// advertised model catalog, usage snapshots and compaction. It replaces
    /// the earlier `codex exec` experiment, which could do none of those and
    /// was unverified besides.
    fn native_flavor(&self) -> Option<crate::native::NativeFlavor> {
        Some(crate::native::NativeFlavor::CodexAppServer)
    }

    /// Native when the CLI is present, the adapter otherwise. The app-server
    /// path is UNVERIFIED against real Codex (it is not installed where this
    /// was written), and that is stated on the transport, the driver and the
    /// status — not hidden in a comment.
    fn transport(&self) -> crate::driver::Transport {
        crate::driver::Transport::Auto
    }

    fn auth_shape(&self) -> AuthShape {
        // ChatGPT subscription login, a Codex key, or an OpenAI key. A key
        // in the environment is sufficient, so no `authenticate` call is
        // required of us.
        AuthShape::EnvKey
    }

    fn api_key_env_vars(&self) -> &'static [&'static str] {
        &["CODEX_API_KEY", "OPENAI_API_KEY"]
    }

    fn known_auth_method_id(&self) -> Option<&'static str> {
        None
    }

    /// Codex's permission level is an ACP *session mode*
    /// (`read-only` / `agent` / `agent-full-access`), selected after the
    /// session exists — not a launch flag. Deliberately passing nothing here
    /// rather than guessing a flag: `agent-full-access` disables Codex's own
    /// sandbox, and a launch-time flag would grant that before the user has
    /// seen it.
    fn permission_args(&self, _mode: PermissionMode) -> Vec<String> {
        Vec::new()
    }

    fn home_env_var(&self) -> Option<&'static str> {
        Some("CODEX_HOME")
    }

    /// `agent-full-access` turns off Codex's own sandbox for the rest of the
    /// session. That is strictly more than Mira's own approval pipeline
    /// would grant, so it is gated behind an explicit acknowledgement.
    fn privileged_mode_ids(&self) -> &'static [&'static str] {
        &["agent-full-access"]
    }

    fn privileged_mode_reason(&self, mode_id: &str) -> Option<&'static str> {
        match mode_id {
            "agent-full-access" => Some(
                "Codex will run shell commands and edit files without its own \
                 sandbox. This applies to every turn until you change it back.",
            ),
            _ => None,
        }
    }

    /// Codex runs its own terminals and streams the bytes, so it must not be
    /// told the client offers them.
    fn client_caps(&self) -> crate::session::ClientCaps {
        crate::session::ClientCaps {
            fs_read: true,
            fs_write: true,
            terminal: false,
        }
    }
}

#[cfg(test)]
mod new_driver_tests {
    use super::super::driver::{AcpDriver, DriverConfig, LaunchConfig, PermissionMode};
    use super::*;
    use std::path::PathBuf;

    fn resolve(d: &dyn AcpDriver, mode: PermissionMode) -> LaunchConfig {
        d.resolve(&DriverConfig::default(), mode, PathBuf::from("bin"))
    }

    #[test]
    fn claude_code_launches_the_adapter_with_no_subcommand() {
        let cfg = resolve(&ClaudeCodeDriver, PermissionMode::Ask);
        // The adapter is the ACP server: it starts speaking the moment it is
        // spawned, so a subcommand here would be passed to it as an argument
        // it does not understand.
        assert_eq!(cfg.program, PathBuf::from("bin"));
        assert!(
            cfg.args.is_empty(),
            "claude-code-acp takes no subcommand, got {:?}",
            cfg.args
        );
    }

    #[test]
    fn claude_code_never_passes_a_permission_flag() {
        // Claude Code expresses permission as a session *mode*; passing
        // --permission-mode would be silently ignored.
        for mode in [
            PermissionMode::Ask,
            PermissionMode::AcceptEdits,
            PermissionMode::Auto,
        ] {
            assert!(
                ClaudeCodeDriver.permission_args(mode).is_empty(),
                "{mode:?} should not produce a launch flag"
            );
        }
    }

    #[test]
    fn claude_code_reads_its_key_and_config_dir() {
        let vars = ClaudeCodeDriver.api_key_env_vars();
        assert!(vars.contains(&"ANTHROPIC_API_KEY"), "got {vars:?}");
        assert_eq!(ClaudeCodeDriver.home_env_var(), Some("CLAUDE_CONFIG_DIR"));

        let cfg = DriverConfig {
            api_key: Some("sk-ant-test".into()),
            ..Default::default()
        };
        let out = ClaudeCodeDriver.resolve(&cfg, PermissionMode::Ask, PathBuf::from("bin"));
        // The key lands in the launch env and is marked secret, so it can
        // never reach a log line.
        assert_eq!(
            out.env.get("ANTHROPIC_API_KEY").map(String::as_str),
            Some("sk-ant-test")
        );
        assert!(out.secret_env.contains(&"ANTHROPIC_API_KEY".to_string()));
        assert!(!out.redacted().contains("sk-ant-test"), "the key leaked");
    }

    #[test]
    fn codex_launches_the_adapter_with_no_subcommand() {
        let cfg = resolve(&CodexDriver, PermissionMode::Ask);
        assert!(cfg.args.is_empty(), "got {:?}", cfg.args);
        assert_eq!(CodexDriver.binary_names(), &["codex-acp"]);
    }

    #[test]
    fn codex_never_passes_a_permission_flag_at_launch() {
        // `agent-full-access` turns off Codex's own sandbox. Passing it at
        // launch would grant that before the user has chosen a mode.
        for mode in [
            PermissionMode::Ask,
            PermissionMode::AcceptEdits,
            PermissionMode::Auto,
        ] {
            assert!(
                CodexDriver.permission_args(mode).is_empty(),
                "{mode:?} must not become a launch flag"
            );
        }
    }

    #[test]
    fn codex_prefers_its_own_key_over_the_openai_one() {
        let cfg = DriverConfig {
            api_key: Some("sk-test".into()),
            ..Default::default()
        };
        let out = CodexDriver.resolve(&cfg, PermissionMode::Ask, PathBuf::from("bin"));
        // Only the *first* listed var is populated: Codex prefers
        // `CODEX_API_KEY` and treats `OPENAI_API_KEY` as the fallback, so
        // writing both would be redundant and could contradict the user.
        assert_eq!(
            out.env.get("CODEX_API_KEY").map(String::as_str),
            Some("sk-test")
        );
        assert!(!out.env.contains_key("OPENAI_API_KEY"));
        assert!(out.secret_env.contains(&"CODEX_API_KEY".to_string()));
    }

    #[test]
    fn codex_is_not_told_the_client_offers_terminals() {
        // Codex runs terminals in its own process and streams the bytes, so
        // advertising ours would invite requests it does not want.
        assert!(
            !CodexDriver.client_caps().terminal,
            "Codex must not be offered client-side terminals"
        );
    }

    #[test]
    fn the_agents_that_do_ask_for_terminals_are_still_offered_them() {
        for d in [&GrokDriver as &dyn AcpDriver, &CursorDriver] {
            assert!(
                d.client_caps().terminal,
                "{} asks the client for terminals and must be offered them",
                d.kind()
            );
        }
        // Claude Code routes through the adapter, which does request them.
        assert!(ClaudeCodeDriver.client_caps().terminal);
    }

    #[test]
    fn antigravity_is_not_offered_client_terminals() {
        // It runs terminals in its own process, like Codex: advertising ours
        // would invite requests it never makes.
        assert!(
            !AntigravityDriver.client_caps().terminal,
            "Antigravity must not be offered client-side terminals"
        );
    }

    #[test]
    fn antigravity_is_isolated_by_default() {
        // No home set: the profile dir keeps credentials out of the ambient
        // Google config. Explicit homes still win.
        let out = AntigravityDriver.resolve(
            &DriverConfig::default(),
            PermissionMode::Ask,
            std::path::PathBuf::from("agy_acp_server.par"),
        );
        let home = out.env.get("ANTIGRAVITY_HOME").cloned().unwrap_or_default();
        assert!(
            home.ends_with(".mira/antigravity-acp"),
            "profile must be isolated, got {home}"
        );
        assert!(
            out.env_deny.iter().any(|p| p == "GOOGLE_"),
            "ambient Google credentials must be denied"
        );
    }

    #[test]
    fn every_agent_is_registered_and_distinct() {
        let all = super::all();
        let kinds: Vec<&str> = all.iter().map(|d| d.kind()).collect();
        assert_eq!(
            kinds,
            vec![
                "claude-code",
                "opencode",
                "codex",
                "grok",
                "cursor",
                "antigravity"
            ]
        );
        for kind in kinds {
            assert!(
                super::by_kind(kind).is_some(),
                "by_kind({kind}) found nothing"
            );
        }
    }

    #[test]
    fn a_user_launch_arg_is_passed_through_verbatim() {
        // The escape hatch for anyone on a nonstandard install path.
        let cfg = DriverConfig {
            launch_args: vec!["--experimental".into()],
            ..Default::default()
        };
        let out = ClaudeCodeDriver.resolve(&cfg, PermissionMode::Ask, PathBuf::from("bin"));
        assert_eq!(out.args, vec!["--experimental".to_string()]);
    }
}

// ---------------------------------------------------------------------
// OpenCode — one of the few agents that speaks ACP natively.
//
// No adapter: `opencode acp` starts a private ACP server for the child
// process and speaks the protocol directly. It is the one agent here that
// also reports its own `agentInfo` (name + version) at `initialize`.
// ---------------------------------------------------------------------

/// OpenCode, native ACP.
pub struct OpenCodeDriver;

impl AcpDriver for OpenCodeDriver {
    fn kind(&self) -> &'static str {
        "opencode"
    }

    fn display_name(&self) -> &'static str {
        "OpenCode"
    }

    /// `acp` is a real subcommand here, unlike the adapter-based agents.
    fn base_args(&self) -> &'static [&'static str] {
        &["acp"]
    }

    fn binary_names(&self) -> &'static [&'static str] {
        &["opencode"]
    }

    /// OpenCode speaks ACP natively — the same binary is both.
    fn underlying_cli_names(&self) -> &'static [&'static str] {
        &["opencode"]
    }

    fn auth_shape(&self) -> AuthShape {
        // Sign-in happens in the agent's own process (`opencode auth login`)
        // and is surfaced as an ACP auth method, so no key is supplied here.
        AuthShape::InteractiveBrowser
    }

    /// OpenCode negotiates its own providers, so there is no key for *us* to
    /// hand it. Reporting one would be inventing a convention.
    fn api_key_env_vars(&self) -> &'static [&'static str] {
        &[]
    }

    /// No launch-time permission flag: OpenCode picks up its configuration
    /// from its own config file and the session's `cwd`.
    fn permission_args(&self, _mode: PermissionMode) -> Vec<String> {
        Vec::new()
    }
}

#[cfg(test)]
mod opencode_tests {
    use super::super::driver::{AcpDriver, DriverConfig, LaunchConfig, PermissionMode};
    use super::*;
    use std::path::PathBuf;

    fn resolve(d: &dyn AcpDriver) -> LaunchConfig {
        d.resolve(
            &DriverConfig::default(),
            PermissionMode::Ask,
            PathBuf::from("bin"),
        )
    }

    #[test]
    fn opencode_launches_the_acp_subcommand() {
        // Unlike the adapter-based agents, `acp` is a genuine subcommand of
        // the `opencode` binary.
        let cfg = resolve(&OpenCodeDriver);
        assert_eq!(cfg.program, PathBuf::from("bin"));
        assert_eq!(cfg.args, vec!["acp".to_string()]);
    }

    #[test]
    fn opencode_gets_no_permission_flag() {
        for mode in [
            PermissionMode::Ask,
            PermissionMode::AcceptEdits,
            PermissionMode::Auto,
        ] {
            assert!(OpenCodeDriver.permission_args(mode).is_empty());
        }
    }

    #[test]
    fn opencode_is_told_we_offer_the_client_side_capabilities_it_uses() {
        let caps = OpenCodeDriver.client_caps();
        assert!(caps.fs_read && caps.fs_write && caps.terminal);
    }

    #[test]
    fn opencode_has_no_key_for_us_to_supply() {
        // It negotiates its own providers; inventing an env var would be a
        // fiction that silently does nothing.
        assert!(
            OpenCodeDriver.api_key_env_vars().is_empty(),
            "opencode should not claim an API key env var"
        );
        assert_eq!(OpenCodeDriver.auth_shape(), AuthShape::InteractiveBrowser);
    }

    #[test]
    fn an_instance_can_override_its_display_name() {
        // Two instances of one agent with different configs need distinct
        // labels in the settings list.
        let cfg = DriverConfig {
            display_name: Some("Claude (work)".into()),
            ..Default::default()
        };
        assert_eq!(cfg.display_name.as_deref(), Some("Claude (work)"));
    }

    #[test]
    fn auto_compact_defaults_to_the_agents_own_threshold() {
        // `None` means "don't send one", so the agent's default stands.
        let cfg = resolve(&OpenCodeDriver);
        assert!(cfg.args.iter().all(|a| !a.contains("compact")));
        let explicit = DriverConfig {
            auto_compact_after: Some(300_000),
            ..Default::default()
        };
        assert_eq!(explicit.auto_compact_after, Some(300_000));
    }

    #[test]
    fn a_disabled_instance_is_still_listed_but_marked_off() {
        // The settings list shows every agent, so `enabled` is a separate
        // fact from whether the binary exists.
        let cfg = DriverConfig {
            enabled: false,
            ..Default::default()
        };
        assert!(!cfg.enabled);
    }
}

#[cfg(test)]
mod privileged_mode_tests {
    use super::super::driver::AcpDriver;
    use super::*;

    #[test]
    fn codex_flags_the_mode_that_disables_its_own_sandbox() {
        assert!(CodexDriver
            .privileged_mode_ids()
            .contains(&"agent-full-access"));
        assert!(
            CodexDriver
                .privileged_mode_reason("agent-full-access")
                .is_some(),
            "a privileged mode must be able to explain itself"
        );
        // The narrow modes are not privileged: they are within what Mira's
        // own approval pipeline already gates.
        for mode in ["read-only", "agent"] {
            assert!(
                !CodexDriver.privileged_mode_ids().contains(&mode),
                "{mode} should not need confirmation"
            );
        }
    }

    #[test]
    fn no_driver_flags_a_mode_without_a_reason() {
        // A listed mode with no explanation still gets a confirmation, but
        // the prompt would be empty, so treat the pair as a unit.
        for d in super::all() {
            for mode in d.privileged_mode_ids() {
                assert!(
                    d.privileged_mode_reason(mode).is_some(),
                    "{} flags {mode} with no reason given",
                    d.kind()
                );
            }
        }
    }

    #[test]
    fn drivers_without_privileged_modes_declare_none() {
        for d in [
            &ClaudeCodeDriver as &dyn AcpDriver,
            &OpenCodeDriver,
            &GrokDriver,
            &CursorDriver,
            &AntigravityDriver,
        ] {
            assert!(
                d.privileged_mode_ids().is_empty(),
                "{} unexpectedly flags {:?}",
                d.kind(),
                d.privileged_mode_ids()
            );
        }
    }
}

#[cfg(test)]
mod binary_name_tests {
    use super::super::driver::AcpDriver;
    use super::*;

    #[test]
    fn the_current_claude_binary_is_preferred_over_the_renamed_one() {
        // The package (and executable) moved from `claude-code-acp` to
        // `claude-agent-acp`. Both are listed so a pre-rename install keeps
        // working, but a fresh one must win — otherwise a user who has both
        // gets the deprecated copy.
        let names = ClaudeCodeDriver.binary_names();
        assert_eq!(
            names[0], "claude-agent-acp",
            "the current executable must be tried first"
        );
        assert!(
            names.contains(&"claude-code-acp"),
            "the pre-rename name should still resolve for existing installs"
        );
    }

    #[test]
    fn the_codex_binary_name_survived_its_package_move() {
        // Unlike Claude, Codex's bin is `codex-acp` in both the old and new
        // packages. Pinned so a future "fix" does not rename it to match the
        // package by mistake.
        assert_eq!(CodexDriver.binary_names(), &["codex-acp"]);
    }

    #[test]
    fn no_driver_lists_a_duplicate_binary() {
        for d in super::all() {
            let mut seen = std::collections::BTreeSet::new();
            for n in d.binary_names() {
                assert!(seen.insert(*n), "{} lists {n} twice", d.kind());
            }
        }
    }
}

#[cfg(test)]
mod cursor_binary_tests {
    use super::super::driver::AcpDriver;
    use super::*;

    #[test]
    fn cursor_prefers_the_current_cli_name() {
        // The Cursor CLI installs as `agent`; `cursor-agent` is the older
        // name. Getting this backwards means the panel reports "Not found"
        // for a correctly-installed Cursor.
        let names = CursorDriver.binary_names();
        assert_eq!(names[0], "agent", "the current executable must be first");
        assert!(
            names.contains(&"cursor-agent"),
            "the pre-rename name should still resolve for older installs"
        );
        assert_eq!(CursorDriver.base_args(), &["acp"]);
        // Verified against Cursor's ACP docs, which name `cursor_login`.
        assert_eq!(CursorDriver.known_auth_method_id(), Some("cursor_login"));
    }
}
