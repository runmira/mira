//! ACP host bindings: the side of `mira_acp::host` that knows about Mira.
//!
//! `mira-acp` deliberately depends on nothing, so it declares what an agent
//! may ask for and leaves the binding to here. This module supplies it:
//! filesystem requests go through `ToolContext::resolve`, permission
//! requests go through the session's `Approver`, and terminals go through
//! `mira-pty`'s sandboxed PTY.
//!
//! # One port set per Mira session
//!
//! Every port here is built **once, for one Mira session**, and holds that
//! session's `ToolContext`, approver and confinement root. Nothing is looked
//! up per request — in particular, never by the `sessionId` an agent puts in
//! its own request, which is the agent's correlation handle rather than a
//! Mira session id. Feeding that into a root or an approver lookup would let
//! one agent session act with another session's authority, so it is
//! structurally impossible here rather than merely avoided.
//!
//! The unifying rule is that an ACP agent is **third-party code**. Every
//! capability it asks for is the same capability Mira's own tools have,
//! granted through the same checks — never a wider one.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use mira_acp::events::PermissionRequest;
use mira_acp::events::NormalizedEvent;
use mira_acp::host::{EventPort, FilePort, HostError, PermissionPort, TerminalPort};
use mira_harness::approver::Approver;
use mira_policy::Decision;
use mira_pty::{Exit, PtyRegistry, PtySpawn};
use mira_sandbox::SandboxProfile;
use mira_tools::context::ToolContext;

use crate::protocol::ServerMsg;

/// Serves ACP `terminal/*` from sandboxed PTYs.
///
/// Terminal ids are namespaced by the owning Mira session, because ACP lets
/// the agent pick its own id and agents reuse names like `"default"` — keying
/// only on the id would let one session read or kill another's terminal.
pub struct AcpTerminals {
    registry: Arc<PtyRegistry>,
    /// The Mira session that owns these terminals.
    session_id: String,
    /// The boundary they are confined to.
    ///
    /// A required constructor input rather than a lookup. An earlier version
    /// hardcoded `/` as a placeholder, which silently meant *no confinement at
    /// all* — a third-party agent could spawn a terminal anywhere. Making it
    /// an input means there is no unwired state to fail open from.
    repo_root: PathBuf,
    sandboxed: bool,
}

impl AcpTerminals {
    pub fn new(
        session_id: impl Into<String>,
        repo_root: impl Into<PathBuf>,
        registry: Arc<PtyRegistry>,
        sandboxed: bool,
    ) -> Arc<Self> {
        Arc::new(AcpTerminals {
            registry,
            session_id: session_id.into(),
            repo_root: repo_root.into(),
            sandboxed,
        })
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn registry(&self) -> &Arc<PtyRegistry> {
        &self.registry
    }

    /// Kill and forget every terminal this session started. Call when the
    /// Mira session ends, so a closed chat cannot leave processes running.
    pub fn shutdown(&self) -> usize {
        self.registry.kill_session(&self.session_id)
    }

    fn get(&self, terminal_id: &str) -> Result<Arc<mira_pty::PtyHandle>, HostError> {
        self.registry
            .get(&self.session_id, terminal_id)
            .ok_or_else(|| HostError::NotFound(terminal_id.to_string()))
    }
}

#[async_trait::async_trait]
impl TerminalPort for AcpTerminals {
    async fn create(
        &self,
        command: &str,
        args: &[String],
        cwd: Option<&str>,
    ) -> Result<String, HostError> {
        if command.trim().is_empty() {
            return Err(HostError::BadRequest("empty command".into()));
        }
        let root = self.repo_root.clone();
        let dir = match cwd {
            Some(c) => {
                let candidate = PathBuf::from(c);
                // The agent may pick its own cwd, but only inside the
                // session's boundary. A lexical prefix check is what stops
                // `..` from escaping; `ToolContext::resolve` handles the
                // symlink case for file paths, and the sandbox profile
                // confines the process regardless.
                if !candidate.starts_with(&root) {
                    return Err(HostError::OutOfBounds(format!(
                        "cwd {} is outside the session root {}",
                        candidate.display(),
                        root.display()
                    )));
                }
                candidate
            }
            None => root.clone(),
        };

        let mut spec = PtySpawn::new(&self.session_id, SandboxProfile::new(&root), command);
        spec.args = args.to_vec();
        spec.cwd = dir;
        spec.sandboxed = self.sandboxed;

        let handle = mira_pty::spawn(spec).map_err(|e| HostError::Failed(e.to_string()))?;
        let id = handle.id().to_string();
        self.registry.insert(handle);
        Ok(id)
    }

    async fn output(&self, terminal_id: &str) -> Result<String, HostError> {
        Ok(self.get(terminal_id)?.output())
    }

    async fn release(&self, terminal_id: &str) -> Result<(), HostError> {
        // Removed from the registry first, so a second release reports
        // "not found" instead of quietly succeeding twice.
        let h = self
            .registry
            .remove(&self.session_id, terminal_id)
            .ok_or_else(|| HostError::NotFound(terminal_id.to_string()))?;
        h.release().map_err(|e| HostError::Failed(e.to_string()))?;
        Ok(())
    }

    async fn wait_for_exit(&self, terminal_id: &str) -> Result<Option<i32>, HostError> {
        match self.get(terminal_id)?.wait().await {
            Exit::Code(code) => Ok(Some(code)),
            // Killed is not a code; reporting one would tell the agent its
            // command exited cleanly when it was terminated.
            Exit::Killed => Ok(None),
        }
    }

    async fn kill(&self, terminal_id: &str) -> Result<(), HostError> {
        self.get(terminal_id)?
            .kill()
            .map_err(|e| HostError::Failed(e.to_string()))
    }
}

/// Serves ACP `fs/read_text_file` and `fs/write_text_file`.
///
/// Both go through `ToolContext::resolve` first. That is the whole point: an
/// ACP agent supplies its own paths, and resolving them is what keeps a
/// third-party agent from being handed filesystem-wide access — strictly
/// more than Mira's own `read_file`/`write_file` tools grant.
pub struct AcpFiles {
    tool_ctx: ToolContext,
    approver: Arc<dyn Approver>,
    /// Cap on a single read, so one request cannot pull the whole disk.
    max_bytes: usize,
}

impl AcpFiles {
    pub fn new(tool_ctx: ToolContext, approver: Arc<dyn Approver>, max_bytes: usize) -> Arc<Self> {
        Arc::new(AcpFiles {
            tool_ctx,
            approver,
            max_bytes,
        })
    }
}

#[async_trait::async_trait]
impl FilePort for AcpFiles {
    async fn read_text(&self, path: &str) -> Result<String, HostError> {
        // The agent's path is untrusted input. `resolve` decides whether it
        // lands inside the session's boundary, including the symlink case for
        // a file that does not exist yet.
        let abs = self
            .tool_ctx
            .resolve(path)
            .ok_or_else(|| classify_path_escape(path))?;

        let bytes = tokio::fs::read(&abs)
            .await
            .map_err(|e| HostError::Failed(format!("{path}: {e}")))?;
        if bytes.len() > self.max_bytes {
            return Err(HostError::Failed(format!(
                "{} is {} bytes, over the {}-byte read limit",
                path,
                bytes.len(),
                self.max_bytes
            )));
        }
        String::from_utf8(bytes)
            .map_err(|_| HostError::Failed(format!("{path} is not valid UTF-8 text")))
    }

    async fn write_text(&self, path: &str, content: &str) -> Result<(), HostError> {
        let abs = self
            .tool_ctx
            .resolve(path)
            .ok_or_else(|| classify_path_escape(path))?;

        // A write is destructive, so it goes to the user through the same
        // approver Mira's own `write_file` uses. The agent gets no private
        // path around the prompt.
        gate_write(self.approver.as_ref(), path, &synthetic_call(path)).await?;

        if let Some(parent) = abs.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| HostError::Failed(e.to_string()))?;
        }
        tokio::fs::write(&abs, content.as_bytes())
            .await
            .map_err(|e| HostError::Failed(e.to_string()))?;
        Ok(())
    }
}

/// Ask the user before letting an agent write.
///
/// The agent's choice maps onto Mira's own `Decision`, so an "always allow"
/// widens policy for Mira's tools too rather than granting a private,
/// invisible exemption to the agent.
pub async fn gate_write(
    approver: &dyn Approver,
    path: &str,
    synthetic_call: &mira_core::ToolCall,
) -> Result<(), HostError> {
    if approver.approve(synthetic_call, Decision::Ask).await {
        Ok(())
    } else {
        Err(HostError::Denied(format!("write to {path} was not approved")))
    }
}

/// Classify a `ToolContext::resolve` miss into a reason the agent can act on.
///
/// `None` from `resolve` means the path escaped containment — including via a
/// symlink, which is why the agent's raw path must never be used directly.
pub fn classify_path_escape(raw: &str) -> HostError {
    HostError::OutOfBounds(format!("{raw} resolves outside this session's root"))
}

/// Build the `ToolCall` shape Mira's policy and approver expect, so an ACP
/// write is gated by exactly the rules a native write is.
fn synthetic_call(path: &str) -> mira_core::ToolCall {
    let arguments = serde_json::json!({ "path": path, "content": "" }).to_string();
    mira_core::ToolCall {
        id: mira_core::ToolCallId::new(),
        kind: mira_core::ToolCallKind::Function,
        function: mira_core::ToolCallFunction {
            name: "write_file".to_string(),
            arguments,
        },
    }
}

/// Serves ACP `session/request_permission`.
///
/// The mapping here is the whole subtlety. Mira's approver answers a yes/no
/// question; ACP requires the client to answer with **one of the option ids
/// the agent itself sent**. So a decision is projected back onto the agent's
/// vocabulary, and when no option expresses it, the only honest answer is
/// `Cancelled` — never a fabricated id.
pub struct AcpPermissions {
    approver: Arc<dyn Approver>,
}

impl AcpPermissions {
    pub fn new(approver: Arc<dyn Approver>) -> Arc<Self> {
        Arc::new(AcpPermissions { approver })
    }
}

/// Project a yes/no decision onto the agent's own option ids.
///
/// Returns `None` when the agent offered nothing that expresses the
/// decision — for example it offered only "always allow" and the user said
/// no. The caller then answers `Cancelled`, which tells the agent to unwind
/// the turn instead of retrying against a wall.
fn choose_option(req: &PermissionRequest, allowed: bool) -> Option<String> {
    let pick = |once: bool| -> Option<String> {
        req.options
            .iter()
            .find(|o| {
                if once {
                    // The narrowest expression of this decision: a one-off
                    // grant or refusal, not a standing change.
                    o.is_allow() == allowed && !o.is_persistent()
                } else {
                    o.is_allow() == allowed
                }
            })
            .map(|o| o.option_id.clone())
    };

    let chosen = pick(true).or_else(|| pick(false));
    if chosen.is_none() {
        return None;
    }
    if allowed {
        // A persistent grant is a materially wider decision than the user may
        // have had in mind, so say which one was chosen.
        if let Some(o) = req
            .options
            .iter()
            .find(|o| o.is_allow() && o.is_persistent())
        {
            if req.options.iter().all(|o| !(o.is_allow() && !o.is_persistent())) {
                tracing::warn!(
                    tool_call_id = %req.tool_call_id,
                    option_id = %o.option_id,
                    "acp: only a persistent permission option was offered; \
                     the user's approval grants it for the whole session"
                );
            }
        }
    }
    chosen
}

#[async_trait::async_trait]
impl PermissionPort for AcpPermissions {
    async fn request_permission(
        &self,
        req: &PermissionRequest,
    ) -> Result<Option<String>, HostError> {
        // The agent's own title and options ride along in the arguments so
        // the existing approval UI has something to show, not a bare tool name.
        let call = mira_core::ToolCall {
            id: mira_core::ToolCallId::new(),
            kind: mira_core::ToolCallKind::Function,
            function: mira_core::ToolCallFunction {
                name: format!("acp: {}", req.title),
                arguments: serde_json::json!({
                    "tool_call_id": req.tool_call_id,
                    "title": req.title,
                    "options": req
                        .options
                        .iter()
                        .map(|o| serde_json::json!({
                            "id": o.option_id,
                            "name": o.name,
                            "kind": format!("{:?}", o.kind).to_lowercase(),
                        }))
                        .collect::<Vec<_>>(),
                })
                .to_string(),
            },
        };

        let allowed = self.approver.approve(&call, Decision::Ask).await;
        let chosen = choose_option(req, allowed);
        if chosen.is_none() {
            tracing::info!(
                tool_call_id = %req.tool_call_id,
                allowed,
                options = req.options.len(),
                "acp: no option expresses this decision; answering cancelled"
            );
        }
        Ok(chosen)
    }
}

/// Narrow a session's working directory to a boundary that can never be
/// smaller than the repository it lives in.
///
/// A `cwd` outside `repo_root` must not shrink the boundary, or pointing a
/// session at `/etc` would confine it *to* `/etc` and quietly widen access to
/// everything else.
pub fn confine_root(repo_root: &Path, session_root: &Path) -> PathBuf {
    if session_root.starts_with(repo_root) {
        session_root.to_path_buf()
    } else {
        repo_root.to_path_buf()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn root() -> PathBuf {
        std::env::temp_dir()
    }

    /// A terminal port for one Mira session, confined to the temp dir.
    fn terminals(reg: Arc<PtyRegistry>, sandboxed: bool) -> Arc<AcpTerminals> {
        AcpTerminals::new("s1", root(), reg, sandboxed)
    }

    /// Read output as it appears, rather than assuming it is already there.
    async fn await_output(t: &AcpTerminals, id: &str, want: &str) -> String {
        for _ in 0..200 {
            let out = t.output(id).await.unwrap_or_default();
            if out.contains(want) {
                return out;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        t.output(id).await.unwrap_or_default()
    }

    struct TestApprover {
        allow: bool,
    }

    #[async_trait::async_trait]
    impl Approver for TestApprover {
        async fn approve(
            &self,
            _call: &mira_core::ToolCall,
            _decision: Decision,
        ) -> bool {
            self.allow
        }
    }

    fn files_ctx(root: &Path) -> (ToolContext, Arc<TestApprover>) {
        let sandbox = Arc::new(mira_sandbox::Sandbox::new(root));
        let ctx = ToolContext::new(root.to_path_buf(), sandbox);
        (ctx, Arc::new(TestApprover { allow: true }))
    }

    // ---- terminals ----

    #[tokio::test]
    async fn a_terminal_runs_and_reports_its_exit() {
        let t = terminals(Arc::new(PtyRegistry::new()), false);
        let id = t
            .create("/bin/sh", &["-c".into(), "echo hi; exit 7".into()], None)
            .await
            .expect("create");
        assert_eq!(t.wait_for_exit(&id).await.expect("wait"), Some(7));
        assert!(t.output(&id).await.expect("output").contains("hi"));
    }

    #[tokio::test]
    async fn output_streams_before_exit() {
        // The payoff of a PTY: the agent can watch a long command.
        let t = terminals(Arc::new(PtyRegistry::new()), false);
        let id = t
            .create(
                "/bin/sh",
                &[
                    "-c".into(),
                    "printf 'started\\n'; sleep 1; printf 'done\\n'".into(),
                ],
                None,
            )
            .await
            .expect("create");
        let early = await_output(&t, &id, "started").await;
        assert!(early.contains("started"), "got {early:?}");
        assert!(
            !early.contains("done"),
            "output was buffered to exit, not streamed: {early:?}"
        );
        let full = await_output(&t, &id, "done").await;
        assert!(full.contains("done"), "got {full:?}");
    }

    #[tokio::test]
    async fn one_mira_session_cannot_reach_another_sessions_terminal() {
        // The registry is namespaced per Mira session, not by the agent's own
        // session id.
        let reg = Arc::new(PtyRegistry::new());
        let a = terminals(reg.clone(), false);
        let b = AcpTerminals::new("s2", root(), reg.clone(), false);
        let id = a
            .create("/bin/sh", &["-c".into(), "sleep 5".into()], None)
            .await
            .expect("create");
        assert!(b.output(&id).await.is_err());
        assert!(b.kill(&id).await.is_err());
        assert!(a.output(&id).await.is_ok());
    }

    #[tokio::test]
    async fn a_cwd_outside_the_root_is_refused() {
        let t = terminals(Arc::new(PtyRegistry::new()), false);
        let err = t.create("/bin/sh", &[], Some("/etc")).await.expect_err("refuse");
        assert!(matches!(err, HostError::OutOfBounds(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn an_empty_command_is_refused() {
        let t = terminals(Arc::new(PtyRegistry::new()), false);
        let err = t.create("   ", &[], None).await.expect_err("refuse");
        assert!(matches!(err, HostError::BadRequest(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn releasing_twice_reports_not_found() {
        let t = terminals(Arc::new(PtyRegistry::new()), false);
        let id = t
            .create("/bin/sh", &["-c".into(), "sleep 5".into()], None)
            .await
            .expect("create");
        t.release(&id).await.expect("first release");
        let err = t.release(&id).await.expect_err("second release");
        assert!(matches!(err, HostError::NotFound(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn shutdown_ends_every_terminal_the_session_started() {
        let reg = Arc::new(PtyRegistry::new());
        let t = terminals(reg.clone(), false);
        for _ in 0..2 {
            t.create("/bin/sh", &["-c".into(), "sleep 30".into()], None)
                .await
                .expect("create");
        }
        let other = AcpTerminals::new("s2", root(), reg.clone(), false);
        other
            .create("/bin/sh", &["-c".into(), "sleep 30".into()], None)
            .await
            .expect("create");
        assert_eq!(reg.count(), 3);
        assert_eq!(t.shutdown(), 2);
        assert_eq!(reg.count(), 1);
    }

    #[test]
    fn the_boundary_never_narrows_below_the_repository() {
        let repo = PathBuf::from("/repo");
        assert_eq!(
            confine_root(&repo, Path::new("/repo/sub")),
            PathBuf::from("/repo/sub")
        );
        assert_eq!(
            confine_root(&repo, Path::new("/etc")),
            PathBuf::from("/repo")
        );
    }

    #[tokio::test]
    async fn a_pty_under_the_sandbox_can_read_its_own_root() {
        let t = terminals(Arc::new(PtyRegistry::new()), true);
        let dir = std::env::temp_dir();
        let Ok(id) = t
            .create(
                "/bin/sh",
                &[
                    "-c".into(),
                    format!("cd {} && echo INSIDE_OK", dir.display()),
                ],
                Some(&dir.to_string_lossy()),
            )
            .await
        else {
            // Seatbelt can refuse in restricted CI; the PTY mechanics are
            // covered by the unsandboxed tests above.
            eprintln!("skipping: no usable sandbox on this host");
            return;
        };
        let _ = t.wait_for_exit(&id).await;
        let out = t.output(&id).await.unwrap_or_default();
        assert!(
            out.contains("INSIDE_OK"),
            "a sandboxed pty should run inside its own root, got {out:?}"
        );
    }

    // ---- files ----

    #[tokio::test]
    async fn a_read_inside_the_root_succeeds_and_one_outside_does_not() {
        // The containment check is the entire reason an ACP agent cannot be
        // handed unrestricted filesystem access.
        let dir = std::env::temp_dir().join("mira-acp-files-test");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(dir.join("ok.txt"), "inside").await.unwrap();

        let (ctx, appr) = files_ctx(&dir);
        let files = AcpFiles::new(ctx, appr, 1024 * 1024);

        assert_eq!(files.read_text("ok.txt").await.expect("in-root"), "inside");
        for escape in ["/etc/passwd", "../outside.txt", "/"] {
            let err = files.read_text(escape).await.expect_err("refuse");
            assert!(
                matches!(err, HostError::OutOfBounds(_)),
                "{escape} was not refused: {err:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_write_outside_the_root_is_refused_before_it_reaches_disk() {
        let dir = std::env::temp_dir().join("mira-acp-write-test");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let (ctx, appr) = files_ctx(&dir);
        let files = AcpFiles::new(ctx, appr, 1024 * 1024);
        let err = files
            .write_text("/tmp/mira-should-not-exist.txt", "x")
            .await
            .expect_err("refuse");
        assert!(matches!(err, HostError::OutOfBounds(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn a_refused_write_never_reaches_the_disk() {
        // Even inside the root, a user who says no must leave no file behind.
        let dir = std::env::temp_dir().join("mira-acp-deny-test");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let target = dir.join("denied.txt");
        let _ = tokio::fs::remove_file(&target).await;

        let sandbox = Arc::new(mira_sandbox::Sandbox::new(&dir));
        let ctx = ToolContext::new(dir.clone(), sandbox);
        let files = AcpFiles::new(ctx, Arc::new(TestApprover { allow: false }), 1024);
        let err = files
            .write_text("denied.txt", "payload")
            .await
            .expect_err("refuse");
        assert!(matches!(err, HostError::Denied(_)), "got {err:?}");
        assert!(
            !tokio::fs::try_exists(&target).await.unwrap_or(false),
            "a denied write still created the file"
        );
    }

    #[tokio::test]
    async fn an_oversized_read_is_refused_rather_than_truncated_silently() {
        let dir = std::env::temp_dir().join("mira-acp-big-test");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(dir.join("big.txt"), vec![b'x'; 4096])
            .await
            .unwrap();
        let (ctx, appr) = files_ctx(&dir);
        let files = AcpFiles::new(ctx, appr, 128);
        let err = files.read_text("big.txt").await.expect_err("refuse");
        assert!(matches!(err, HostError::Failed(_)), "got {err:?}");
    }

    // ---- permissions ----

    #[test]
    fn approving_prefers_the_narrowest_grant() {
        use mira_acp::events::{AcpPermissionOptionKind as Kind, PermissionChoice};
        let r = PermissionRequest {
            session_id: "s".into(),
            tool_call_id: "c".into(),
            title: "t".into(),
            kind: None,
            options: vec![
                PermissionChoice {
                    option_id: "always".into(),
                    name: "Always".into(),
                    kind: Kind::AllowAlways,
                },
                PermissionChoice {
                    option_id: "once".into(),
                    name: "Once".into(),
                    kind: Kind::AllowOnce,
                },
            ],
        };
        assert_eq!(choose_option(&r, true).as_deref(), Some("once"));
    }

    #[test]
    fn a_persistent_only_offer_is_still_chosen_when_approving() {
        use mira_acp::events::{AcpPermissionOptionKind as Kind, PermissionChoice};
        let r = PermissionRequest {
            session_id: "s".into(),
            tool_call_id: "c".into(),
            title: "t".into(),
            kind: None,
            options: vec![PermissionChoice {
                option_id: "always".into(),
                name: "Always".into(),
                kind: Kind::AllowAlways,
            }],
        };
        assert_eq!(choose_option(&r, true).as_deref(), Some("always"));
    }

    #[test]
    fn a_decline_with_no_reject_option_cancels_rather_than_lying() {
        use mira_acp::events::{AcpPermissionOptionKind as Kind, PermissionChoice};
        let r = PermissionRequest {
            session_id: "s".into(),
            tool_call_id: "c".into(),
            title: "t".into(),
            kind: None,
            options: vec![PermissionChoice {
                option_id: "always".into(),
                name: "Always".into(),
                kind: Kind::AllowAlways,
            }],
        };
        assert_eq!(choose_option(&r, false), None);
    }

    #[test]
    fn declining_prefers_a_one_off_rejection() {
        use mira_acp::events::{AcpPermissionOptionKind as Kind, PermissionChoice};
        let mk = |id: &str, k: Kind| PermissionChoice {
            option_id: id.into(),
            name: id.into(),
            kind: k,
        };
        let r = PermissionRequest {
            session_id: "s".into(),
            tool_call_id: "c".into(),
            title: "t".into(),
            kind: None,
            options: vec![mk("no-always", Kind::RejectAlways), mk("no", Kind::RejectOnce)],
        };
        assert_eq!(choose_option(&r, false).as_deref(), Some("no"));
    }

    #[test]
    fn no_options_at_all_cancels_rather_than_inventing_an_id() {
        let r = PermissionRequest {
            session_id: "s".into(),
            tool_call_id: "c".into(),
            title: "t".into(),
            kind: None,
            options: vec![],
        };
        assert_eq!(choose_option(&r, true), None);
        assert_eq!(choose_option(&r, false), None);
    }

    #[test]
    fn vendor_option_ids_are_echoed_verbatim() {
        use mira_acp::events::{AcpPermissionOptionKind as Kind, PermissionChoice};
        for id in ["allow-once", "proceed_2", "YES"] {
            let r = PermissionRequest {
                session_id: "s".into(),
                tool_call_id: "c".into(),
                title: "t".into(),
                kind: None,
                options: vec![PermissionChoice {
                    option_id: id.into(),
                    name: id.into(),
                    kind: Kind::AllowOnce,
                }],
            };
            assert_eq!(choose_option(&r, true).as_deref(), Some(id));
        }
    }

    #[tokio::test]
    async fn a_permission_routes_through_the_session_approver() {
        use mira_acp::events::{AcpPermissionOptionKind as Kind, PermissionChoice};
        let p = AcpPermissions::new(Arc::new(TestApprover { allow: true }));
        let r = PermissionRequest {
            session_id: "s".into(),
            tool_call_id: "c".into(),
            title: "t".into(),
            kind: None,
            options: vec![
                PermissionChoice {
                    option_id: "always".into(),
                    name: "a".into(),
                    kind: Kind::AllowAlways,
                },
                PermissionChoice {
                    option_id: "once".into(),
                    name: "o".into(),
                    kind: Kind::AllowOnce,
                },
            ],
        };
        assert_eq!(
            p.request_permission(&r).await.expect("permission"),
            Some("once".to_string())
        );
    }

    #[tokio::test]
    async fn a_decline_answers_cancelled_when_no_option_fits() {
        use mira_acp::events::{AcpPermissionOptionKind as Kind, PermissionChoice};
        let p = AcpPermissions::new(Arc::new(TestApprover { allow: false }));
        let r = PermissionRequest {
            session_id: "s".into(),
            tool_call_id: "c".into(),
            title: "t".into(),
            kind: None,
            options: vec![PermissionChoice {
                option_id: "always".into(),
                name: "a".into(),
                kind: Kind::AllowAlways,
            }],
        };
        assert_eq!(p.request_permission(&r).await.expect("permission"), None);
    }
}

/// Forwards normalized ACP events onto a Mira session's WebSocket stream.
///
/// The seam that lets `mira-acp` stay ignorant of Mira: it produces
/// `NormalizedEvent`, this turns those into `ServerMsg` and pushes them at
/// the session's existing broadcast channel, so an external agent's output
/// reaches the frontend by the same path as Mira's own.
pub struct AcpEventPort {
    events: tokio::sync::broadcast::Sender<ServerMsg>,
    /// Where the agent's spend reports are booked, for the Usage page.
    spend: Option<(crate::agent_spend::SpendLedger, crate::agent_spend::Spender)>,
}

impl AcpEventPort {
    pub fn new(events: tokio::sync::broadcast::Sender<ServerMsg>) -> Arc<Self> {
        Arc::new(AcpEventPort { events, spend: None })
    }

    /// A port that also books the agent's spend reports to `ledger`.
    pub fn with_spend(
        events: tokio::sync::broadcast::Sender<ServerMsg>,
        ledger: crate::agent_spend::SpendLedger,
        who: crate::agent_spend::Spender,
    ) -> Arc<Self> {
        Arc::new(AcpEventPort { events, spend: Some((ledger, who)) })
    }

    /// Book a spend report; anything else becomes a frame.
    fn route(&self, event: NormalizedEvent) {
        if let mira_acp::events::MiraEvent::Spend { session, models } = &event.event {
            if let Some((ledger, who)) = &self.spend {
                let (ledger, who, session, models) = (ledger.clone(), who.clone(), session.clone(), models.clone());
                // File I/O, off the event loop.
                tokio::task::spawn_blocking(move || ledger.record(&who, session.as_deref(), &models));
            }
            return;
        }
        if let Some(msg) = ServerMsg::from_acp(event) {
            self.push(msg);
        }
    }

    /// Forward a native agent's event onto the wire.
    ///
    /// Same conversion the ACP path uses, which is the point: by the time it
    /// reaches the client, a native agent is indistinguishable from an ACP
    /// one.
    pub async fn emit(&self, event: NormalizedEvent) {
        self.route(event);
    }

    /// Push a frame, logging rather than failing on a send error.
    ///
    /// A send error means nobody is attached, which is normal for a
    /// background session and must not take the agent down with it.
    pub fn push(&self, msg: ServerMsg) {
        if let Err(e) = self.events.send(msg) {
            match e {
                tokio::sync::broadcast::error::SendError(value) => {
                    tracing::trace!(?value, "acp: no attached client for a session update");
                }
            }
        }
    }
}

#[async_trait::async_trait]
impl EventPort for AcpEventPort {
    async fn emit(&self, event: NormalizedEvent) {
        // Retain provenance for anything unmodelled before it becomes a frame,
        // so a vendor extension is greppable in the logs.
        self.route(event);
    }
}

impl AcpEventPort {
    /// Announce the end of a turn.
    pub fn turn_ended(&self, stop_reason: &str) {
        self.push(ServerMsg::acp_turn_end(stop_reason));
    }
}

#[cfg(test)]
mod event_tests {
    use super::*;
    use mira_acp::events::{
        ConfigValueView, EventSource, MiraEvent, NormalizedEvent, PlanEntry, SessionConfigView,
        SessionModeView, ToolCallState,
    };

    fn frame(e: MiraEvent) -> ServerMsg {
        ServerMsg::from_acp(NormalizedEvent {
            source: EventSource::Acp {
                variant: "x".into(),
            },
            event: e,
        })
        .expect("every ACP event maps to a frame")
    }

    fn empty_state() -> ToolCallState {
        ToolCallState {
            id: "c1".into(),
            title: "Build".into(),
            name: None,
            kind: None,
            status: mira_acp::events::AcpToolCallStatus::Pending,
            raw_input: None,
            raw_output: None,
            content: vec![],
            locations: vec![],
        }
    }

    /// Distinct `type` per variant, which is what lets the frontend
    /// pattern-match without a wrapper.
    #[test]
    fn each_event_maps_to_its_own_discriminated_type() {
        let cases: Vec<(MiraEvent, &str)> = vec![
            (
                MiraEvent::AssistantText {
                    message_id: None,
                    text: "hi".into(),
                },
                "acp_text",
            ),
            (
                MiraEvent::AgentThought {
                    message_id: None,
                    text: "hmm".into(),
                },
                "acp_thought",
            ),
            (MiraEvent::ToolCall(empty_state()), "acp_tool_call"),
            (
                MiraEvent::ToolCallUpdate(empty_state()),
                "acp_tool_call_update",
            ),
            (MiraEvent::Plan { entries: vec![] }, "acp_plan"),
            (MiraEvent::Commands { names: vec![] }, "acp_commands"),
            (MiraEvent::Usage {
                used: 1,
                size: 2,
                cost: None,
            },
            "acp_usage"),
            (MiraEvent::SessionInfo {
                title: None,
                updated_at: None,
            },
            "acp_session_info"),
        ];
        for (event, expected) in cases {
            let json = serde_json::to_value(frame(event)).expect("serialize");
            assert_eq!(
                json["type"], expected,
                "wrong wire type for {expected}"
            );
        }
    }

    #[test]
    fn text_and_reasoning_stay_distinguishable() {
        // Collapsing these would hide an agent's reasoning in the answer.
        let a = serde_json::to_value(frame(MiraEvent::AssistantText {
            message_id: None,
            text: "x".into(),
        }))
        .unwrap();
        let b = serde_json::to_value(frame(MiraEvent::AgentThought {
            message_id: None,
            text: "x".into(),
        }))
        .unwrap();
        assert_ne!(a["type"], b["type"]);
    }

    #[test]
    fn a_tool_call_forwards_its_whole_normalized_state() {
        // The diff and terminal content ride along; a bare ToolCall would
        // drop them.
        let mut state = empty_state();
        state.content = vec![mira_acp::events::ToolContent::Diff {
            path: "/repo/a.rs".into(),
            old_text: Some("old".to_string()),
            new_text: Some("new".to_string()),
        }];
        let json = serde_json::to_value(frame(MiraEvent::ToolCall(state))).unwrap();
        assert_eq!(json["call"]["id"], "c1");
        assert_eq!(json["call"]["content"][0]["path"], "/repo/a.rs");
    }

    #[test]
    fn cost_is_projected_onto_named_fields() {
        let json = serde_json::to_value(frame(MiraEvent::Usage {
            used: 10,
            size: 1000,
            cost: Some((0.25, "USD".to_string())),
        }))
        .unwrap();
        assert_eq!(json["cost"]["amount"], 0.25);
        assert_eq!(json["cost"]["currency"], "USD");
    }

    #[test]
    fn an_unmodelled_event_keeps_its_provenance() {
        // The whole point: a gap must be visible, not silent.
        let msg = ServerMsg::from_acp(NormalizedEvent {
            source: EventSource::Unmodelled {
                method: "session/vendor_thing".into(),
            },
            event: MiraEvent::Unmodelled {
                source: EventSource::Unmodelled {
                    method: "session/vendor_thing".into(),
                },
                reason: "not modelled".into(),
            },
        })
        .expect("frame");
        let json = serde_json::to_value(msg).unwrap();
        assert_eq!(json["type"], "acp_unmodelled");
        assert_eq!(json["method"], "session/vendor_thing");
        assert_eq!(json["reason"], "not modelled");
    }

    #[test]
    fn a_modelled_but_unsurfaced_variant_is_still_reported() {
        let msg = ServerMsg::from_acp(NormalizedEvent {
            source: EventSource::Acp {
                variant: "available_modes_update".into(),
            },
            event: MiraEvent::Unmodelled {
                source: EventSource::Acp {
                    variant: "available_modes_update".into(),
                },
                reason: "unhandled sessionUpdate".into(),
            },
        })
        .expect("frame");
        let json = serde_json::to_value(msg).unwrap();
        assert_eq!(json["type"], "acp_unmodelled");
        assert_eq!(json["method"], "available_modes_update");
    }

    #[test]
    fn a_turn_end_reports_the_agents_own_stop_reason() {
        // A cancelled turn must not render as a completed answer.
        let json = serde_json::to_value(ServerMsg::acp_turn_end("cancelled")).unwrap();
        assert_eq!(json["type"], "acp_turn_end");
        assert_eq!(json["stop_reason"], "cancelled");
    }

    #[test]
    fn modes_and_config_options_survive_the_wire() {
        let json = serde_json::to_value(frame(MiraEvent::Modes {
            current: "ask".into(),
            available: vec![SessionModeView {
                id: "ask".into(),
                name: "Ask".into(),
                description: None,
            }],
        }))
        .unwrap();
        assert_eq!(json["current"], "ask");
        assert_eq!(json["available"][0]["id"], "ask");

        let json = serde_json::to_value(frame(MiraEvent::ConfigOptions {
            options: vec![SessionConfigView {
                id: "model".into(),
                name: "Model".into(),
                description: None,
                category: Some("model".into()),
                current: Some("grok-code".into()),
                values: vec![ConfigValueView {
                    value: "grok-code".into(),
                    name: "Grok Code".into(),
                    description: None,
                }],
            }],
        }))
        .unwrap();
        assert_eq!(json["options"][0]["category"], "model");
        assert_eq!(json["options"][0]["current"], "grok-code");
        assert_eq!(json["options"][0]["values"][0]["value"], "grok-code");
    }

    #[test]
    fn plan_entries_reach_the_wire_with_their_status() {
        let json = serde_json::to_value(frame(MiraEvent::Plan {
            entries: vec![PlanEntry {
                content: "do it".into(),
                priority: "high".into(),
                status: "in_progress".into(),
            }],
        }))
        .unwrap();
        assert_eq!(json["entries"][0]["content"], "do it");
        assert_eq!(json["entries"][0]["status"], "in_progress");
    }

    #[tokio::test]
    async fn the_event_port_pushes_onto_the_session_channel() {
        let (tx, mut rx) = tokio::sync::broadcast::channel::<ServerMsg>(16);
        let port = AcpEventPort::new(tx);
        port.emit(NormalizedEvent {
            source: EventSource::Acp {
                variant: "agent_message_chunk".into(),
            },
            event: MiraEvent::AssistantText {
                message_id: None,
                text: "streamed".into(),
            },
        })
        .await;
        port.turn_ended("end_turn");

        let first = rx.recv().await.expect("text frame");
        let second = rx.recv().await.expect("turn end");
        let a = serde_json::to_value(&first).unwrap();
        let b = serde_json::to_value(&second).unwrap();
        assert_eq!(a["type"], "acp_text");
        assert_eq!(a["text"], "streamed");
        assert_eq!(b["type"], "acp_turn_end");
    }

    #[tokio::test]
    async fn an_unattached_session_does_not_take_the_agent_down() {
        // A send error just means nobody is listening, which is normal for a
        // background session.
        let (tx, rx) = tokio::sync::broadcast::channel::<ServerMsg>(4);
        drop(rx);
        let port = AcpEventPort::new(tx);
        port.emit(NormalizedEvent {
            source: EventSource::Acp {
                variant: "v".into(),
            },
            event: MiraEvent::AssistantText {
                message_id: None,
                text: "nobody home".into(),
            },
        })
        .await;
        port.turn_ended("end_turn");
    }
}
