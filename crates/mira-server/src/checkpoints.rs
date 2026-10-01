//! Per-message checkpoints: put the files back the way they were before a
//! message ran.
//!
//! Before every prompt, the working tree is snapshotted (the same
//! throwaway-index snapshot `session_changes` uses, so the user's index is
//! never touched) and filed under the message that's about to run.
//! Restoring a message's checkpoint returns every file that has changed
//! since — by Mira, an external agent, a shell command or the user — to its
//! state at that moment: modified files get their old content back, files
//! created since are removed, files deleted since come back. Ignored files
//! are never touched. A snapshot of the current state is taken first, so a
//! restore can itself be undone.
//!
//! Messages are identified the way editing one already is: by text and
//! occurrence, counted from the latest (see `Session::rewind_to_user`).
//! An index would shift when a long chat is compacted, and restoring the
//! wrong point is the one mistake this must never make.
//!
//! Stored at `<git-dir>/mira/checkpoints/<session>.json`. The trees are
//! unreferenced objects, so `git gc` may prune them after a couple of weeks;
//! a checkpoint whose tree is gone is reported as unavailable.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::session_changes::{git, git_path, run, snapshot_tree, tree_diff_paths};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Store {
    /// One per prompt, oldest first.
    checkpoints: Vec<Checkpoint>,
    /// States saved just before a restore, so the restore can be undone.
    #[serde(default)]
    undo: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Checkpoint {
    /// The message, as the user sent it.
    text: String,
    tree: String,
    /// Unix seconds.
    at: u64,
}

/// What restoring would do to one file.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RestoreChange {
    /// Relative to the session's cwd.
    pub path: String,
    pub action: RestoreAction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RestoreAction {
    /// Content goes back to what it was.
    Revert,
    /// Created since; removed.
    Remove,
    /// Deleted since; brought back.
    Recreate,
}

fn store_file(cwd: &Path, session: &str) -> Option<PathBuf> {
    if session.is_empty()
        || !session
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }
    git_path(cwd, &format!("mira/checkpoints/{session}.json"))
}

fn load(file: &Path) -> Store {
    std::fs::read(file)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save(file: &Path, store: &Store) {
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = file.with_extension("json.tmp");
    let ok = std::fs::write(&tmp, serde_json::to_vec(store).unwrap_or_default())
        .and_then(|_| std::fs::rename(&tmp, file));
    if let Err(e) = ok {
        tracing::warn!(%e, "checkpoints: could not save");
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Index of a message's checkpoint: the `occurrence`-th match of `text`,
/// counted from the latest.
fn find(store: &Store, text: &str, occurrence: usize) -> Option<usize> {
    store
        .checkpoints
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, c)| c.text == text)
        .nth(occurrence)
        .map(|(i, _)| i)
}

/// Snapshot the files before `text` runs. `replaces` is the message being
/// edited, for a resend: its checkpoint and every later one go, as those
/// messages leave the transcript.
pub fn record(cwd: &Path, session: &str, text: &str, replaces: Option<(&str, usize)>) {
    let Some(file) = store_file(cwd, session) else {
        return;
    };
    let Some(tree) = snapshot_tree(cwd) else {
        return;
    };
    let mut store = load(&file);
    if let Some((original, occurrence)) = replaces {
        if let Some(i) = find(&store, original, occurrence) {
            store.checkpoints.truncate(i);
        }
    }
    store.checkpoints.push(Checkpoint {
        text: text.to_string(),
        tree,
        at: now_secs(),
    });
    save(&file, &store);
}

fn tree_exists(cwd: &Path, tree: &str) -> bool {
    run(git(cwd).args(["cat-file", "-e", &format!("{tree}^{{tree}}")])).is_some()
}

/// The checkpoint tree for a message, if it's still there.
fn checkpoint_tree(
    cwd: &Path,
    session: &str,
    text: &str,
    occurrence: usize,
) -> Result<String, String> {
    let file = store_file(cwd, session).ok_or("no checkpoints for this chat")?;
    let store = load(&file);
    let i = find(&store, text, occurrence)
        .ok_or("no checkpoint for this message — it was sent before checkpoints were kept")?;
    let tree = store.checkpoints[i].tree.clone();
    if !tree_exists(cwd, &tree) {
        return Err("this checkpoint has expired (git cleaned up its snapshot)".into());
    }
    Ok(tree)
}

/// Does `tree` have `path` (relative to cwd)?
fn in_tree(cwd: &Path, tree: &str, path: &str) -> bool {
    run(git(cwd).args(["cat-file", "-e", &format!("{tree}:./{path}")])).is_some()
}

/// What restoring `tree` would change.
fn plan(cwd: &Path, tree: &str) -> Result<Vec<RestoreChange>, String> {
    let now = snapshot_tree(cwd).ok_or("could not read the working tree")?;
    let mut out: Vec<RestoreChange> = tree_diff_paths(cwd, tree, &now)
        .into_iter()
        .map(|path| {
            let action = match (in_tree(cwd, tree, &path), cwd.join(&path).exists()) {
                (true, true) => RestoreAction::Revert,
                (true, false) => RestoreAction::Recreate,
                (false, _) => RestoreAction::Remove,
            };
            RestoreChange { path, action }
        })
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

/// What restoring the checkpoint before a message would change.
pub fn preview(
    cwd: &Path,
    session: &str,
    text: &str,
    occurrence: usize,
) -> Result<Vec<RestoreChange>, String> {
    let tree = checkpoint_tree(cwd, session, text, occurrence)?;
    plan(cwd, &tree)
}

/// The result of a restore.
#[derive(Debug, Serialize)]
pub struct Restored {
    pub changes: Vec<RestoreChange>,
    /// Pass back to [`undo`] to put things as they were before the restore.
    pub undo: String,
}

/// Restore the files to the checkpoint before a message.
pub fn restore(
    cwd: &Path,
    session: &str,
    text: &str,
    occurrence: usize,
) -> Result<Restored, String> {
    let tree = checkpoint_tree(cwd, session, text, occurrence)?;
    restore_tree(cwd, session, &tree)
}

/// Undo a restore, from the id it returned.
pub fn undo(cwd: &Path, session: &str, undo_id: &str) -> Result<Restored, String> {
    let file = store_file(cwd, session).ok_or("no checkpoints for this chat")?;
    // Only states this chat saved — never an arbitrary object.
    if !load(&file).undo.iter().any(|t| t == undo_id) {
        return Err("nothing to undo".into());
    }
    if !tree_exists(cwd, undo_id) {
        return Err("that undo point has expired".into());
    }
    restore_tree(cwd, session, undo_id)
}

fn restore_tree(cwd: &Path, session: &str, tree: &str) -> Result<Restored, String> {
    let file = store_file(cwd, session).ok_or("no checkpoints for this chat")?;
    let undo_point = snapshot_tree(cwd).ok_or("could not save the current state first")?;
    let changes = plan(cwd, tree)?;
    // Save the way back before touching anything.
    let mut store = load(&file);
    store.undo.push(undo_point.clone());
    let excess = store.undo.len().saturating_sub(20);
    store.undo.drain(..excess);
    save(&file, &store);

    let mut failed = Vec::new();
    for c in &changes {
        if let Err(e) = apply(cwd, tree, c) {
            failed.push(format!("{}: {e}", c.path));
        }
    }
    if !failed.is_empty() {
        return Err(format!(
            "restored all but {} file(s): {}",
            failed.len(),
            failed.join("; ")
        ));
    }
    Ok(Restored {
        changes,
        undo: undo_point,
    })
}

/// Put one file back as it is in `tree`, without touching the index.
fn apply(cwd: &Path, tree: &str, c: &RestoreChange) -> Result<(), String> {
    let dest = cwd.join(&c.path);
    if c.action == RestoreAction::Remove {
        return std::fs::remove_file(&dest).or_else(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                Ok(())
            } else {
                Err(e.to_string())
            }
        });
    }
    let spec = format!("{tree}:./{}", c.path);
    let entry = run(git(cwd).args(["ls-tree", tree, "--", &c.path])).unwrap_or_default();
    let mode = entry.split_whitespace().next().unwrap_or("100644");
    if mode == "160000" {
        return Ok(()); // a submodule pointer; not ours to move
    }
    let out = git(cwd)
        .args(["cat-file", "blob", &spec])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    // Replace rather than write through: the path may be a symlink now.
    let _ = std::fs::remove_file(&dest);
    #[cfg(unix)]
    if mode == "120000" {
        let target = String::from_utf8_lossy(&out.stdout).into_owned();
        return std::os::unix::fs::symlink(target, &dest).map_err(|e| e.to_string());
    }
    std::fs::write(&dest, &out.stdout).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let bits = if mode == "100755" { 0o755 } else { 0o644 };
        let _ = std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(bits));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// The shape real session ids have (`SessionId`).
    const S: &str = "sess_a41504d51dd44bfca7e51968e0e88f24";

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
        std::fs::write(d.path().join("a.txt"), "v1\n").unwrap();
        std::fs::write(d.path().join("keep.txt"), "untouched\n").unwrap();
        std::fs::write(d.path().join(".gitignore"), "target/\n").unwrap();
        sh(d.path(), &["add", "-A"]);
        sh(d.path(), &["commit", "-qm", "init"]);
        d
    }

    fn read(p: &Path, f: &str) -> Option<String> {
        std::fs::read_to_string(p.join(f)).ok()
    }

    #[test]
    fn restoring_puts_every_kind_of_change_back() {
        let d = repo();
        let p = d.path();
        record(p, S, "first", None);
        // The turn: an edit, a new file, a deletion, and build output.
        std::fs::write(p.join("a.txt"), "v2\n").unwrap();
        std::fs::write(p.join("new.rs"), "fn x() {}\n").unwrap();
        std::fs::remove_file(p.join("keep.txt")).unwrap();
        std::fs::create_dir(p.join("target")).unwrap();
        std::fs::write(p.join("target/out"), "bin").unwrap();

        let plan = preview(p, S, "first", 0).unwrap();
        let got: Vec<_> = plan.iter().map(|c| (c.path.as_str(), c.action)).collect();
        assert_eq!(
            got,
            vec![
                ("a.txt", RestoreAction::Revert),
                ("keep.txt", RestoreAction::Recreate),
                ("new.rs", RestoreAction::Remove),
            ]
        );

        let r = restore(p, S, "first", 0).unwrap();
        assert_eq!(r.changes.len(), 3);
        assert_eq!(read(p, "a.txt").as_deref(), Some("v1\n"));
        assert_eq!(read(p, "keep.txt").as_deref(), Some("untouched\n"));
        assert!(read(p, "new.rs").is_none());
        // Ignored output is never touched.
        assert_eq!(read(p, "target/out").as_deref(), Some("bin"));

        // And the restore can be undone.
        undo(p, S, &r.undo).unwrap();
        assert_eq!(read(p, "a.txt").as_deref(), Some("v2\n"));
        assert!(read(p, "keep.txt").is_none());
        assert_eq!(read(p, "new.rs").as_deref(), Some("fn x() {}\n"));
    }

    #[test]
    fn repeated_messages_are_told_apart_from_the_latest() {
        let d = repo();
        let p = d.path();
        record(p, S, "fix it", None);
        std::fs::write(p.join("a.txt"), "v2\n").unwrap();
        record(p, S, "fix it", None);
        std::fs::write(p.join("a.txt"), "v3\n").unwrap();

        restore(p, S, "fix it", 0).unwrap(); // the latest "fix it"
        assert_eq!(read(p, "a.txt").as_deref(), Some("v2\n"));
        restore(p, S, "fix it", 1).unwrap(); // the one before
        assert_eq!(read(p, "a.txt").as_deref(), Some("v1\n"));
        assert!(restore(p, S, "fix it", 2).is_err());
    }

    #[test]
    fn an_edited_message_replaces_its_checkpoint_and_later_ones() {
        let d = repo();
        let p = d.path();
        record(p, S, "one", None);
        record(p, S, "two", None);
        record(p, S, "three", None);
        record(p, S, "two, better", Some(("two", 0)));
        let store = load(&store_file(p, S).unwrap());
        let texts: Vec<_> = store.checkpoints.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(texts, vec!["one", "two, better"]);
    }

    #[test]
    fn the_index_is_left_alone() {
        let d = repo();
        let p = d.path();
        std::fs::write(p.join("a.txt"), "staged\n").unwrap();
        sh(p, &["add", "a.txt"]);
        record(p, S, "go", None);
        std::fs::write(p.join("a.txt"), "after\n").unwrap();
        restore(p, S, "go", 0).unwrap();
        assert_eq!(read(p, "a.txt").as_deref(), Some("staged\n"));
        let staged = run(git(p).args(["diff", "--cached", "--name-only"])).unwrap();
        assert_eq!(staged.trim(), "a.txt");
    }

    #[test]
    fn only_saved_undo_points_are_accepted() {
        let d = repo();
        let p = d.path();
        record(p, S, "go", None);
        let head_tree = run(git(p).args(["rev-parse", "HEAD^{tree}"])).unwrap();
        assert!(undo(p, S, head_tree.trim()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn executable_bits_survive() {
        use std::os::unix::fs::PermissionsExt;
        let d = repo();
        let p = d.path();
        std::fs::write(p.join("run.sh"), "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(p.join("run.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
        record(p, S, "go", None);
        std::fs::remove_file(p.join("run.sh")).unwrap();
        restore(p, S, "go", 0).unwrap();
        let mode = std::fs::metadata(p.join("run.sh"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111);
    }
}
