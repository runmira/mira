//! Ephemeral git-worktree isolation for write-capable subagents.
//!
//! When an agent type declares `worktree: true`, `AgentTool::invoke`
//! spins up a `WorktreeSession` before building the child's `ToolContext`
//! so the child edits land in a linked worktree instead of the parent's
//! primary tree. On completion the caller invokes `merge_and_cleanup`,
//! which diffs the worktree against its base commit, copies modified /
//! added / deleted files back to the parent, then `git worktree remove`s
//! the directory and deletes the ephemeral branch.
//!
//! The `Drop` impl runs the same teardown as a safety net for error
//! paths (session panics, early returns) so we don't leak worktrees on
//! disk. `merge_and_cleanup` sets `disarmed = true` before returning so
//! the drop no-ops in the happy path.
//!
//! Untracked files are captured via `git ls-files --others
//! --exclude-standard`; renames get flattened to `D old` + `A new` at
//! parse time so `apply_change` never has to deal with the two-path
//! rename form. Files ignored by `.gitignore` are intentionally NOT
//! merged back — a coder that touches, say, `target/` is almost always
//! a mistake and the caller wants those to disappear with the worktree.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use tracing::warn;

/// One ephemeral worktree, keyed to a single subagent spawn.
pub(crate) struct WorktreeSession {
    /// Absolute path to the linked worktree directory. Also serves as
    /// the child session's `cwd`.
    path: PathBuf,
    /// The ephemeral branch name (`mira/agent/<slug>`) — deleted on cleanup.
    branch: String,
    /// The parent's cwd. Changed files get copied here on merge.
    parent_cwd: PathBuf,
    /// Commit sha the worktree was branched from. Used as the diff base
    /// so we pick up both committed and working-tree changes.
    base_sha: String,
    /// Set by `merge_and_cleanup` so `Drop` doesn't re-run teardown.
    disarmed: bool,
}

impl WorktreeSession {
    /// Try to create an isolated worktree for a subagent. Returns
    /// `Ok(None)` when the parent's cwd isn't inside a git repo — the
    /// caller should fall back to running in the parent's cwd (with a
    /// warning event) rather than erroring the whole spawn.
    pub(crate) fn try_create(
        parent_cwd: &Path,
        type_name: &str,
        call_id: &str,
    ) -> Result<Option<Self>> {
        if !is_git_repo(parent_cwd) {
            return Ok(None);
        }
        let primary = primary_worktree(parent_cwd)
            .ok_or_else(|| anyhow!("could not locate primary worktree"))?;
        let base_sha = current_head_sha(parent_cwd)
            .ok_or_else(|| anyhow!("could not resolve HEAD sha"))?;

        // Slug pieces: type name for grep-ability, first 8 chars of the
        // call id for traceability across the child transcript, and a
        // millis timestamp so back-to-back spawns of the same call id
        // (retries) don't collide.
        let short_id: String = call_id.chars().take(8).collect();
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let slug = sanitize_slug(&format!("agent-{type_name}-{short_id}-{ts}"));
        let branch = format!("mira/agent/{slug}");
        let path = primary.join(".mira").join("worktrees").join(&slug);

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("mkdir {}", parent.display()))?;
        }

        // `git worktree add -b <branch> <path> <base>` — creates the
        // ephemeral branch off the base commit and checks it out at path.
        let out = Command::new("git")
            .current_dir(parent_cwd)
            .args(["worktree", "add", "-b", &branch])
            .arg(&path)
            .arg(&base_sha)
            .output()
            .context("spawn `git worktree add`")?;
        if !out.status.success() {
            return Err(anyhow!(
                "git worktree add failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }

        Ok(Some(Self {
            path,
            branch,
            parent_cwd: parent_cwd.to_path_buf(),
            base_sha,
            disarmed: false,
        }))
    }

    /// The linked worktree's absolute path — feed this into the child
    /// session's `ToolContext` so its edits land here rather than in the
    /// parent tree.
    pub(crate) fn cwd(&self) -> &Path {
        &self.path
    }

    /// Diff the worktree against its base commit, apply the file
    /// changes back to `parent_cwd`, tear the worktree + branch down.
    /// Consumes `self`; disarms the drop guard so cleanup isn't retried.
    pub(crate) fn merge_and_cleanup(mut self) -> MergeReport {
        let report = self.harvest();
        self.teardown();
        self.disarmed = true;
        report
    }

    fn harvest(&self) -> MergeReport {
        let mut report = MergeReport::default();

        // 1. Modified / deleted / renamed vs the base commit. `git diff
        //    <base>` compares base ↔ working tree, so this covers both
        //    committed and uncommitted changes in one shot.
        match git_diff_name_status(&self.path, &self.base_sha) {
            Ok(entries) => {
                for (status, rel) in entries {
                    self.apply_change(&status, &rel, &mut report);
                }
            }
            Err(e) => {
                warn!("worktree diff failed: {e}");
                report.errors.push(format!("diff: {e}"));
            }
        }

        // 2. Untracked (freshly created, gitignore-respecting) files.
        //    `git diff` doesn't see these — they aren't part of the index.
        match git_ls_untracked(&self.path) {
            Ok(files) => {
                for rel in files {
                    self.copy_from_worktree(&rel, &mut report);
                }
            }
            Err(e) => {
                warn!("worktree ls-files failed: {e}");
                report.errors.push(format!("ls-files: {e}"));
            }
        }

        report
    }

    fn apply_change(&self, status: &str, rel_path: &Path, report: &mut MergeReport) {
        // Status codes from `git diff --name-status`:
        //   A = added, M = modified, T = type change, D = deleted.
        //   R/C were flattened to D+A by the parser so we never see them here.
        match status.chars().next().unwrap_or(' ') {
            'A' | 'M' | 'T' => self.copy_from_worktree(rel_path, report),
            'D' => {
                let dst = self.parent_cwd.join(rel_path);
                if dst.exists() {
                    if let Err(e) = std::fs::remove_file(&dst) {
                        warn!("delete {}: {}", dst.display(), e);
                        report.errors.push(format!(
                            "delete {}: {e}",
                            rel_path.display()
                        ));
                    } else {
                        report.files_deleted.push(rel_path.to_path_buf());
                    }
                }
            }
            other => {
                report.errors.push(format!(
                    "unknown diff status `{other}` for {}",
                    rel_path.display()
                ));
            }
        }
    }

    fn copy_from_worktree(&self, rel_path: &Path, report: &mut MergeReport) {
        let src = self.path.join(rel_path);
        let dst = self.parent_cwd.join(rel_path);
        if let Err(e) = copy_file(&src, &dst) {
            warn!("copy {} → {}: {}", src.display(), dst.display(), e);
            report
                .errors
                .push(format!("copy {}: {e}", rel_path.display()));
        } else {
            report.files_merged.push(rel_path.to_path_buf());
        }
    }

    fn teardown(&self) {
        // `git worktree remove --force` — force flag because the child
        // may have left uncommitted changes we already harvested.
        let out = Command::new("git")
            .current_dir(&self.parent_cwd)
            .args(["worktree", "remove", "--force"])
            .arg(&self.path)
            .output();
        match out {
            Ok(o) if !o.status.success() => {
                warn!(
                    worktree = %self.path.display(),
                    "git worktree remove: {}",
                    String::from_utf8_lossy(&o.stderr).trim()
                );
            }
            Err(e) => warn!("git worktree remove spawn failed: {e}"),
            _ => {}
        }
        // Best-effort branch cleanup. `-D` because the branch may have
        // commits that were never merged into anything.
        let _ = Command::new("git")
            .current_dir(&self.parent_cwd)
            .args(["branch", "-D", &self.branch])
            .output();
    }
}

impl Drop for WorktreeSession {
    fn drop(&mut self) {
        if self.disarmed {
            return;
        }
        // Error path: caller returned/panicked before calling
        // merge_and_cleanup. Skip harvest (state may be inconsistent)
        // and just make sure the worktree directory + branch don't leak.
        self.teardown();
    }
}

/// What happened during merge. `is_empty()` is true when the child made
/// no filesystem changes, so callers can suppress a "merged 0 files"
/// note in that case.
#[derive(Debug, Default)]
pub(crate) struct MergeReport {
    pub files_merged: Vec<PathBuf>,
    pub files_deleted: Vec<PathBuf>,
    pub errors: Vec<String>,
}

impl MergeReport {
    pub fn is_empty(&self) -> bool {
        self.files_merged.is_empty()
            && self.files_deleted.is_empty()
            && self.errors.is_empty()
    }

    /// Compact one-liner for the tool result / parent transcript, e.g.
    /// "merged 3 file(s): a.rs, b.rs, +1 more". Returns `None` when
    /// nothing happened.
    pub fn short_summary(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        let mut parts = Vec::new();
        if !self.files_merged.is_empty() {
            parts.push(format_file_list("merged", &self.files_merged));
        }
        if !self.files_deleted.is_empty() {
            parts.push(format_file_list("deleted", &self.files_deleted));
        }
        if !self.errors.is_empty() {
            parts.push(format!(
                "{} error(s): {}",
                self.errors.len(),
                self.errors.join("; ")
            ));
        }
        Some(parts.join("; "))
    }
}

fn format_file_list(verb: &str, files: &[PathBuf]) -> String {
    let n = files.len();
    let shown: Vec<String> = files
        .iter()
        .take(5)
        .map(|p| p.display().to_string())
        .collect();
    let suffix = if n > 5 {
        format!(", +{} more", n - 5)
    } else {
        String::new()
    };
    format!("{verb} {n} file(s): {}{suffix}", shown.join(", "))
}

/* ---------- git shell-outs ---------- */

fn is_git_repo(cwd: &Path) -> bool {
    Command::new("git")
        .current_dir(cwd)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map(|o| {
            o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == "true"
        })
        .unwrap_or(false)
}

fn primary_worktree(cwd: &Path) -> Option<PathBuf> {
    let out = Command::new("git")
        .current_dir(cwd)
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let common = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim().to_owned());
    // `--git-common-dir` returns `<primary>/.git` (or the bare dir); the
    // primary worktree is its parent.
    common.parent().map(|p| p.to_path_buf())
}

fn current_head_sha(cwd: &Path) -> Option<String> {
    let out = Command::new("git")
        .current_dir(cwd)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// Parse `git diff --name-status -z <base>` into `(status, path)` pairs.
/// Renames (`R100\told\tnew`) are flattened to a delete of `old` plus an
/// add of `new` so downstream code doesn't need special-casing.
fn git_diff_name_status(cwd: &Path, base: &str) -> Result<Vec<(String, PathBuf)>> {
    let out = Command::new("git")
        .current_dir(cwd)
        .args(["diff", "--name-status", "-z", base])
        .output()
        .context("git diff")?;
    if !out.status.success() {
        return Err(anyhow!(
            "git diff failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    // `-z` output for A/M/D/T: STATUS\0PATH\0. For R/C:
    // STATUS\0OLD\0NEW\0 (STATUS itself is e.g. "R100").
    let bytes = out.stdout;
    let mut entries = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let s_end = next_nul(&bytes, i)
            .ok_or_else(|| anyhow!("malformed diff-z: missing status terminator"))?;
        let status = std::str::from_utf8(&bytes[i..s_end])
            .context("diff status not utf-8")?
            .to_string();
        i = s_end + 1;

        let is_rename_or_copy = status.starts_with('R') || status.starts_with('C');
        let p1_end = next_nul(&bytes, i)
            .ok_or_else(|| anyhow!("malformed diff-z: missing path terminator"))?;
        let path1 = PathBuf::from(
            std::str::from_utf8(&bytes[i..p1_end]).context("diff path not utf-8")?,
        );
        i = p1_end + 1;

        if is_rename_or_copy {
            let p2_end = next_nul(&bytes, i).ok_or_else(|| {
                anyhow!("malformed diff-z: missing rename target terminator")
            })?;
            let path2 = PathBuf::from(
                std::str::from_utf8(&bytes[i..p2_end])
                    .context("rename target not utf-8")?,
            );
            i = p2_end + 1;
            // Rename = old path goes away, new path appears with old content.
            // Emit both so apply_change treats them uniformly.
            entries.push(("D".to_string(), path1));
            entries.push(("A".to_string(), path2));
        } else {
            entries.push((status, path1));
        }
    }
    Ok(entries)
}

fn git_ls_untracked(cwd: &Path) -> Result<Vec<PathBuf>> {
    let out = Command::new("git")
        .current_dir(cwd)
        .args(["ls-files", "--others", "--exclude-standard", "-z"])
        .output()
        .context("git ls-files")?;
    if !out.status.success() {
        return Err(anyhow!(
            "git ls-files failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let mut files = Vec::new();
    for chunk in out.stdout.split(|&b| b == 0) {
        if chunk.is_empty() {
            continue;
        }
        files.push(PathBuf::from(
            std::str::from_utf8(chunk).context("ls-files path not utf-8")?,
        ));
    }
    Ok(files)
}

fn next_nul(bytes: &[u8], from: usize) -> Option<usize> {
    bytes[from..].iter().position(|&b| b == 0).map(|p| from + p)
}

fn copy_file(src: &Path, dst: &Path) -> std::io::Result<()> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(src, dst)?;
    Ok(())
}

/// Keep the slug safe for filesystem + git ref usage. Git refs disallow
/// `..`, spaces, leading `-`, and a bunch of shell metas — the paths we
/// generate control most of that already but tool names + call ids are
/// external input, so scrub anything that isn't `[A-Za-z0-9._-]`.
fn sanitize_slug(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    if out.starts_with('-') {
        out.insert(0, '_');
    }
    out
}

/* ---------- tests ---------- */

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    struct GitRepo {
        dir: PathBuf,
    }

    impl GitRepo {
        fn init() -> Self {
            let mut path = std::env::temp_dir();
            path.push(format!(
                "mira-worktree-test-{}-{}",
                std::process::id(),
                rand_suffix(),
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();

            run(&path, &["init", "-q", "-b", "main"]);
            run(&path, &["config", "user.email", "t@t.t"]);
            run(&path, &["config", "user.name", "t"]);
            // Seed with an empty commit so HEAD resolves.
            std::fs::write(path.join("README"), b"seed\n").unwrap();
            run(&path, &["add", "."]);
            run(&path, &["commit", "-q", "-m", "seed"]);
            Self { dir: path }
        }
    }

    impl Drop for GitRepo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn run(cwd: &Path, args: &[&str]) {
        let out = Command::new("git")
            .current_dir(cwd)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?} in {}: {}",
            cwd.display(),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn rand_suffix() -> String {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .to_string()
    }

    #[test]
    fn try_create_returns_none_outside_git() {
        let tmp = std::env::temp_dir().join(format!("mira-worktree-nogit-{}", rand_suffix()));
        std::fs::create_dir_all(&tmp).unwrap();
        let res = WorktreeSession::try_create(&tmp, "coder", "call-1").unwrap();
        assert!(res.is_none());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn merge_copies_modified_and_new_files() {
        let repo = GitRepo::init();
        std::fs::write(repo.dir.join("existing.txt"), b"orig\n").unwrap();
        run(&repo.dir, &["add", "."]);
        run(&repo.dir, &["commit", "-q", "-m", "add existing"]);

        let session =
            WorktreeSession::try_create(&repo.dir, "coder", "abcd1234ef").unwrap().unwrap();

        // Child modifies one file + creates a new one.
        std::fs::write(session.cwd().join("existing.txt"), b"edited\n").unwrap();
        let sub = session.cwd().join("nested");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("fresh.txt"), b"new file\n").unwrap();

        let report = session.merge_and_cleanup();
        assert!(
            report.errors.is_empty(),
            "unexpected errors: {:?}",
            report.errors
        );

        // Parent tree should now have both changes.
        assert_eq!(
            std::fs::read_to_string(repo.dir.join("existing.txt")).unwrap(),
            "edited\n"
        );
        assert_eq!(
            std::fs::read_to_string(repo.dir.join("nested").join("fresh.txt")).unwrap(),
            "new file\n"
        );
        // Worktree should be gone.
        assert!(!repo.dir.join(".mira").join("worktrees").exists()
            || std::fs::read_dir(repo.dir.join(".mira").join("worktrees"))
                .map(|d| d.count() == 0)
                .unwrap_or(true));
    }

    #[test]
    fn merge_deletes_removed_files() {
        let repo = GitRepo::init();
        std::fs::write(repo.dir.join("gone.txt"), b"bye\n").unwrap();
        run(&repo.dir, &["add", "."]);
        run(&repo.dir, &["commit", "-q", "-m", "add gone"]);

        let session =
            WorktreeSession::try_create(&repo.dir, "coder", "abcd1234").unwrap().unwrap();
        std::fs::remove_file(session.cwd().join("gone.txt")).unwrap();

        let report = session.merge_and_cleanup();
        assert!(report.errors.is_empty(), "errors: {:?}", report.errors);
        assert!(!repo.dir.join("gone.txt").exists());
        assert!(report
            .files_deleted
            .iter()
            .any(|p| p.to_string_lossy() == "gone.txt"));
    }

    #[test]
    fn drop_cleans_up_when_not_finalized() {
        let repo = GitRepo::init();
        let path;
        {
            let s =
                WorktreeSession::try_create(&repo.dir, "coder", "callid01").unwrap().unwrap();
            path = s.cwd().to_path_buf();
            // Simulate an error path — drop without calling merge_and_cleanup.
        }
        assert!(!path.exists(), "worktree dir should be removed by Drop");
    }

    #[test]
    fn is_empty_reflects_activity() {
        let report = MergeReport::default();
        assert!(report.is_empty());
        let mut r2 = MergeReport::default();
        r2.files_merged.push(PathBuf::from("a"));
        assert!(!r2.is_empty());
    }

    // Suppress unused warning on the import.
    #[allow(dead_code)]
    fn _writer_marker() -> impl Write {
        std::io::stderr()
    }
}
