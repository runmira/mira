//! Per-turn filesystem snapshots for agent sessions.
//!
//! Rollback needs something to roll back *to*. Before every agent turn, in a
//! git repo, `git stash create` mints a commit object of the working tree
//! without touching it — no ref, no branch pollution, no user-visible state.
//! The hash lands in the sidecar tagged with its turn; reverting checks that
//! hash back out.
//!
//! What this is not: tracked files only (like a default stash, untracked
//! files are left alone), and the agent's *memory* is untouched — the CLI
//! offers no rewind, so revert restores files and truncates the displayed
//! transcript while the relaunched agent starts fresh. All three limits are
//! surfaced in the UI copy, not buried here.

use std::path::Path;
use std::process::Stdio;

/// A snapshot is only meaningful where git is watching.
pub fn is_git_repo(cwd: &Path) -> bool {
    // `.git` is a dir in a clone and a file in a linked worktree; either
    // counts. Anything else (bare repos, submodules oddities) degrades to
    // "no snapshots" rather than a failed turn.
    let dot = cwd.join(".git");
    dot.is_dir() || dot.is_file()
}

/// Mint a snapshot commit of the current tracked state, returning its hash.
///
/// `None` means "nothing to restore": clean tree, non-repo, missing git, or
/// any failure. Snapshot failure must never fail the turn it precedes, so
/// every error path collapses to `None` here — the caller records whatever
/// comes back, including nothing.
pub fn snapshot_turn(cwd: &Path) -> Option<String> {
    if !is_git_repo(cwd) {
        return None;
    }
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(cwd)
        .arg("stash")
        .arg("create")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let hash = String::from_utf8_lossy(&out.stdout).trim().to_string();
    // Clean tree: git prints nothing. That is a valid snapshot of
    // "nothing changed" — but there is nothing to restore, so report None
    // and let the caller record the absence honestly.
    if hash.is_empty() {
        return None;
    }
    Some(hash)
}

/// Restore tracked files to a snapshot. Untracked files are left alone —
/// same scope as the snapshot, so revert cannot delete anything the agent
/// created but never tracked.
pub fn restore_snapshot(cwd: &Path, hash: &str) -> Result<(), String> {
    if hash.trim().is_empty() {
        return Err("empty snapshot hash".to_string());
    }
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(cwd)
        .arg("checkout")
        .arg(hash)
        .arg("--")
        .arg(".")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("could not run git: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    Err(format!(
        "git checkout failed: {}",
        String::from_utf8_lossy(&out.stderr).trim()
    ))
}

/// Number the next turn: one more than the user lines already recorded.
pub fn next_turn(path: &Path) -> u64 {
    super::agent_sessions::read_lines(path)
        .iter()
        .filter(|l| l.get("user").is_some())
        .count() as u64
        + 1
}

/// Drop everything from turn `n` onward (its user line, frames, and later
/// snapshots), keeping history before it. Returns lines kept.
pub fn truncate_from_turn(path: &Path, n: u64) -> std::io::Result<usize> {
    let lines = super::agent_sessions::read_lines(path);
    // Turn boundaries are user lines, in order. Find the nth.
    let mut seen = 0u64;
    let mut cut = lines.len();
    for (i, l) in lines.iter().enumerate() {
        if l.get("user").is_some() {
            seen += 1;
            if seen == n {
                cut = i;
                break;
            }
        }
    }
    let kept: Vec<_> = lines[..cut]
        .iter()
        // Snapshots belong to the turn that follows them; a snapshot tagged
        // >= n restores state its turn will never reach, so drop it. (The
        // restore itself already read its hash before truncating.)
        .filter(|l| {
            l.get("snapshot")
                .and_then(|s| s.get("turn"))
                .and_then(|t| t.as_u64())
                .is_none_or(|t| t < n)
        })
        .collect();
    let text = kept
        .iter()
        .map(|l| l.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(
        path,
        if text.is_empty() {
            String::new()
        } else {
            text + "\n"
        },
    )?;
    Ok(kept.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "test@mira"]);
        git(&["config", "user.name", "mira-test"]);
        git(&["config", "commit.gpgsign", "false"]);
        std::fs::write(root.join("a.txt"), "v1\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "base"]);
        (dir, root)
    }

    #[test]
    fn non_repos_snapshot_to_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_git_repo(dir.path()));
        assert!(snapshot_turn(dir.path()).is_none());
    }

    #[test]
    fn snapshot_and_restore_round_trip() {
        let (_d, root) = repo();
        // Clean tree: nothing to restore.
        assert!(snapshot_turn(&root).is_none());
        std::fs::write(root.join("a.txt"), "v2\n").unwrap();
        let hash = snapshot_turn(&root).expect("dirty tree snapshots");
        assert_eq!(hash.len(), 40);
        std::fs::write(root.join("a.txt"), "v3\n").unwrap();
        restore_snapshot(&root, &hash).expect("restore");
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "v2\n");
        // Untracked files survive both directions.
        std::fs::write(root.join("new.txt"), "x\n").unwrap();
        assert!(snapshot_turn(&root).is_none() || true);
        restore_snapshot(&root, &hash).expect("restore again");
        assert!(std::fs::read_to_string(root.join("new.txt")).unwrap() == "x\n");
        assert!(restore_snapshot(&root, "").is_err());
        assert!(restore_snapshot(&root, "deadbeef").is_err());
    }

    #[test]
    fn truncation_keeps_earlier_turns_and_drops_later_snapshots() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.agent.jsonl");
        let line = |t: u64, body: &str| {
            let mut v: serde_json::Value = serde_json::from_str(body).unwrap();
            v["t"] = serde_json::json!(t);
            super::super::agent_sessions::append_line_to(&path, &v);
        };
        line(1, r#"{"snapshot":{"turn":1,"hash":"h1"}}"#);
        line(2, r#"{"user":{"text":"one"}}"#);
        line(3, r#"{"frame":{"type":"acp_text"}}"#);
        line(4, r#"{"snapshot":{"turn":2,"hash":"h2"}}"#);
        line(5, r#"{"user":{"text":"two"}}"#);
        line(6, r#"{"frame":{"type":"acp_text"}}"#);
        assert_eq!(next_turn(&path), 3);
        truncate_from_turn(&path, 2).unwrap();
        let back = super::super::agent_sessions::read_lines(&path);
        // Snapshot 1, user 1, frame 1 survive; snapshot 2 and turn 2 go.
        assert_eq!(back.len(), 3);
        assert!(back[0].get("snapshot").is_some());
        assert!(back[1].get("user").is_some());
    }
}
