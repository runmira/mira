//! Run against a directory on this machine.
//!
//! Commands go through `mira-sandbox`, so they get the same OS isolation
//! (Seatbelt / Bubblewrap) as the default local path, with the backend's
//! root as the sandbox boundary. With `--sandbox local` that root is a
//! scratch copy of the repo, so the user's real checkout stays untouched
//! until they apply the resulting patch.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use mira_sandbox::Sandbox;
use tokio::sync::mpsc;

use crate::{ComputeBackend, ComputeError, ComputeEvent, ExecOutput, ExecRequest, Result};

pub struct LocalBackend {
    root: PathBuf,
    sandbox: Sandbox,
    /// Delete `root` on shutdown (scratch copies only).
    owned: bool,
}

impl LocalBackend {
    /// Work in `root` itself. It's never deleted.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            sandbox: Sandbox::new(&root),
            root,
            owned: false,
        }
    }

    /// A fresh scratch directory under the system temp dir, removed on
    /// [`ComputeBackend::shutdown`]. Starts empty; fill it with
    /// [`crate::workspace::upload`].
    pub fn scratch() -> Result<Self> {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let root = std::env::temp_dir().join(format!("mira-sandbox-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&root)?;
        let root = std::fs::canonicalize(&root)?;
        Ok(Self {
            sandbox: Sandbox::new(&root),
            root,
            owned: true,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn abs(&self, rel: &str) -> PathBuf {
        if rel.is_empty() {
            self.root.clone()
        } else {
            self.root.join(rel)
        }
    }
}

#[async_trait]
impl ComputeBackend for LocalBackend {
    fn name(&self) -> &'static str {
        "local"
    }

    fn workspace_root(&self) -> String {
        self.root.to_string_lossy().into_owned()
    }

    async fn exec(
        &self,
        req: ExecRequest,
        sink: Option<mpsc::Sender<ComputeEvent>>,
    ) -> Result<ExecOutput> {
        let cwd = self.abs(&req.cwd);
        let args = vec!["-lc".to_owned(), req.command];
        let out = self
            .sandbox
            .run_with_timeout("bash", &args, &cwd, req.timeout.as_secs().max(1))
            .await
            .map_err(|e| ComputeError::Remote(e.to_string()))?;
        // The local runner collects output at exit; replay it so callers
        // that render a live tail still see it.
        if let Some(tx) = sink {
            for line in out.stdout.lines() {
                let _ = tx.send(ComputeEvent::Stdout(line.to_owned())).await;
            }
            for line in out.stderr.lines() {
                let _ = tx.send(ComputeEvent::Stderr(line.to_owned())).await;
            }
        }
        Ok(ExecOutput {
            stdout: out.stdout,
            stderr: out.stderr,
            exit_code: out.exit_code,
            timed_out: out.timed_out,
        })
    }

    async fn read_file(&self, rel: &str) -> Result<Vec<u8>> {
        let p = self.abs(rel);
        tokio::fs::read(&p).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ComputeError::NotFound(format!("no such file: {rel}"))
            } else {
                e.into()
            }
        })
    }

    async fn write_file(&self, rel: &str, data: &[u8]) -> Result<()> {
        if rel.is_empty() {
            return Err(ComputeError::InvalidPath(
                "cannot write the workspace root".into(),
            ));
        }
        let p = self.abs(rel);
        if let Some(parent) = p.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&p, data).await?;
        Ok(())
    }

    async fn shutdown(&self) -> Result<()> {
        if self.owned && self.root.exists() {
            tokio::fs::remove_dir_all(&self.root).await?;
        }
        Ok(())
    }
}
