//! Getting the project into a sandbox and the result back out.
//!
//! 1. [`pack`] the local project into a `.tar.gz`: git-tracked plus
//!    untracked-but-not-ignored files, so build output, `node_modules`
//!    and secrets in `.gitignore` never leave the machine.
//! 2. [`upload`] extracts it into the backend's workspace and commits it
//!    as a `mira-baseline` tag in a fresh git repo there.
//! 3. The session works in the sandbox, which is the source of truth
//!    while it runs.
//! 4. [`diff`] returns everything that changed since the baseline as a
//!    binary-safe patch, for the user to review and `git apply` locally.

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
pub async fn upload(backend: &dyn ComputeBackend, archive: &[u8]) -> Result<()> {
    backend.write_file(ARCHIVE_NAME, archive).await?;
    let script = format!(
        "set -e\n\
         tar -xzf {ARCHIVE_NAME}\n\
         rm -f {ARCHIVE_NAME}\n\
         git init -q\n\
         git add -A\n\
         git -c user.name=mira -c user.email=mira@localhost -c commit.gpgsign=false \
           commit -q --allow-empty --no-verify -m 'mira baseline'\n\
         git tag -f {BASELINE_TAG} >/dev/null\n"
    );
    run_checked(backend, "workspace setup", &script).await?;
    Ok(())
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
