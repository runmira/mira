//! Getting the project into a sandbox and the result back out.
//!
//! 1. [`pack`] the local project into a `.tar.gz`: git-tracked plus
//!    untracked-but-not-ignored files, so build output, `node_modules`
//!    and secrets in `.gitignore` never leave the machine.
//! 2. [`upload`] extracts it into the backend's workspace and commits it
//!    as a `mira-baseline` tag in a git repo there. Uploading again (a
//!    resumed sandbox) replaces the tracked files but keeps ignored ones,
//!    so warm build caches survive.
//! 3. The session works in the sandbox, which is the source of truth
//!    while it runs.
//! 4. [`diff`] returns everything that changed since the baseline as a
//!    binary-safe patch; [`pull_into`] merges it into the local worktree
//!    file by file (three-way, so the user's own edits survive) and moves
//!    the baseline forward.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::{ComputeBackend, ComputeError, ExecRequest, Result};

/// Refuse to upload more than this (compressed). A project that big
/// almost certainly has build output or data files that aren't ignored.
pub const MAX_ARCHIVE_BYTES: usize = 200 * 1024 * 1024;

/// Name of the git tag marking the uploaded state in the sandbox.
pub const BASELINE_TAG: &str = "mira-baseline";

const ARCHIVE_NAME: &str = ".mira-upload.tar.gz";

/// Directories skipped when the project isn't a git repo.
const SKIP_DIRS: &[&str] = &[".git", "target", "node_modules", ".venv", "dist", "build"];

/// Files to upload, relative to `root`.
fn list_files(root: &Path) -> Result<Vec<PathBuf>> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .output();
    if let Ok(out) = out {
        if out.status.success() {
            let mut files: Vec<PathBuf> = out
                .stdout
                .split(|b| *b == 0)
                .filter(|s| !s.is_empty())
                .map(|s| PathBuf::from(String::from_utf8_lossy(s).into_owned()))
                // Deleted-but-still-indexed files show up in ls-files.
                .filter(|p| root.join(p).symlink_metadata().is_ok())
                .collect();
            files.sort();
            files.dedup();
            return Ok(files);
        }
    }
    // Not a git repo: walk, skipping the usual heavy directories.
    let mut files = Vec::new();
    let mut stack = vec![PathBuf::new()];
    while let Some(rel) = stack.pop() {
        for entry in std::fs::read_dir(root.join(&rel))? {
            let entry = entry?;
            let name = entry.file_name();
            let child = rel.join(&name);
            let ft = entry.file_type()?;
            if ft.is_dir() {
                if !SKIP_DIRS.contains(&name.to_string_lossy().as_ref()) {
                    stack.push(child);
                }
            } else {
                files.push(child);
            }
        }
    }
    files.sort();
    Ok(files)
}

/// Pack the project at `root` into a gzipped tarball.
///
/// Symlinks are stored as links, never followed, so a link pointing
/// outside the project can't smuggle outside files into the upload.
pub fn pack(root: &Path) -> Result<Vec<u8>> {
    let files = list_files(root)?;
    let enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut tar = tar::Builder::new(enc);
    tar.follow_symlinks(false);
    for rel in &files {
        tar.append_path_with_name(root.join(rel), rel)?;
        if tar.get_ref().get_ref().len() > MAX_ARCHIVE_BYTES {
            return Err(too_big());
        }
    }
    let mut enc = tar.into_inner()?;
    enc.flush()?;
    let bytes = enc.finish()?;
    if bytes.len() > MAX_ARCHIVE_BYTES {
        return Err(too_big());
    }
    Ok(bytes)
}

fn too_big() -> ComputeError {
    ComputeError::Config(format!(
        "project is over {} MB compressed; add build output and data to .gitignore",
        MAX_ARCHIVE_BYTES / (1024 * 1024)
    ))
}

async fn run_checked(backend: &dyn ComputeBackend, what: &str, command: &str) -> Result<String> {
    let out = backend.exec(ExecRequest::new(command), None).await?;
    if out.exit_code != Some(0) {
        return Err(ComputeError::Remote(format!(
            "{what} failed (exit {:?}): {}{}",
            out.exit_code,
            out.stderr.trim(),
            if out.timed_out { " (timed out)" } else { "" }
        )));
    }
    Ok(out.stdout)
}

/// Extract `archive` into the backend's workspace and record the
/// baseline commit that [`diff`] compares against. Needs `tar` and `git`
/// in the sandbox image.
///
/// Safe to call again on a workspace that already has a baseline (a
/// resumed sandbox): tracked files are removed first, so deletions made
/// locally since take effect, while ignored files such as `target/` or
/// `node_modules/` stay.
pub async fn upload(backend: &dyn ComputeBackend, archive: &[u8]) -> Result<()> {
    backend.write_file(ARCHIVE_NAME, archive).await?;
    let script = format!(
        "set -e\n\
         if [ -d .git ]; then git ls-files -z | xargs -0 -r rm -f --; fi\n\
         tar -xzf {ARCHIVE_NAME}\n\
         rm -f {ARCHIVE_NAME}\n\
         [ -d .git ] || git init -q\n\
         {REBASELINE}"
    );
    run_checked(backend, "workspace setup", &script).await?;
    Ok(())
}

/// Commit everything in the sandbox and move the baseline tag there.
const REBASELINE: &str = "git add -A\n\
     git -c user.name=mira -c user.email=mira@localhost -c commit.gpgsign=false \
       commit -q --allow-empty --no-verify -m 'mira baseline'\n\
     git tag -f mira-baseline >/dev/null\n";

/// What [`pull_into`] did.
#[derive(Clone, Debug, Default)]
pub struct PullReport {
    /// `git diff --stat` of what came back; empty when nothing changed.
    pub stat: String,
    /// Where the patch was saved before applying (always kept, so a bad
    /// merge can be redone by hand).
    pub patch_path: Option<PathBuf>,
    /// Files left with conflict markers by the three-way merge.
    pub conflicts: Vec<String>,
    /// Set when the patch couldn't be applied at all; the worktree is
    /// unchanged and the user has to apply `patch_path` themselves.
    pub failed: Option<String>,
}

impl PullReport {
    pub fn changed(&self) -> bool {
        !self.stat.trim().is_empty()
    }
}

/// Bring the sandbox's changes into the local worktree at `project`.
///
/// Per file, a three-way merge between the sandbox's baseline (what was
/// uploaded), the sandbox's current version, and the worktree's current
/// version. Files the user didn't touch meanwhile are simply updated;
/// files both sides changed go through `git merge-file`, which leaves
/// conflict markers where the edits overlap. This works on dirty
/// worktrees (and non-git projects), which `git apply --3way` can't.
///
/// The full patch is saved under `patch_dir` first, so nothing is lost
/// even if a merge goes wrong. The sandbox's baseline then moves forward,
/// so the same changes are never pulled twice.
pub async fn pull_into(
    backend: &dyn ComputeBackend,
    project: &Path,
    patch_dir: &Path,
) -> Result<PullReport> {
    let patch = diff(backend).await?;
    if patch.trim().is_empty() {
        return Ok(PullReport::default());
    }
    let stat = diff_stat(backend).await.unwrap_or_default();
    std::fs::create_dir_all(patch_dir)?;
    let name = project
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "project".into());
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    let patch_path = patch_dir.join(format!("{name}-{ts}.patch"));
    std::fs::write(&patch_path, &patch)?;

    let mut report = PullReport {
        stat,
        patch_path: Some(patch_path),
        ..Default::default()
    };
    let changes = run_checked(
        backend,
        "listing changes",
        &format!("git diff --cached --no-renames --name-status -z {BASELINE_TAG}"),
    )
    .await?;
    let mut fields = changes.split('\0').filter(|f| !f.is_empty());
    let mut failures = Vec::new();
    while let (Some(status), Some(path)) = (fields.next(), fields.next()) {
        let status = status.chars().next().unwrap_or('M');
        match merge_one(backend, project, path, status).await {
            Ok(Merge::Clean) => {}
            Ok(Merge::Conflict) => report.conflicts.push(path.to_owned()),
            Err(e) => failures.push(format!("{path}: {e}")),
        }
    }
    if !failures.is_empty() {
        report.failed = Some(failures.join("; "));
        return Ok(report);
    }
    run_checked(
        backend,
        "recording the sync",
        &format!("set -e\n{REBASELINE}"),
    )
    .await?;
    Ok(report)
}

enum Merge {
    Clean,
    Conflict,
}

/// Where baseline versions are staged inside the sandbox (untracked).
const BASE_STAGING: &str = ".git/mira-base";

async fn baseline_version(backend: &dyn ComputeBackend, path: &str) -> Result<Vec<u8>> {
    let quoted = format!(
        "'{}'",
        format!("{BASELINE_TAG}:{path}").replace('\'', r"'\''")
    );
    run_checked(
        backend,
        "reading the baseline",
        &format!("mkdir -p {BASE_STAGING} && git show {quoted} > {BASE_STAGING}/file"),
    )
    .await?;
    backend.read_file(&format!("{BASE_STAGING}/file")).await
}

async fn merge_one(
    backend: &dyn ComputeBackend,
    project: &Path,
    path: &str,
    status: char,
) -> Result<Merge> {
    let local_path = project.join(path);
    let ours = match std::fs::read(&local_path) {
        Ok(b) => Some(b),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    let base = if status == 'A' {
        None
    } else {
        Some(baseline_version(backend, path).await?)
    };
    let theirs = if status == 'D' {
        None
    } else {
        Some(backend.read_file(path).await?)
    };

    // Worktree untouched since upload (or already identical): take theirs.
    if ours == base || ours == theirs {
        match &theirs {
            Some(t) => {
                if let Some(parent) = local_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&local_path, t)?;
            }
            None => {
                if local_path.exists() {
                    std::fs::remove_file(&local_path)?;
                }
            }
        }
        return Ok(Merge::Clean);
    }
    // Both sides changed it. A deletion vs an edit, or binary content,
    // can't be merged line-wise: keep the local version, flag it.
    let (Some(ours), Some(theirs)) = (ours, theirs) else {
        return Ok(Merge::Conflict);
    };
    let base = base.unwrap_or_default();
    if [&ours, &base, &theirs].iter().any(|b| b.contains(&0)) {
        return Ok(Merge::Conflict);
    }
    let dir = tempfile_dir()?;
    let (o, b, t) = (dir.join("ours"), dir.join("base"), dir.join("theirs"));
    std::fs::write(&o, &ours)?;
    std::fs::write(&b, &base)?;
    std::fs::write(&t, &theirs)?;
    let out = tokio::process::Command::new("git")
        .args([
            "merge-file",
            "-p",
            "-L",
            "local",
            "-L",
            "base",
            "-L",
            "remote",
        ])
        .args([&o, &b, &t])
        .output()
        .await;
    let _ = std::fs::remove_dir_all(&dir);
    let out = out?;
    // Exit status: 0 = clean, >0 = number of conflicts, <0 = error.
    match out.status.code() {
        Some(code) if code >= 0 => {
            std::fs::write(&local_path, &out.stdout)?;
            Ok(if code == 0 {
                Merge::Clean
            } else {
                Merge::Conflict
            })
        }
        _ => Err(ComputeError::Remote(format!(
            "git merge-file failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))),
    }
}

fn tempfile_dir() -> Result<PathBuf> {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let dir = std::env::temp_dir().join(format!("mira-merge-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Every change in the workspace since the upload, as a patch that
/// `git apply` accepts (binary files included).
pub async fn diff(backend: &dyn ComputeBackend) -> Result<String> {
    run_checked(
        backend,
        "collecting changes",
        &format!("git add -A && git diff --cached --binary {BASELINE_TAG}"),
    )
    .await
}

/// Summary line per changed file (`git diff --stat`) for the same range.
pub async fn diff_stat(backend: &dyn ComputeBackend) -> Result<String> {
    run_checked(
        backend,
        "collecting changes",
        &format!("git add -A && git diff --cached --stat {BASELINE_TAG}"),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_honors_gitignore_and_keeps_symlinks_as_links() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let git = |args: &[&str]| {
            assert!(std::process::Command::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
                .output()
                .unwrap()
                .status
                .success());
        };
        git(&["init", "-q"]);
        std::fs::write(root.join(".gitignore"), "target/\n.env\n").unwrap();
        std::fs::write(root.join("main.rs"), "fn main() {}").unwrap();
        std::fs::write(root.join(".env"), "SECRET=1").unwrap();
        std::fs::create_dir(root.join("target")).unwrap();
        std::fs::write(root.join("target/big.bin"), "x").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("/etc/passwd", root.join("link")).unwrap();

        let bytes = pack(root).unwrap();
        let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(&bytes[..]));
        let mut names = Vec::new();
        for e in ar.entries().unwrap() {
            let e = e.unwrap();
            let name = e.path().unwrap().to_string_lossy().into_owned();
            if name == "link" {
                assert!(e.header().entry_type().is_symlink());
            }
            names.push(name);
        }
        names.sort();
        #[cfg(unix)]
        assert_eq!(names, [".gitignore", "link", "main.rs"]);
    }
}
