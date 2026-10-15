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
    #[serde(default)]
    result_tree: Option<String>,
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

fn store_lock(file: &Path) -> std::sync::Arc<std::sync::Mutex<()>> {
    static LOCKS: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<PathBuf, std::sync::Weak<std::sync::Mutex<()>>>>,
    > = std::sync::OnceLock::new();
    let mut locks = LOCKS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(file).and_then(std::sync::Weak::upgrade) {
        return lock;
    }
    let lock = std::sync::Arc::new(std::sync::Mutex::new(()));
    locks.insert(file.to_owned(), std::sync::Arc::downgrade(&lock));
    lock
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
    if !crate::session_changes::in_repo(cwd) {
        for repo in crate::session_changes::repositories(cwd) {
            record(&repo, session, text, replaces);
        }
        return;
    }
    let Some(file) = store_file(cwd, session) else {
        return;
    };
    let lock = store_lock(&file);
    // Another snapshot may be running for this chat. Include lock contention
    // in the prompt's budget, not just time spent inside Git.
    let _guard = loop {
        match lock.try_lock() {
            Ok(guard) => break guard,
            Err(std::sync::TryLockError::Poisoned(error)) => break error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                if crate::session_changes::snapshot_budget_expired() {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
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
        result_tree: None,
        at: now_secs(),
    });
    save(&file, &store);
}

#[derive(Clone, Debug, Serialize)]
pub struct TurnDiffSummary {
    pub text: String,
    pub occurrence: usize,
    pub files: Vec<crate::session_changes::ChangedFile>,
}

pub fn pending_tree(cwd: &Path, session: &str) -> Option<String> {
    let file = store_file(cwd, session)?;
    load(&file)
        .checkpoints
        .iter()
        .enumerate()
        .next_back()
        .filter(|(_, checkpoint)| checkpoint.result_tree.is_none())
        .map(|(index, checkpoint)| format!("{index}:{}", checkpoint.tree))
}
pub fn finish(cwd: &Path, session: &str, expected_tree: &str) {
    let Some(file) = store_file(cwd, session) else {
        return;
    };
    let lock = store_lock(&file);
    let _guard = loop {
        match lock.try_lock() {
            Ok(guard) => break guard,
            Err(std::sync::TryLockError::Poisoned(error)) => break error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                if crate::session_changes::snapshot_budget_expired() {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    };
    let result = snapshot_tree(cwd);
    let mut store = load(&file);
    let Some((index, tree)) = expected_tree.split_once(':') else {
        return;
    };
    let Ok(index) = index.parse::<usize>() else {
        return;
    };
    let Some(checkpoint) = store
        .checkpoints
        .get_mut(index)
        .filter(|checkpoint| checkpoint.tree == tree && checkpoint.result_tree.is_none())
    else {
        return;
    };
    checkpoint.result_tree = result;
    save(&file, &store);
}

pub fn turn_diffs(cwd: &Path, session: &str) -> Vec<TurnDiffSummary> {
    if !crate::session_changes::in_repo(cwd) {
        let mut merged: Vec<TurnDiffSummary> = vec![];
        for repo in crate::session_changes::owned_repositories(cwd, session) {
            let prefix = repo.strip_prefix(cwd).unwrap();
            for mut summary in turn_diffs(&repo, session) {
                for file in &mut summary.files {
                    file.path = prefix.join(&file.path).to_string_lossy().into_owned();
                }
                if let Some(existing) = merged.iter_mut().find(|other| {
                    other.text == summary.text && other.occurrence == summary.occurrence
                }) {
                    existing.files.extend(summary.files);
                } else {
                    merged.push(summary);
                }
            }
        }
        return merged;
    }
    let Some(file) = store_file(cwd, session) else {
        return vec![];
    };
    let store = load(&file);
    store
        .checkpoints
        .iter()
        .enumerate()
        .filter_map(|(index, checkpoint)| {
            let result = checkpoint.result_tree.as_ref()?;
            let output = run(git(cwd).args([
                "diff",
                "--relative",
                "--numstat",
                "-z",
                "--no-renames",
                &checkpoint.tree,
                result,
            ]))?;
            Some(TurnDiffSummary {
                text: checkpoint.text.clone(),
                occurrence: store.checkpoints[index + 1..]
                    .iter()
                    .filter(|other| other.text == checkpoint.text)
                    .count(),
                files: crate::session_changes::parse_numstat_z(&output),
            })
        })
        .collect()
}

/// Read only the frozen before/after trees. Never substitute today's working file.
#[derive(Debug, Serialize)]
pub struct TurnFileDiff {
    #[serde(flatten)]
    pub preview: mira_tools::DiffPreview,
    pub before: String,
    pub after: String,
}
pub fn turn_file_diff(
    cwd: &Path,
    session: &str,
    text: &str,
    occurrence: usize,
    path: &str,
) -> Result<TurnFileDiff, String> {
    if path.is_empty()
        || Path::new(path)
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        return Err("invalid file path".into());
    }
    if !crate::session_changes::in_repo(cwd) {
        let (repo, relative) = crate::session_changes::file_repository(cwd, path)
            .ok_or("file is outside this workspace's repositories")?;
        let mut diff = turn_file_diff(&repo, session, text, occurrence, &relative)?;
        diff.preview.path = path.to_owned();
        return Ok(diff);
    }
    let file = store_file(cwd, session).ok_or("no checkpoints for this chat")?;
    let store = load(&file);
    let index = find(&store, text, occurrence).ok_or("no checkpoint for this turn")?;
    let checkpoint = &store.checkpoints[index];
    let result = checkpoint
        .result_tree
        .as_ref()
        .ok_or("this turn has no completed snapshot")?;
    let read = |tree: &str| -> Result<Option<String>, String> {
        if !tree_exists(cwd, tree) {
            return Err("this snapshot has expired".into());
        }
        let spec = format!("{tree}:./{path}");
        if !in_tree(cwd, tree, path) {
            return Ok(None);
        }
        let size = run(git(cwd).args(["cat-file", "-s", &spec]))
            .and_then(|s| s.trim().parse::<usize>().ok())
            .ok_or("could not read snapshot size")?;
        if size > 2 * 1024 * 1024 {
            return Err("this file is too large for an inline diff".into());
        }
        let value = run(git(cwd).args(["show", &spec])).ok_or("could not read snapshot")?;
        if value.contains('\0') {
            return Err("binary files have no text diff".into());
        }
        Ok(Some(value))
    };
    let before = read(&checkpoint.tree)?;
    let after = read(result)?;
    if before.is_none() && after.is_none() {
        return Err("this file is absent from the turn snapshots".into());
    }
    let kind = if before.is_none() {
        mira_tools::DiffKind::Create
    } else {
        mira_tools::DiffKind::Edit
    };
    let before = before.unwrap_or_default();
    let after = after.unwrap_or_default();
    Ok(TurnFileDiff {
        preview: mira_tools::preview_between(path.to_owned(), &before, &after, kind),
        before,
        after,
    })
}

pub(crate) async fn finish_slot(slot: std::sync::Arc<crate::slot::SessionSlot>) {
    // Provider/MCP writes use the harness file guard rather than native agent
    // file-change notifications. Claim those repos before freezing this turn.
    let guard = slot.session.read().await.file_guard();
    if let Some(guard) = guard {
        let paths: Vec<_> = guard.written_snapshot().await.into_iter().collect();
        if !paths.is_empty() {
            let cwd = slot.cwd.read().await.clone();
            let id = slot.id.to_string();
            let _ = tokio::task::spawn_blocking(move || {
                crate::session_changes::with_snapshot_budget(
                    std::time::Duration::from_secs(1),
                    || crate::session_changes::claim_paths(&cwd, &id, &paths),
                )
            })
            .await;
        }
    }
    if let Some(summaries) = finish_in_cwd(slot.cwd.clone(), slot.id.to_string()).await {
        let _ = slot
            .events_tx
            .send(crate::protocol::ServerMsg::TurnDiffs { summaries });
    }
}

async fn finish_in_cwd(
    cwd: std::sync::Arc<tokio::sync::RwLock<PathBuf>>,
    id: String,
) -> Option<Vec<TurnDiffSummary>> {
    let cwd = cwd.read().await.clone();
    tokio::task::spawn_blocking(move || {
        crate::session_changes::with_snapshot_budget(std::time::Duration::from_secs(5), || {
            for repo in crate::session_changes::owned_repositories(&cwd, &id) {
                if let Some(tree) = pending_tree(&repo, &id) {
                    finish(&repo, &id, &tree);
                }
            }
            turn_diffs(&cwd, &id)
        })
    })
    .await
    .ok()
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

    #[tokio::test]
    async fn workspace_turns_track_only_claimed_repos_and_keep_committed_diffs() {
        let workspace = tempfile::tempdir().unwrap();
        let first = repo();
        let other = repo();
        let owned = workspace.path().join("mira");
        let sibling = workspace.path().join("other");
        std::fs::rename(first.path(), &owned).unwrap();
        std::fs::rename(other.path(), &sibling).unwrap();
        let cwd = std::sync::Arc::new(tokio::sync::RwLock::new(workspace.path().to_path_buf()));
        // Work in the sibling before this chat claims it must stay excluded.
        crate::session_changes::ensure_baseline(workspace.path(), S);
        record(workspace.path(), S, "first", None);
        std::fs::write(sibling.join("a.txt"), "someone else\n").unwrap();
        finish_in_cwd(cwd.clone(), S.into()).await.unwrap();
        assert!(turn_diffs(workspace.path(), S).is_empty());
        record(workspace.path(), S, "second", None);
        crate::session_changes::claim_paths(workspace.path(), S, &[owned.join("a.txt")]);
        std::fs::write(owned.join("a.txt"), "v2\n").unwrap();
        std::fs::write(sibling.join("keep.txt"), "unrelated edit\n").unwrap();
        let pending = crate::session_changes::changed_files(workspace.path(), S, &[]);
        assert_eq!(
            pending
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            vec!["mira/a.txt"]
        );
        // The agent commits before ending: turn snapshots must still show it.
        sh(&owned, &["add", "a.txt"]);
        sh(&owned, &["commit", "-qm", "agent edit"]);
        let summaries = finish_in_cwd(cwd, S.into()).await.unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].text, "second");
        assert_eq!(summaries[0].files[0].path, "mira/a.txt");
        assert_eq!(summaries[0].files[0].added, 1);
        assert_eq!(summaries[0].files[0].removed, 1);
        let diff = turn_file_diff(workspace.path(), S, "second", 0, "mira/a.txt").unwrap();
        assert_eq!(diff.preview.path, "mira/a.txt");
        assert_eq!(diff.before, "v1\n");
        assert_eq!(diff.after, "v2\n");
        assert!(turn_file_diff(workspace.path(), S, "second", 0, "../other/a.txt").is_err());
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
    fn historical_diff_uses_frozen_versions_and_rejects_traversal() {
        let dir = repo();
        let cwd = dir.path();
        std::fs::write(cwd.join("a.txt"), "before\n").unwrap();
        record(cwd, S, "change", None);
        let token = pending_tree(cwd, S).unwrap();
        std::fs::write(cwd.join("a.txt"), "after\n").unwrap();
        std::fs::write(cwd.join("new.txt"), "created\n").unwrap();
        finish(cwd, S, &token);
        std::fs::write(cwd.join("a.txt"), "unrelated later edit\n").unwrap();
        let diff = turn_file_diff(cwd, S, "change", 0, "a.txt").unwrap();
        assert_eq!(diff.before, "before\n");
        assert_eq!(diff.after, "after\n");
        assert_eq!(
            turn_file_diff(cwd, S, "change", 0, "new.txt")
                .unwrap()
                .before,
            ""
        );
        assert!(turn_file_diff(cwd, S, "change", 0, "../a.txt").is_err());
        assert!(turn_file_diff(cwd, S, "change", 0, "missing.txt").is_err());
    }

    #[tokio::test]
    async fn finalization_waits_for_folder_lock_and_persists_changes() {
        let dir = repo();
        record(dir.path(), S, "edit", None);
        std::fs::write(dir.path().join("a.txt"), "edited\n").unwrap();
        let cwd = std::sync::Arc::new(tokio::sync::RwLock::new(dir.path().to_path_buf()));
        let guard = cwd.write().await;
        let task = tokio::spawn(finish_in_cwd(cwd.clone(), S.into()));
        tokio::task::yield_now().await;
        assert!(!task.is_finished());
        drop(guard);
        let summaries = task.await.unwrap().unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].text, "edit");
        assert_eq!(summaries[0].files[0].path, "a.txt");
        assert_eq!(turn_diffs(dir.path(), S).len(), 1);
        assert_eq!(
            turn_file_diff(dir.path(), S, "edit", 0, "a.txt")
                .unwrap()
                .after,
            "edited\n"
        );
    }

    #[test]
    fn completed_turn_diffs_stay_frozen_after_later_edits() {
        let dir = repo();
        record(dir.path(), S, "first", None);
        let first = pending_tree(dir.path(), S).unwrap();
        std::fs::write(dir.path().join("a.txt"), "turn one\n").unwrap();
        finish(dir.path(), S, &first);
        let summaries = turn_diffs(dir.path(), S);
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].files[0].path, "a.txt");
        record(dir.path(), S, "second", None);
        std::fs::write(dir.path().join("keep.txt"), "later edit\n").unwrap();
        finish(dir.path(), S, &first);
        assert_eq!(turn_diffs(dir.path(), S)[0].files.len(), 1);
        assert!(pending_tree(dir.path(), S).is_some());
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
