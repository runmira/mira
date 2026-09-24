//! Named environments and switching a live session between them.
//!
//! The user's worktree is the anchor. `local` means tools run on it
//! directly. A remote environment is a copy of it: switching there
//! uploads the worktree's current state (uncommitted edits included), and
//! switching back three-way-merges the changes into the worktree
//! ([`workspace::pull_into`]). A remote environment you leave is parked
//! (E2B: paused; scratch: kept), so coming back to it is quick and its
//! installed dependencies and build caches are still warm.
//!
//! One [`EnvironmentManager`] per session; the TUI's `/remote-env` and
//! the web UI's environment switcher both drive it.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use mira_config::ComputeConfig;
use tokio::sync::{mpsc, Mutex};

use crate::workspace::{self, PullReport};
use crate::{
    ComputeBackend, ComputeError, ComputeEvent, E2bBackend, E2bOptions, ExecRequest, LocalBackend,
    Result,
};

/// Name of the user's own machine as a switch target.
pub const LOCAL: &str = "local";

/// Longest a setup script may run.
const SETUP_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Shared handle to the session's current backend. Tools read it on every
/// call, so a switch takes effect for the very next tool call — including
/// in subagents, which share the parent's slot.
#[derive(Clone, Default)]
pub struct ComputeSlot(Arc<RwLock<Option<Arc<dyn ComputeBackend>>>>);

impl ComputeSlot {
    pub fn new(backend: Option<Arc<dyn ComputeBackend>>) -> Self {
        Self(Arc::new(RwLock::new(backend)))
    }

    /// The current backend; `None` = the local machine.
    pub fn get(&self) -> Option<Arc<dyn ComputeBackend>> {
        self.0.read().expect("compute slot poisoned").clone()
    }

    pub fn set(&self, backend: Option<Arc<dyn ComputeBackend>>) {
        *self.0.write().expect("compute slot poisoned") = backend;
    }

    pub fn is_remote(&self) -> bool {
        self.0.read().expect("compute slot poisoned").is_some()
    }
}

impl std::fmt::Debug for ComputeSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.get() {
            Some(b) => write!(f, "ComputeSlot({})", b.name()),
            None => write!(f, "ComputeSlot(local)"),
        }
    }
}

#[derive(Clone, Debug)]
pub enum BackendSpec {
    Scratch,
    E2b(E2bOptions),
}

/// A resolved environment, ready to start.
#[derive(Clone, Debug)]
pub struct EnvironmentSpec {
    pub name: String,
    pub description: Option<String>,
    pub backend: BackendSpec,
    pub env: BTreeMap<String, String>,
    pub setup: Option<String>,
}

/// One row of [`EnvironmentManager::list`].
#[derive(Clone, Debug, serde::Serialize)]
pub struct EnvironmentInfo {
    pub name: String,
    /// `local`, `scratch` or `e2b`.
    pub backend: String,
    pub description: String,
}

fn e2b_options(cfg: &ComputeConfig) -> Result<E2bOptions> {
    let e = &cfg.e2b;
    let key_env = e.api_key_env();
    let key = std::env::var(key_env).map_err(|_| {
        ComputeError::Config(format!(
            "E2B needs an API key: set {key_env} (or put it under `keys:` in ~/.mira/mira.yaml). \
             Get one at https://e2b.dev"
        ))
    })?;
    let mut opts = E2bOptions::new(key);
    if let Some(t) = &e.template {
        opts.template = t.clone();
    }
    if let Some(t) = e.timeout_secs {
        opts.timeout_secs = t;
    }
    if let Some(u) = &e.api_url {
        opts.api_url = u.trim_end_matches('/').to_owned();
    }
    if let Some(d) = &e.domain {
        opts.domain = d.clone();
    }
    Ok(opts)
}

impl EnvironmentSpec {
    /// Resolve `name` against the config: a declared environment, or the
    /// built-ins `scratch` / `e2b`.
    pub fn resolve(name: &str, cfg: &ComputeConfig) -> Result<Self> {
        if name == LOCAL {
            return Err(ComputeError::Config(
                "`local` is the user's own machine, not a remote environment".into(),
            ));
        }
        if let Some(e) = cfg.environments.get(name) {
            let backend = match e.backend.as_str() {
                "scratch" => BackendSpec::Scratch,
                "e2b" => {
                    let mut o = e2b_options(cfg)?;
                    if let Some(t) = &e.template {
                        o.template = t.clone();
                    }
                    if let Some(t) = e.timeout_secs {
                        o.timeout_secs = t;
                    }
                    o.timeout_secs = o.timeout_secs.clamp(60, 24 * 3600);
                    o.env = e.env.clone();
                    BackendSpec::E2b(o)
                }
                other => {
                    return Err(ComputeError::Config(format!(
                        "environment `{name}`: unknown backend `{other}` (expected e2b or scratch)"
                    )))
                }
            };
            return Ok(Self {
                name: name.to_owned(),
                description: e.description.clone(),
                backend,
                env: e.env.clone(),
                setup: e.setup.clone(),
            });
        }
        let backend = match name {
            "scratch" => BackendSpec::Scratch,
            "e2b" => BackendSpec::E2b(e2b_options(cfg)?),
            _ => {
                let known: Vec<String> = list(cfg).into_iter().map(|e| e.name).collect();
                return Err(ComputeError::Config(format!(
                    "no environment named `{name}` (known: {})",
                    known.join(", ")
                )));
            }
        };
        Ok(Self {
            name: name.to_owned(),
            description: None,
            backend,
            env: BTreeMap::new(),
            setup: None,
        })
    }

    pub fn backend_name(&self) -> &'static str {
        match self.backend {
            BackendSpec::Scratch => "scratch",
            BackendSpec::E2b(_) => "e2b",
        }
    }
}

/// Every environment the user can switch to, `local` first.
pub fn list(cfg: &ComputeConfig) -> Vec<EnvironmentInfo> {
    let mut out = vec![EnvironmentInfo {
        name: LOCAL.into(),
        backend: LOCAL.into(),
        description: "this machine, directly on the worktree".into(),
    }];
    for (name, e) in &cfg.environments {
        out.push(EnvironmentInfo {
            name: name.clone(),
            backend: e.backend.clone(),
            description: e.description.clone().unwrap_or_default(),
        });
    }
    for (name, backend, desc) in [
        (
            "scratch",
            "scratch",
            "a copy of the worktree on this machine",
        ),
        ("e2b", "e2b", "an E2B cloud sandbox with default settings"),
    ] {
        if !cfg.environments.contains_key(name) {
            out.push(EnvironmentInfo {
                name: name.into(),
                backend: backend.into(),
                description: desc.into(),
            });
        }
    }
    out
}

/// Status line for UIs.
#[derive(Clone, Debug, serde::Serialize)]
pub struct EnvironmentStatus {
    /// `local` or the active environment's name.
    pub current: String,
    pub backend: String,
    /// Where the project lives in the backend (remote only).
    pub workspace: Option<String>,
    /// Environments left running or paused, ready to switch back to.
    pub parked: Vec<String>,
}

/// Outcome of [`EnvironmentManager::switch`].
#[derive(Clone, Debug, Default)]
pub struct SwitchReport {
    pub from: String,
    pub to: String,
    /// Changes merged into the worktree when leaving a remote environment.
    pub pulled: Option<PullReport>,
    /// Message for the model's context describing the new situation.
    pub model_note: String,
}

/// Progress reporting for slow steps (upload, setup script output).
pub type Progress = Arc<dyn Fn(String) + Send + Sync>;

struct Active {
    spec: EnvironmentSpec,
    backend: Arc<dyn ComputeBackend>,
}

enum Parked {
    /// Still running (scratch copies).
    Live(Active),
    /// E2B sandbox paused; resume by id.
    Paused {
        spec: EnvironmentSpec,
        sandbox_id: String,
    },
}

#[derive(Default)]
struct State {
    active: Option<Active>,
    parked: HashMap<String, Parked>,
}

pub struct EnvironmentManager {
    project: PathBuf,
    patch_dir: PathBuf,
    cfg: ComputeConfig,
    slot: ComputeSlot,
    state: Mutex<State>,
    switching: std::sync::atomic::AtomicBool,
}

/// Clears the "switch in progress" flag however `switch` returns.
struct SwitchingGuard<'a>(&'a std::sync::atomic::AtomicBool);

impl Drop for SwitchingGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

impl EnvironmentManager {
    /// `project` is the worktree; patches are kept under `patch_dir`.
    pub fn new(
        project: impl Into<PathBuf>,
        patch_dir: impl Into<PathBuf>,
        cfg: ComputeConfig,
    ) -> Self {
        Self {
            project: project.into(),
            patch_dir: patch_dir.into(),
            cfg,
            slot: ComputeSlot::default(),
            state: Mutex::new(State::default()),
            switching: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// `~/.mira/sandbox`, where pulled patches are kept.
    pub fn default_patch_dir() -> PathBuf {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default()
            .join(".mira")
            .join("sandbox")
    }

    /// The slot to hand to the session's `ToolContext`.
    pub fn slot(&self) -> ComputeSlot {
        self.slot.clone()
    }

    pub fn project(&self) -> &Path {
        &self.project
    }

    /// True while a switch is moving files around. UIs should hold new
    /// turns until it's done, so no tool runs against a half-moved tree.
    pub fn is_switching(&self) -> bool {
        self.switching.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn list(&self) -> Vec<EnvironmentInfo> {
        list(&self.cfg)
    }

    pub async fn status(&self) -> EnvironmentStatus {
        let st = self.state.lock().await;
        let mut parked: Vec<String> = st.parked.keys().cloned().collect();
        parked.sort();
        match &st.active {
            Some(a) => EnvironmentStatus {
                current: a.spec.name.clone(),
                backend: a.spec.backend_name().into(),
                workspace: Some(a.backend.workspace_root()),
                parked,
            },
            None => EnvironmentStatus {
                current: LOCAL.into(),
                backend: LOCAL.into(),
                workspace: None,
                parked,
            },
        }
    }

    /// Switch the session to `target` (`local` or an environment name).
    ///
    /// Leaving a remote environment first merges its changes into the
    /// worktree. If that merge can't be applied at all, the switch is
    /// aborted and the session stays where it was, with the patch saved.
    pub async fn switch(&self, target: &str, progress: Progress) -> Result<SwitchReport> {
        let target = match target.trim() {
            "" | "off" | "host" => LOCAL,
            t => t,
        };
        let mut st = self.state.lock().await;
        self.switching
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let _guard = SwitchingGuard(&self.switching);
        let from = st
            .active
            .as_ref()
            .map_or(LOCAL.to_owned(), |a| a.spec.name.clone());
        if from == target {
            return Ok(SwitchReport {
                from: from.clone(),
                to: from,
                ..Default::default()
            });
        }
        // Resolve before touching anything, so a typo or a missing key
        // doesn't pull changes back for nothing.
        let spec = if target == LOCAL {
            None
        } else {
            Some(EnvironmentSpec::resolve(target, &self.cfg)?)
        };

        let mut report = SwitchReport {
            from: from.clone(),
            to: target.to_owned(),
            ..Default::default()
        };

        if let Some(active) = st.active.take() {
            progress(format!(
                "bringing changes back from `{}`…",
                active.spec.name
            ));
            let pulled =
                match workspace::pull_into(active.backend.as_ref(), &self.project, &self.patch_dir)
                    .await
                {
                    Ok(p) => p,
                    Err(e) => {
                        st.active = Some(active);
                        return Err(e);
                    }
                };
            if let Some(why) = &pulled.failed {
                let path = pulled
                    .patch_path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                st.active = Some(active);
                return Err(ComputeError::Remote(format!(
                    "couldn't merge the changes into {} ({why}); staying in `{from}`. \
                     The patch is saved at {path}",
                    self.project.display()
                )));
            }
            self.slot.set(None);
            park(&mut st, active, &progress).await;
            report.pulled = Some(pulled);
        }

        let Some(spec) = spec else {
            report.model_note = local_note(&self.project, &from, report.pulled.as_ref());
            return Ok(report);
        };

        let archive = {
            let project = self.project.clone();
            tokio::task::spawn_blocking(move || workspace::pack(&project))
                .await
                .map_err(|e| ComputeError::Remote(format!("packing: {e}")))??
        };

        let backend = match st.parked.remove(&spec.name) {
            Some(Parked::Live(a)) => {
                progress(format!("resyncing `{}`…", spec.name));
                a.backend
            }
            Some(Parked::Paused {
                spec: pspec,
                sandbox_id,
            }) => {
                progress(format!("resuming `{}`…", spec.name));
                match &pspec.backend {
                    BackendSpec::E2b(o) => match E2bBackend::resume(o.clone(), &sandbox_id).await {
                        Ok(b) => Arc::new(b) as Arc<dyn ComputeBackend>,
                        Err(e) => {
                            progress(format!("resume failed ({e}); starting fresh"));
                            self.start_fresh(&spec, &progress).await?
                        }
                    },
                    BackendSpec::Scratch => self.start_fresh(&spec, &progress).await?,
                }
            }
            None => self.start_fresh(&spec, &progress).await?,
        };
        let fresh = !backend_has_baseline(backend.as_ref()).await;

        progress(format!(
            "uploading {:.1} MB to `{}`…",
            archive.len() as f64 / (1024.0 * 1024.0),
            spec.name
        ));
        if let Err(e) = workspace::upload(backend.as_ref(), &archive).await {
            let _ = backend.shutdown().await;
            return Err(e);
        }
        if fresh {
            if let Some(script) = &spec.setup {
                progress(format!("running setup for `{}`…", spec.name));
                if let Err(e) = run_setup(backend.as_ref(), script, &progress).await {
                    let _ = backend.shutdown().await;
                    return Err(e);
                }
            }
        }

        report.model_note = remote_note(&spec, backend.as_ref(), &self.project);
        self.slot.set(Some(backend.clone()));
        st.active = Some(Active { spec, backend });
        Ok(report)
    }

    async fn start_fresh(
        &self,
        spec: &EnvironmentSpec,
        progress: &Progress,
    ) -> Result<Arc<dyn ComputeBackend>> {
        progress(format!(
            "starting `{}` ({})…",
            spec.name,
            spec.backend_name()
        ));
        Ok(match &spec.backend {
            BackendSpec::Scratch => Arc::new(LocalBackend::scratch()?.with_env(spec.env.clone())),
            BackendSpec::E2b(o) => Arc::new(E2bBackend::create(o.clone()).await?),
        })
    }

    /// End of session: save (never apply) the active environment's
    /// changes as a patch, then release every environment. Returns the
    /// report for the caller to show.
    pub async fn finish(&self) -> Result<Option<PullReport>> {
        let mut st = self.state.lock().await;
        let mut result = Ok(None);
        if let Some(a) = st.active.take() {
            result = save_patch(a.backend.as_ref(), &self.project, &self.patch_dir)
                .await
                .map(Some);
            let _ = a.backend.shutdown().await;
        }
        self.slot.set(None);
        for (_, p) in st.parked.drain() {
            match p {
                Parked::Live(a) => {
                    let _ = a.backend.shutdown().await;
                }
                Parked::Paused { spec, sandbox_id } => {
                    if let BackendSpec::E2b(o) = spec.backend {
                        let _ = E2bBackend::kill(&o, &sandbox_id).await;
                    }
                }
            }
        }
        result
    }
}

async fn backend_has_baseline(backend: &dyn ComputeBackend) -> bool {
    backend
        .exec(
            ExecRequest::new(format!(
                "git rev-parse -q --verify {} >/dev/null",
                workspace::BASELINE_TAG
            )),
            None,
        )
        .await
        .map(|o| o.exit_code == Some(0))
        .unwrap_or(false)
}

async fn park(st: &mut State, active: Active, progress: &Progress) {
    let name = active.spec.name.clone();
    match active.spec.backend {
        BackendSpec::Scratch => {
            st.parked.insert(name, Parked::Live(active));
        }
        BackendSpec::E2b(_) => match active.backend.checkpoint().await {
            Ok(sandbox_id) => {
                progress(format!("paused `{name}`; switch back to resume it"));
                st.parked.insert(
                    name,
                    Parked::Paused {
                        spec: active.spec,
                        sandbox_id,
                    },
                );
            }
            Err(e) => {
                progress(format!("couldn't pause `{name}` ({e}); shutting it down"));
                let _ = active.backend.shutdown().await;
            }
        },
    }
}

async fn run_setup(backend: &dyn ComputeBackend, script: &str, progress: &Progress) -> Result<()> {
    let (tx, mut rx) = mpsc::channel::<ComputeEvent>(256);
    let p = progress.clone();
    let forward = tokio::spawn(async move {
        while let Some(ev) = rx.recv().await {
            let (ComputeEvent::Stdout(l) | ComputeEvent::Stderr(l)) = ev;
            p(format!("  {l}"));
        }
    });
    let out = backend
        .exec(
            ExecRequest::new(script.to_owned()).timeout(SETUP_TIMEOUT),
            Some(tx),
        )
        .await;
    let _ = forward.await;
    let out = out?;
    if out.exit_code != Some(0) {
        let tail: Vec<&str> = out.stderr.lines().rev().take(10).collect();
        return Err(ComputeError::Remote(format!(
            "setup script failed (exit {:?}{}): {}",
            out.exit_code,
            if out.timed_out { ", timed out" } else { "" },
            tail.into_iter().rev().collect::<Vec<_>>().join("\n")
        )));
    }
    Ok(())
}

/// Save the active environment's pending changes without applying them.
async fn save_patch(
    backend: &dyn ComputeBackend,
    project: &Path,
    dir: &Path,
) -> Result<PullReport> {
    let patch = workspace::diff(backend).await?;
    if patch.trim().is_empty() {
        return Ok(PullReport::default());
    }
    let stat = workspace::diff_stat(backend).await.unwrap_or_default();
    std::fs::create_dir_all(dir)?;
    let name = project
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "project".into());
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    let path = dir.join(format!("{name}-{ts}.patch"));
    std::fs::write(&path, patch)?;
    Ok(PullReport {
        stat,
        patch_path: Some(path),
        ..Default::default()
    })
}

fn remote_note(spec: &EnvironmentSpec, backend: &dyn ComputeBackend, project: &Path) -> String {
    format!(
        "[environment] The session's tools now run in the remote environment `{}` ({}), not on \
         the user's machine. The project is at {} (paths under {} map there), and it's a git \
         repo whose latest commit is the state you were given, so `git diff` shows your \
         changes. Only file, shell and search tools work there; git, code-intel, MCP and \
         desktop tools are hidden until the user switches back to local. Changes reach the \
         user's worktree when they switch back.",
        spec.name,
        spec.backend_name(),
        backend.workspace_root(),
        project.display(),
    )
}

fn local_note(project: &Path, from: &str, pulled: Option<&PullReport>) -> String {
    let mut note = format!(
        "[environment] The session's tools run on the user's machine again, directly in {}.",
        project.display()
    );
    match pulled {
        Some(p) if p.changed() => {
            note.push_str(&format!(
                " Changes made in `{from}` were merged into the worktree:\n{}",
                p.stat.trim_end()
            ));
            if !p.conflicts.is_empty() {
                note.push_str(&format!(
                    "\nThese files have merge conflicts to resolve: {}",
                    p.conflicts.join(", ")
                ));
            }
        }
        _ => note.push_str(&format!(" `{from}` had no changes to bring back.")),
    }
    note
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_and_named_environments_resolve() {
        let cfg: ComputeConfig = serde_yaml::from_str(
            "environments:\n  quick:\n    backend: scratch\n    env: {A: '1'}\n    setup: echo hi\n",
        )
        .unwrap();
        let q = EnvironmentSpec::resolve("quick", &cfg).unwrap();
        assert_eq!(q.backend_name(), "scratch");
        assert_eq!(q.env["A"], "1");
        assert!(EnvironmentSpec::resolve("scratch", &cfg).is_ok());
        assert!(EnvironmentSpec::resolve("local", &cfg).is_err());
        let err = EnvironmentSpec::resolve("nope", &cfg)
            .unwrap_err()
            .to_string();
        assert!(err.contains("quick") && err.contains("scratch"), "{err}");
        let names: Vec<String> = list(&cfg).into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["local", "quick", "scratch", "e2b"]);
    }
}
