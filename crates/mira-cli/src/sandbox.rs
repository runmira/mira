//! `mira --sandbox <backend>`: run the session's tools in an isolated
//! workspace instead of on the checkout.
//!
//! Start: create the backend, upload the project (git-tracked files plus
//! untracked-but-not-ignored ones). Finish: collect every change as a
//! patch under `~/.mira/sandbox/` and print how to apply it. The local
//! checkout is never modified by the session itself.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use mira_compute::{workspace, ComputeBackend, E2bBackend, E2bOptions, LocalBackend};
use mira_config::ComputeConfig;

pub const BACKENDS: &[&str] = &["local", "e2b"];

/// The backend to use: `--sandbox` wins over `compute.backend`.
pub fn requested(flag: Option<&str>, cfg: &ComputeConfig) -> Option<String> {
    flag.map(str::to_owned).or_else(|| cfg.backend.clone())
}

pub struct ActiveSandbox {
    backend: Arc<dyn ComputeBackend>,
    project: PathBuf,
    /// E2B sandbox id, for the session banner.
    label: String,
}

async fn create(name: &str, cfg: &ComputeConfig) -> Result<(Arc<dyn ComputeBackend>, String)> {
    match name {
        "local" => {
            let b = LocalBackend::scratch()?;
            let label = b.root().display().to_string();
            Ok((Arc::new(b), label))
        }
        "e2b" => {
            let e = &cfg.e2b;
            let key_env = e.api_key_env();
            let key = std::env::var(key_env).map_err(|_| {
                anyhow!(
                    "--sandbox e2b needs an API key: set {key_env} (or add it under `keys:` in \
                     ~/.mira/mira.yaml). Get one at https://e2b.dev"
                )
            })?;
            let mut opts = E2bOptions::new(key);
            if let Some(t) = &e.template {
                opts.template = t.clone();
            }
            if let Some(t) = e.timeout_secs {
                opts.timeout_secs = t.clamp(60, 24 * 3600);
            }
            if let Some(u) = &e.api_url {
                opts.api_url = u.trim_end_matches('/').to_owned();
            }
            if let Some(d) = &e.domain {
                opts.domain = d.clone();
            }
            let b = E2bBackend::create(opts).await?;
            let label = format!("sandbox {}", b.sandbox_id());
            Ok((Arc::new(b), label))
        }
        other => bail!(
            "unknown sandbox backend `{other}` (expected one of: {})",
            BACKENDS.join(", ")
        ),
    }
}

impl ActiveSandbox {
    /// Create the backend and upload `project` into it.
    pub async fn start(name: &str, cfg: &ComputeConfig, project: &Path) -> Result<Self> {
        let archive = {
            let project = project.to_path_buf();
            tokio::task::spawn_blocking(move || workspace::pack(&project))
                .await
                .context("packing the project")??
        };
        let (backend, label) = create(name, cfg).await?;
        eprintln!(
            "sandbox: uploading {:.1} MB to {name} ({label})…",
            archive.len() as f64 / (1024.0 * 1024.0)
        );
        if let Err(e) = workspace::upload(backend.as_ref(), &archive).await {
            let _ = backend.shutdown().await;
            return Err(e).context("uploading the project to the sandbox");
        }
        eprintln!(
            "sandbox: ready. Tools run in {name}; your checkout stays untouched until you apply the patch."
        );
        Ok(Self {
            backend,
            project: project.to_path_buf(),
            label,
        })
    }

    pub fn backend(&self) -> Arc<dyn ComputeBackend> {
        self.backend.clone()
    }

    /// Appended to the system prompt so the model knows where it is.
    pub fn prompt_note(&self) -> String {
        format!(
            "\n\nSANDBOX. Your tools run in an isolated `{}` sandbox, not on the user's \
             machine. The project is uploaded at {} (paths under {} map there too). \
             It's a fresh git repo whose only commit is the uploaded state, so \
             `git diff` shows exactly your changes. Nothing reaches the user's \
             checkout until they apply the patch Mira produces at the end of the \
             session, so don't tell them a change is \"on their machine\".",
            self.backend.name(),
            self.backend.workspace_root(),
            self.project.display(),
        )
    }

    /// Save the session's changes as a patch, print how to apply them,
    /// and release the backend.
    pub async fn finish(self) -> Result<()> {
        let result = self.save_patch().await;
        if let Err(e) = self.backend.shutdown().await {
            eprintln!("sandbox: shutdown failed ({e}); it will expire on its own");
        }
        match result {
            Ok(Some(path)) => {
                eprintln!(
                    "\nApply them to your checkout with:\n  git -C {} apply {}",
                    self.project.display(),
                    path.display()
                );
                Ok(())
            }
            Ok(None) => {
                eprintln!("sandbox: no changes.");
                Ok(())
            }
            Err(e) => Err(e.context(format!(
                "collecting changes from {} failed; the sandbox has been shut down",
                self.label
            ))),
        }
    }

    async fn save_patch(&self) -> Result<Option<PathBuf>> {
        let patch = workspace::diff(self.backend.as_ref()).await?;
        if patch.trim().is_empty() {
            return Ok(None);
        }
        let stat = workspace::diff_stat(self.backend.as_ref())
            .await
            .unwrap_or_default();
        let dir = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default()
            .join(".mira")
            .join("sandbox");
        std::fs::create_dir_all(&dir)?;
        let name = self
            .project
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "project".into());
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        let path = dir.join(format!("{name}-{ts}.patch"));
        std::fs::write(&path, patch)?;
        eprintln!(
            "\nsandbox: changes saved to {}\n{}",
            path.display(),
            stat.trim_end()
        );
        Ok(Some(path))
    }
}
