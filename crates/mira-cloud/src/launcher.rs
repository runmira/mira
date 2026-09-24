//! Starting a cloud task from your machine.
//!
//! Create an E2B sandbox that lives for the task's time budget, make sure
//! Mira is inside it, write the spec and secrets, start the worker
//! detached, check that it came up, and hand the sandbox over. Returns in
//! seconds; the laptop can close right after.

use std::sync::Arc;

use mira_compute::env::BackendSpec;
use mira_compute::{ComputeBackend, E2bBackend, EnvironmentSpec, ExecRequest};

use crate::spec::{paths, Secrets, TaskSpec};
use crate::store::TaskRecord;
use crate::CloudError;

/// How Mira gets into the sandbox.
#[derive(Clone, Debug)]
pub enum MiraInstall {
    /// The sandbox template already has `mira` on PATH (fastest; see
    /// docs/cloud-tasks.md for the template).
    Preinstalled,
    /// Upload this Linux x86_64 binary (e.g. the running `mira`, when
    /// launching from Linux).
    Upload(std::path::PathBuf),
    /// Install this release (`v0.3.10`) with the project's install.sh.
    Release(String),
}

/// Seconds added to the sandbox's lifetime on top of the task budget, so
/// the worker's final push never races the sandbox's death.
const SANDBOX_GRACE_SECS: u64 = 120;

pub type Progress = Arc<dyn Fn(String) + Send + Sync>;

pub struct Launch {
    pub spec: TaskSpec,
    pub secrets: Secrets,
    pub environment: EnvironmentSpec,
    pub install: MiraInstall,
    /// Command the sandbox runs to start the worker; defaults to
    /// `<mira> cloud worker …`. Tests override it.
    pub worker_command: Option<String>,
}

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

async fn run_ok(backend: &dyn ComputeBackend, what: &str, cmd: &str) -> Result<String, CloudError> {
    let out = backend
        .exec(
            ExecRequest::new(cmd).timeout(std::time::Duration::from_secs(600)),
            None,
        )
        .await?;
    if out.exit_code != Some(0) {
        return Err(CloudError::Sandbox(format!(
            "{what} failed (exit {:?}): {}",
            out.exit_code,
            out.stderr.trim()
        )));
    }
    Ok(out.stdout)
}

impl Launch {
    pub async fn run(mut self, progress: Progress) -> Result<TaskRecord, CloudError> {
        let BackendSpec::E2b(mut opts) = self.environment.backend.clone() else {
            return Err(CloudError::Config(format!(
                "cloud tasks need an E2B environment; `{}` is `{}`",
                self.environment.name,
                self.environment.backend_name()
            )));
        };
        opts.timeout_secs = self.spec.limits.max_runtime_secs + SANDBOX_GRACE_SECS;
        opts.env = self.environment.env.clone();
        let api_url = opts.api_url.clone();

        progress(format!("starting an E2B sandbox ({})…", opts.template));
        let backend = E2bBackend::create(opts).await?;
        self.spec.sandbox_id = Some(backend.sandbox_id().to_owned());
        self.spec.e2b_api_url = Some(api_url);
        // Relative to the worker's cwd (the sandbox workspace root).
        self.spec.workdir = std::path::PathBuf::from(paths::REPO);
        if self.spec.setup.is_none() {
            self.spec.setup = self.environment.setup.clone();
        }

        match self.prepare(&backend, &progress).await {
            Ok(record) => {
                backend.detach();
                Ok(record)
            }
            Err(e) => {
                let _ = backend.shutdown().await;
                Err(e)
            }
        }
    }

    async fn prepare(
        &self,
        backend: &E2bBackend,
        progress: &Progress,
    ) -> Result<TaskRecord, CloudError> {
        let mira = match &self.install {
            MiraInstall::Preinstalled => run_ok(backend, "finding mira", "command -v mira")
                .await
                .map_err(|_| {
                    CloudError::Config(
                        "the sandbox template has no `mira` on PATH; use a template with Mira \
                         installed (docs/cloud-tasks.md) or set `cloud.install`"
                            .into(),
                    )
                })?
                .trim()
                .to_owned(),
            MiraInstall::Upload(path) => {
                progress("uploading mira…".into());
                let bin = tokio::fs::read(path).await?;
                backend.write_file(".mira-task/bin/mira", &bin).await?;
                run_ok(backend, "installing mira", "chmod +x .mira-task/bin/mira").await?;
                // Relative: the worker command runs from the workspace root.
                "./.mira-task/bin/mira".to_owned()
            }
            MiraInstall::Release(version) => {
                progress(format!("installing mira {version}…"));
                run_ok(
                    backend,
                    "installing mira",
                    &format!(
                        "curl -fsSL https://raw.githubusercontent.com/runmira/mira/main/install.sh \
                         | MIRA_VERSION={} MIRA_INSTALL_DIR=$HOME/.local/bin bash >/dev/null",
                        sh_quote(version)
                    ),
                )
                .await?;
                "$HOME/.local/bin/mira".to_owned()
            }
        };

        progress("handing over the task…".into());
        let spec_json =
            serde_json::to_vec_pretty(&self.spec).map_err(|e| CloudError::Config(e.to_string()))?;
        backend.write_file(paths::SPEC, &spec_json).await?;
        let secrets_json =
            serde_json::to_vec(&self.secrets).map_err(|e| CloudError::Config(e.to_string()))?;
        backend.write_file(paths::SECRETS, &secrets_json).await?;
        run_ok(
            backend,
            "securing secrets",
            &format!("chmod 600 {}", paths::SECRETS),
        )
        .await?;

        let worker = self.worker_command.clone().unwrap_or_else(|| {
            format!(
                "{mira} cloud worker --spec {} --secrets {}",
                paths::SPEC,
                paths::SECRETS
            )
        });
        // setsid + nohup: the worker outlives this exec call and any
        // connection from the laptop.
        run_ok(
            backend,
            "starting the worker",
            &format!(
                "(command -v setsid >/dev/null && S=setsid || S=); \
                 nohup $S {worker} > {log} 2>&1 < /dev/null & echo $! > .mira-task/worker.pid",
                log = paths::LOG
            ),
        )
        .await?;
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let alive = run_ok(
            backend,
            "checking the worker",
            "kill -0 \"$(cat .mira-task/worker.pid)\" 2>/dev/null && echo alive || echo dead",
        )
        .await?;
        if alive.trim() != "alive" {
            let log = backend
                .read_file(paths::LOG)
                .await
                .map(|b| String::from_utf8_lossy(&b).into_owned())
                .unwrap_or_default();
            // A fast worker may already be done; only fail on an error.
            if !log.contains("finished:") {
                return Err(CloudError::Sandbox(format!(
                    "the worker exited right away:\n{}",
                    log.lines()
                        .rev()
                        .take(15)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect::<Vec<_>>()
                        .join("\n")
                )));
            }
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        Ok(TaskRecord {
            id: self.spec.id.clone(),
            prompt: self.spec.prompt.clone(),
            repo: format!("{}/{}", self.spec.repo.owner, self.spec.repo.name),
            api_url: self.spec.repo.api_url.clone(),
            branch: self.spec.branch.clone(),
            base_branch: self.spec.base_branch.clone(),
            environment: self.environment.name.clone(),
            sandbox_id: backend.sandbox_id().to_owned(),
            envd_access_token: backend.envd_access_token().map(str::to_owned),
            created_at: now,
            max_runtime_secs: self.spec.limits.max_runtime_secs,
        })
    }
}

/// Last `lines` lines of a running task's log.
pub async fn tail_log(
    opts: mira_compute::E2bOptions,
    record: &TaskRecord,
    lines: usize,
) -> Result<String, CloudError> {
    let backend = E2bBackend::attach(opts, &record.sandbox_id, record.envd_access_token.clone())?;
    let bytes = backend.read_file(paths::LOG).await?;
    let text = String::from_utf8_lossy(&bytes);
    let all: Vec<&str> = text.lines().collect();
    Ok(all[all.len().saturating_sub(lines)..].join("\n"))
}
