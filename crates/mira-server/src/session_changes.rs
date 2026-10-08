//! Which files a chat changed, whoever made the change.
//!
//! Mira's own write/edit tools record what they touch, but that misses most
//! of a real session: an external agent (Claude Code, Codex) edits files in
//! its own process, and a `bash` command (`sed -i`, a formatter, codegen, a
//! `git mv`) changes files no tool recorded. So each chat takes a
//! *baseline* — a snapshot of the working tree, as a git tree object —
//! before its first prompt runs, and its changes are the files that differ
//! from that baseline, plus anything the tools recorded writing.
//!
//! The snapshot is built with a throwaway index (`GIT_INDEX_FILE`), so the
//! user's own index and staging are never touched, and it respects
//! `.gitignore`. Its id lives at `<git-dir>/mira/baselines/<session>`.
//! Nothing references the tree, so `git gc` may prune it after a couple of
//! weeks; a chat whose baseline is gone falls back to what its tools
//! recorded.
//!
//! Every path here is relative to the session's cwd, which need not be the
//! repository root — git is always asked for cwd-relative paths
//! (`--relative`, `ls-files`) rather than the root-relative ones
//! `git status --porcelain` prints.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

/// git's well-known empty tree, for a repository with no commits yet.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// One changed file, as a commit of it would carry it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ChangedFile {
    /// Relative to the session's cwd.
    pub path: String,
    pub status: FileStatus,
    pub added: u64,
    pub removed: u64,
    /// Binary content: no line counts.
    pub binary: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
}

pub(crate) fn git(cwd: &Path) -> Command {
    let mut c = Command::new("git");
    c.current_dir(cwd)
        // Never block on a pager or an editor, never colour.
        .env("GIT_PAGER", "cat")
        .env("GIT_EDITOR", "true")
        .args(["-c", "color.ui=never", "-c", "core.quotepath=off"]);
    c
}

pub(crate) fn run(cmd: &mut Command) -> Option<String> {
    let out = cmd.output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

pub(crate) fn git_path(cwd: &Path, what: &str) -> Option<PathBuf> {
    let p = run(git(cwd).args(["rev-parse", "--git-path", what]))?;
    let p = PathBuf::from(p.trim());
    Some(if p.is_absolute() { p } else { cwd.join(p) })
}

/// Whether `cwd` is inside a git work tree.
pub fn in_repo(cwd: &Path) -> bool {
    run(git(cwd).args(["rev-parse", "--is-inside-work-tree"])).is_some_and(|s| s.trim() == "true")
}

/// `HEAD`, or the empty tree when the repository has no commits.
fn head(cwd: &Path) -> String {
    run(git(cwd).args(["rev-parse", "--verify", "-q", "HEAD^{tree}"]))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| EMPTY_TREE.to_string())
}

/// The working tree under `cwd` as a tree object, without touching the
/// user's index. Seeded from a copy of the real index so unchanged files
/// aren't re-hashed.
pub fn snapshot_tree(cwd: &Path) -> Option<String> {
    let index = git_path(cwd, "index")?;
    // Unique per call: the diff and review endpoints often snapshot at once.
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = git_path(
        cwd,
        &format!("mira-snapshot-{}-{n}.index", std::process::id()),
    )?;
    if index.exists() {
        std::fs::copy(&index, &tmp).ok()?;
    }
    let tree = run(git(cwd)
        .env("GIT_INDEX_FILE", &tmp)
        .args(["add", "-A", "--", "."]))
    .and_then(|_| run(git(cwd).env("GIT_INDEX_FILE", &tmp).arg("write-tree")))
    .map(|s| s.trim().to_string());
    let _ = std::fs::remove_file(&tmp);
    tree.filter(|t| !t.is_empty())
}

fn baseline_file(cwd: &Path, session: &str) -> Option<PathBuf> {
    // Session ids are `sess_<hex>`; anything else is refused rather than joined
    // into a path.
    if session.is_empty()
        || !session
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }
    git_path(cwd, &format!("mira/baselines/{session}"))
}

/// Record the chat's baseline, once. Later calls keep the first one — the
/// point is "since this chat began".
pub fn ensure_baseline(cwd: &Path, session: &str) {
    let Some(file) = baseline_file(cwd, session) else {
        return;
    };
    if file.exists() || !in_repo(cwd) {
        return;
    }
    let Some(tree) = snapshot_tree(cwd) else {
        return;
    };
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(&file, &tree) {
        tracing::warn!(%e, "session changes: could not record the baseline");
    }
}

/// The recorded baseline, if it's still in the object store.
fn baseline(cwd: &Path, session: &str) -> Option<String> {
    let tree = std::fs::read_to_string(baseline_file(cwd, session)?).ok()?;
    let tree = tree.trim();
    run(git(cwd).args(["cat-file", "-e", &format!("{tree}^{{tree}}")]))?;
    Some(tree.to_string())
}

fn split_z(s: &str) -> impl Iterator<Item = &str> {
    s.split('\0').filter(|p| !p.is_empty())
}

/// Paths under `cwd` that differ between two trees.
pub(crate) fn tree_diff_paths(cwd: &Path, from: &str, to: &str) -> Vec<String> {
    run(git(cwd).args([
        "diff",
        "--name-only",
        "-z",
        "--no-renames",
        "--relative",
        from,
        to,
    ]))
    .map(|out| split_z(&out).map(str::to_string).collect())
    .unwrap_or_default()
}

/// Everything the chat changed that a commit would still carry, with line
/// counts against HEAD. `written` is what Mira's own tools recorded writing
/// (absolute paths); paths outside `cwd` are left out.
pub fn changed_files(cwd: &Path, session: &str, written: &[PathBuf]) -> Vec<ChangedFile> {
    if !in_repo(cwd) {
        return Vec::new();
    }
    let root = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let mut candidates: BTreeSet<String> = written
        .iter()
        .filter_map(|p| {
            p.strip_prefix(cwd)
                .or_else(|_| p.strip_prefix(&root))
                .ok()
                .map(|r| r.to_string_lossy().into_owned())
        })
        .filter(|r| !r.is_empty())
        .collect();
    if let Some(base) = baseline(cwd, session) {
        if let Some(now) = snapshot_tree(cwd) {
            candidates.extend(tree_diff_paths(cwd, &base, &now));
        }
    }
    if candidates.is_empty() {
        return Vec::new();
    }
    stats_against_head(cwd, &candidates.into_iter().collect::<Vec<_>>())
}

/// Line counts and status, against HEAD, for those of `paths` that differ
/// from it. Tracked files come from `git diff HEAD`; new files from
/// `ls-files --others` (ignored files excluded), counted as all-added.
pub fn stats_against_head(cwd: &Path, paths: &[String]) -> Vec<ChangedFile> {
    let head = head(cwd);
    let mut out = Vec::new();
    for chunk in paths.chunks(500) {
        let mut tracked = git(cwd);
        tracked.args([
            "diff",
            "--numstat",
            "-z",
            "--no-renames",
            "--relative",
            &head,
            "--",
        ]);
        tracked.args(chunk);
        if let Some(s) = run(&mut tracked) {
            out.extend(parse_numstat_z(&s));
        }
        // Which of them are gone from the work tree (a deletion's numstat
        // looks like any other change).
        let mut untracked = git(cwd);
        untracked.args(["ls-files", "--others", "--exclude-standard", "-z", "--"]);
        untracked.args(chunk);
        if let Some(s) = run(&mut untracked) {
            for path in split_z(&s) {
                let (added, binary) = match std::fs::read(cwd.join(path)) {
                    Ok(b) if b.contains(&0) => (0, true),
                    Ok(b) => (String::from_utf8_lossy(&b).lines().count() as u64, false),
                    Err(_) => (0, false),
                };
                out.push(ChangedFile {
                    path: path.to_string(),
                    status: FileStatus::Added,
                    added,
                    removed: 0,
                    binary,
                });
            }
        }
    }
    for f in &mut out {
        if f.status == FileStatus::Modified && !cwd.join(&f.path).exists() {
            f.status = FileStatus::Deleted;
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out.dedup_by(|a, b| a.path == b.path);
    out
}

/// `git diff --numstat -z` without renames: `added\tremoved\tpath\0`, with
/// `-` counts for binary files.
pub(crate) fn parse_numstat_z(s: &str) -> Vec<ChangedFile> {
    split_z(s)
        .filter_map(|rec| {
            let mut it = rec.splitn(3, '\t');
            let (a, r, path) = (it.next()?, it.next()?, it.next()?);
            let binary = a == "-" || r == "-";
            Some(ChangedFile {
                path: path.trim_start_matches('\n').to_string(),
                status: FileStatus::Modified,
                added: a.parse().unwrap_or(0),
                removed: r.parse().unwrap_or(0),
                binary,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .current_dir(dir)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?}");
    }

    fn repo() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        sh(d.path(), &["init", "-q", "--template="]);
        std::fs::write(d.path().join("a.txt"), "one\ntwo\n").unwrap();
        std::fs::write(d.path().join("gone.txt"), "x\n").unwrap();
        std::fs::write(d.path().join(".gitignore"), "target/\n").unwrap();
        std::fs::create_dir(d.path().join("sub")).unwrap();
        std::fs::write(d.path().join("sub/mod.rs"), "fn a() {}\n").unwrap();
        sh(d.path(), &["add", "-A"]);
        sh(d.path(), &["commit", "-qm", "init"]);
        d
    }

    /// The shape real session ids have (`SessionId`).
    const S: &str = "sess_cbe42bdbc3d442c9814188145e99af38";

    #[test]
    fn changes_nobody_recorded_are_found() {
        let d = repo();
        let p = d.path();
        // Dirty before the chat: not the chat's doing.
        std::fs::write(p.join("sub/mod.rs"), "fn a() {}\nfn before() {}\n").unwrap();
        ensure_baseline(p, S);
        // An agent edits, a shell command deletes, a new file appears, and
        // a build writes ignored output — none of it through Mira's tools.
        std::fs::write(p.join("a.txt"), "one\nTWO\nthree\n").unwrap();
        std::fs::remove_file(p.join("gone.txt")).unwrap();
        std::fs::write(p.join("new.rs"), "a\nb\nc\n").unwrap();
        std::fs::create_dir(p.join("target")).unwrap();
        std::fs::write(p.join("target/out"), "bin").unwrap();

        let files = changed_files(p, S, &[]);
        let got: Vec<_> = files
            .iter()
            .map(|f| (f.path.as_str(), f.status, f.added, f.removed))
            .collect();
        assert_eq!(
            got,
            vec![
                ("a.txt", FileStatus::Modified, 2, 1),
                ("gone.txt", FileStatus::Deleted, 0, 1),
                ("new.rs", FileStatus::Added, 3, 0),
            ]
        );
    }

    #[test]
    fn recorded_writes_count_even_without_a_baseline() {
        let d = repo();
        let p = d.path();
        std::fs::write(p.join("sub/mod.rs"), "fn b() {}\n").unwrap();
        let files = changed_files(
            p,
            S,
            &[p.join("sub/mod.rs"), PathBuf::from("/elsewhere/x.rs")],
        );
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "sub/mod.rs");
    }

    #[test]
    fn paths_are_relative_to_a_subfolder_cwd() {
        let d = repo();
        let sub = d.path().join("sub");
        ensure_baseline(&sub, S);
        std::fs::write(sub.join("mod.rs"), "fn c() {}\n").unwrap();
        std::fs::write(d.path().join("a.txt"), "outside the cwd\n").unwrap();
        let files = changed_files(&sub, S, &[]);
        assert_eq!(
            files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
            vec!["mod.rs"]
        );
    }

    #[test]
    fn a_change_that_was_undone_drops_out() {
        let d = repo();
        let p = d.path();
        ensure_baseline(p, S);
        std::fs::write(p.join("a.txt"), "changed\n").unwrap();
        assert_eq!(changed_files(p, S, &[]).len(), 1);
        std::fs::write(p.join("a.txt"), "one\ntwo\n").unwrap();
        assert!(changed_files(p, S, &[p.join("a.txt")]).is_empty());
    }

    #[test]
    fn the_users_index_is_left_alone() {
        let d = repo();
        let p = d.path();
        std::fs::write(p.join("a.txt"), "staged\n").unwrap();
        sh(p, &["add", "a.txt"]);
        std::fs::write(p.join("loose.txt"), "untracked\n").unwrap();
        let before = run(git(p).args(["diff", "--cached", "--name-only"])).unwrap();
        ensure_baseline(p, S);
        assert_eq!(
            run(git(p).args(["diff", "--cached", "--name-only"])).unwrap(),
            before
        );
        assert!(
            run(git(p).args(["ls-files", "--others", "--exclude-standard"]))
                .unwrap()
                .contains("loose.txt")
        );
    }

    #[test]
    fn a_repo_without_commits_still_works() {
        let d = tempfile::tempdir().unwrap();
        sh(d.path(), &["init", "-q", "--template="]);
        ensure_baseline(d.path(), S);
        std::fs::write(d.path().join("first.rs"), "x\n").unwrap();
        let files = changed_files(d.path(), S, &[]);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].status, FileStatus::Added);
    }

    #[test]
    fn odd_session_ids_are_refused() {
        let d = repo();
        assert!(baseline_file(d.path(), "../../etc").is_none());
    }
}
