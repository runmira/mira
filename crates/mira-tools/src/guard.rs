//! Per-session file safety layer: read-watermarks + pre-write snapshots.
//!
//! Two related concerns share this module:
//!
//! - **Conflict detection.** When `read_file` observes a file, we stash a
//!   watermark (mtime + size). Later, when `edit_file` or `write_file`
//!   targets the same path, we compare against the current metadata. If it
//!   moved *without* our involvement, the user (or another process) edited
//!   it mid-turn — refuse the write instead of silently clobbering.
//!
//! - **Undo.** Before any successful `edit_file`/`write_file`, we snapshot
//!   the pre-image into `.mira/.undo/<session>/<seq>-<path>` and append a
//!   line to `manifest.jsonl`. `undo(n)` reads the last `n` lines in
//!   reverse and restores.
//!
//! The design is deliberately optional (`Option<Arc<FileGuard>>` on
//! `ToolContext`) so headless / non-persistent runs can skip the whole
//! thing without special-casing the tools.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::fs;
use tokio::sync::Mutex;
use tracing::{debug, warn};

#[derive(Debug, Error)]
pub enum GuardError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("serde: {0}")]
    Serde(#[from] serde_json::Error),
    #[error(
        "[file-conflict] {path} was modified on disk since it was last read \
         this session — reload with read_file (or ask the user) before writing"
    )]
    Conflict { path: String },
    #[error("no undo history for this session")]
    Empty,
}

/// mtime + size signature. Not cryptographically strong; catches
/// concurrent-editor mistakes, not adversarial tampering.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Watermark {
    /// Seconds since UNIX epoch. `0` when we couldn't read mtime (rare).
    pub mtime_secs: u64,
    pub size: u64,
}

impl Watermark {
    pub async fn of(path: &Path) -> Result<Option<Self>, GuardError> {
        match fs::metadata(path).await {
            Ok(m) => Ok(Some(Self {
                mtime_secs: m
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
                size: m.len(),
            })),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn matches(&self, other: &Watermark) -> bool {
        self.mtime_secs == other.mtime_secs && self.size == other.size
    }
}

/// One row in the undo manifest.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UndoEntry {
    pub seq: u64,
    /// Path relative to the session's cwd (portable across worktree swaps).
    pub path: String,
    pub op: UndoOp,
    /// Snapshot file name under the session's undo dir. Absent for `create`
    /// (no pre-image to restore — undo is a delete).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<String>,
    /// Millisecond epoch when the operation ran.
    pub ts_ms: u64,
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UndoOp {
    /// Existing file was overwritten or edited. Undo restores the snapshot.
    Overwrite,
    /// File was newly created. Undo deletes it.
    Create,
    /// Bash-mediated change: a shell command touched a tracked file that
    /// was clean before the command. Undo restores the snapshot (grabbed
    /// from git HEAD at the time of the bash invocation).
    BashModify,
    /// Bash-mediated creation: bash produced an untracked file. Undo
    /// deletes it.
    BashCreate,
}

/// Per-session file-safety state. Cheap to Clone (all fields are Arc /
/// atomics), so tools hold on to their own reference through ToolContext.
#[derive(Clone)]
pub struct FileGuard {
    /// `.mira/.undo/<session>/` — snapshots + manifest live here.
    root: PathBuf,
    /// The repo cwd. Manifest paths are stored relative to this so a session
    /// resumed after a worktree swap still reverts to the right locations.
    cwd: PathBuf,
    /// Absolute path → last-seen watermark from `read_file`.
    watermarks: Arc<Mutex<HashMap<PathBuf, Watermark>>>,
    /// Paths we've written to in this session. Once we've authored a change
    /// to `p`, future mtime changes come from us — no false conflict.
    written: Arc<Mutex<HashSet<PathBuf>>>,
    seq: Arc<AtomicU64>,
}

impl FileGuard {
    /// Construct + prepare the on-disk state. Idempotent: safe to call for
    /// resumed sessions with an existing undo dir (seq is bumped past the
    /// highest entry so old snapshots aren't overwritten). Sync so it can
    /// be called from `Session::new` without turning the harness async.
    pub fn open(session_id: &str, cwd: PathBuf) -> Result<Self, GuardError> {
        let root = cwd.join(".mira").join(".undo").join(session_id);
        std::fs::create_dir_all(&root)?;

        // Seed seq from the existing manifest so resuming doesn't collide
        // with prior entries.
        let manifest = root.join("manifest.jsonl");
        let mut seq = 0u64;
        if let Ok(body) = std::fs::read_to_string(&manifest) {
            for line in body.lines() {
                if let Ok(e) = serde_json::from_str::<UndoEntry>(line) {
                    if e.seq > seq {
                        seq = e.seq;
                    }
                }
            }
        }

        // Rehydrate read watermarks from the persisted snapshot so a restart
        // mid-session keeps its "these files were seen at version X" memory.
        let watermarks: HashMap<PathBuf, Watermark> = match std::fs::read_to_string(root.join("watermarks.json")) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => HashMap::new(),
        };
        let written: HashSet<PathBuf> = match std::fs::read_to_string(root.join("written.json")) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => HashSet::new(),
        };

        Ok(Self {
            root,
            cwd,
            watermarks: Arc::new(Mutex::new(watermarks)),
            written: Arc::new(Mutex::new(written)),
            seq: Arc::new(AtomicU64::new(seq)),
        })
    }

    /// Flush the in-memory watermark + written sets to disk. Best-effort:
    /// a failed write means we lose the conflict layer across restart for
    /// paths visited since the last flush, but doesn't break the session.
    async fn persist_maps(&self) {
        let watermarks = self.watermarks.lock().await.clone();
        let written = self.written.lock().await.clone();
        let watermarks_path = self.root.join("watermarks.json");
        let written_path = self.root.join("written.json");
        // Serialise off the async task's critical path.
        tokio::task::spawn_blocking(move || {
            if let Ok(s) = serde_json::to_string(&watermarks) {
                let _ = std::fs::write(watermarks_path, s);
            }
            if let Ok(s) = serde_json::to_string(&written) {
                let _ = std::fs::write(written_path, s);
            }
        })
        .await
        .ok();
    }

    /// Snapshot the set of paths this session has written to so far.
    /// Cheap clone of the underlying HashSet — callers diff it across a
    /// turn to compute "what did the model touch this turn?"
    pub async fn written_snapshot(&self) -> HashSet<PathBuf> {
        self.written.lock().await.clone()
    }

    /// Called by `read_file` after a successful read. Records what the file
    /// looked like so a later write can detect out-of-band changes.
    pub async fn record_read(&self, abs_path: &Path) {
        if let Ok(Some(w)) = Watermark::of(abs_path).await {
            self.watermarks.lock().await.insert(abs_path.to_path_buf(), w);
            self.persist_maps().await;
        }
    }

    /// Called by write/edit tools right before mutating `abs_path`. Fails if
    /// we recorded a watermark and the file has since drifted without us
    /// touching it. First-write to a path never seen by `read_file` is
    /// allowed — you can create new files without reading them first.
    pub async fn check_conflict(&self, abs_path: &Path) -> Result<(), GuardError> {
        // We own subsequent changes to files we've already written to.
        if self.written.lock().await.contains(abs_path) {
            return Ok(());
        }
        let recorded = { self.watermarks.lock().await.get(abs_path).copied() };
        let Some(expected) = recorded else {
            return Ok(());
        };
        let Some(current) = Watermark::of(abs_path).await? else {
            // File disappeared between read and write — treat as conflict.
            return Err(GuardError::Conflict {
                path: abs_path.display().to_string(),
            });
        };
        if !expected.matches(&current) {
            return Err(GuardError::Conflict {
                path: abs_path.display().to_string(),
            });
        }
        Ok(())
    }

    /// Snapshot the pre-image of `abs_path` before a write, append a manifest
    /// entry, and stamp the path as "written by us." If the path doesn't
    /// exist yet, records a `Create` entry with no snapshot so undo can
    /// remove it.
    pub async fn snapshot_before(&self, abs_path: &Path) -> Result<(), GuardError> {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let rel = pathdiff_or_abs(&self.cwd, abs_path);

        let (op, snapshot_name) = match fs::read(abs_path).await {
            Ok(bytes) => {
                let name = format!("{:04}-{}", seq, escape_for_fs(&rel));
                fs::write(self.root.join(&name), bytes).await?;
                (UndoOp::Overwrite, Some(name))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (UndoOp::Create, None),
            Err(e) => return Err(e.into()),
        };

        let entry = UndoEntry {
            seq,
            path: rel,
            op,
            snapshot: snapshot_name,
            ts_ms: now_ms(),
        };
        let manifest = self.root.join("manifest.jsonl");
        let line = serde_json::to_string(&entry)? + "\n";
        // Append. `append(true).create(true)` opens or creates.
        use tokio::io::AsyncWriteExt;
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&manifest)
            .await?;
        f.write_all(line.as_bytes()).await?;
        f.flush().await?;

        self.written.lock().await.insert(abs_path.to_path_buf());
        self.persist_maps().await;
        Ok(())
    }

    /// Read all undo entries (oldest → newest). Cheap enough to re-read
    /// each call; the file only holds one line per edit.
    pub async fn load_manifest(&self) -> Result<Vec<UndoEntry>, GuardError> {
        let path = self.root.join("manifest.jsonl");
        let body = match fs::read_to_string(&path).await {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(e.into()),
        };
        let mut out = Vec::new();
        for line in body.lines() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<UndoEntry>(line) {
                Ok(e) => out.push(e),
                Err(err) => warn!(?err, line, "skipping malformed undo entry"),
            }
        }
        Ok(out)
    }

    /// Revert the last `n` operations. Applies in reverse order so a later
    /// edit of the same file lands *before* the earlier one is restored.
    /// Returns descriptions of the applied reverts for UI feedback.
    pub async fn undo(&self, n: usize) -> Result<Vec<AppliedUndo>, GuardError> {
        let mut entries = self.load_manifest().await?;
        if entries.is_empty() {
            return Err(GuardError::Empty);
        }
        let take = n.min(entries.len());
        let tail: Vec<UndoEntry> = entries.split_off(entries.len() - take);

        let mut applied = Vec::new();
        // Reverse: newest first, so overlapping edits unwind coherently.
        for entry in tail.iter().rev() {
            let target = self.cwd.join(&entry.path);
            match entry.op {
                UndoOp::Overwrite | UndoOp::BashModify => {
                    let snapshot = entry
                        .snapshot
                        .as_deref()
                        .ok_or_else(|| GuardError::Io(std::io::Error::other("missing snapshot")))?;
                    let src = self.root.join(snapshot);
                    if let Some(parent) = target.parent() {
                        fs::create_dir_all(parent).await.ok();
                    }
                    fs::copy(&src, &target).await?;
                    fs::remove_file(&src).await.ok();
                    applied.push(AppliedUndo {
                        seq: entry.seq,
                        path: entry.path.clone(),
                        op: entry.op,
                    });
                }
                UndoOp::Create | UndoOp::BashCreate => {
                    // Undo of "create" is delete. Best-effort: if the file no
                    // longer exists we still record the undo as applied.
                    let _ = fs::remove_file(&target).await;
                    applied.push(AppliedUndo {
                        seq: entry.seq,
                        path: entry.path.clone(),
                        op: entry.op,
                    });
                }
            }
        }

        // Rewrite the manifest without the reverted tail. Keep the rest so
        // successive `undo(1)` calls continue backwards.
        let mut kept = String::new();
        for e in entries.iter() {
            kept.push_str(&serde_json::to_string(e)?);
            kept.push('\n');
        }
        fs::write(self.root.join("manifest.jsonl"), kept.as_bytes()).await?;

        // Once we undo a file, drop our "we authored it" flag so a future
        // external edit is caught again.
        for a in &applied {
            self.written.lock().await.remove(&self.cwd.join(&a.path));
        }
        debug!(count = applied.len(), "undo applied");
        Ok(applied)
    }

    /* ---------- bash write tracking (git-based) ---------- */

    /// Snapshot the git-visible state of the cwd right before a bash call.
    /// Returned handle is passed back to [`Self::record_bash_changes`] so
    /// the two calls bracket a shell command. `None` when we're not in a
    /// git repo — the bash call runs normally but its writes stay
    /// untracked.
    pub fn pre_bash(&self) -> Option<BashSnapshot> {
        let head = git_output(&self.cwd, &["rev-parse", "HEAD"])?;
        let porcelain = git_output(&self.cwd, &["status", "--porcelain=v1", "-z"]).unwrap_or_default();
        Some(BashSnapshot {
            head,
            dirty: parse_porcelain_z(&porcelain),
        })
    }

    /// After a bash call, diff the working tree against the pre-snapshot
    /// and record undo entries for any file bash touched that we can
    /// safely revert. Only handles files that were **clean before the bash
    /// call** — a file that was already dirty carries user work we can't
    /// disambiguate from bash's contribution and is left alone.
    pub async fn record_bash_changes(&self, pre: &BashSnapshot) -> Result<usize, GuardError> {
        let porcelain = match git_output(&self.cwd, &["status", "--porcelain=v1", "-z"]) {
            Some(s) => s,
            None => return Ok(0),
        };
        let after = parse_porcelain_z(&porcelain);
        let mut recorded = 0usize;
        for (path, code) in &after {
            let was = pre.dirty.get(path);
            if was == Some(code) {
                continue; // unchanged since pre-bash
            }
            if was.is_some() {
                // Was already dirty — can't cleanly attribute changes to
                // bash vs prior user work. Skip.
                continue;
            }
            let untracked = code.starts_with("??");
            let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
            let entry = if untracked {
                UndoEntry {
                    seq,
                    path: path.clone(),
                    op: UndoOp::BashCreate,
                    snapshot: None,
                    ts_ms: now_ms(),
                }
            } else {
                // Pull the HEAD blob for this path into our undo dir.
                let Some(blob) = git_show_head(&self.cwd, &pre.head, path) else {
                    warn!(path, "bash-modified file has no HEAD blob; skipping undo");
                    continue;
                };
                let name = format!("{:04}-{}", seq, escape_for_fs(path));
                fs::write(self.root.join(&name), blob).await?;
                UndoEntry {
                    seq,
                    path: path.clone(),
                    op: UndoOp::BashModify,
                    snapshot: Some(name),
                    ts_ms: now_ms(),
                }
            };
            self.append_manifest(&entry).await?;
            self.written.lock().await.insert(self.cwd.join(path));
            recorded += 1;
        }
        if recorded > 0 {
            self.persist_maps().await;
        }
        Ok(recorded)
    }

    async fn append_manifest(&self, entry: &UndoEntry) -> Result<(), GuardError> {
        use tokio::io::AsyncWriteExt;
        let line = serde_json::to_string(entry)? + "\n";
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.root.join("manifest.jsonl"))
            .await?;
        f.write_all(line.as_bytes()).await?;
        f.flush().await?;
        Ok(())
    }
}

/// Opaque handle returned by [`FileGuard::pre_bash`] and consumed by
/// [`FileGuard::record_bash_changes`]. Cheap to hold across an await.
#[derive(Clone, Debug)]
pub struct BashSnapshot {
    /// Git HEAD sha at pre-bash time — used to fetch clean pre-images.
    head: String,
    /// path → porcelain status code (`" M"`, `"??"`, …) as it looked
    /// before bash ran. Anything not in this map was tracked-and-clean.
    dirty: HashMap<String, String>,
}

fn git_output(cwd: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git").current_dir(cwd).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// Extract a file's HEAD blob content. Returns `None` if the path isn't
/// in HEAD (path was staged-only, etc).
fn git_show_head(cwd: &Path, head: &str, path: &str) -> Option<Vec<u8>> {
    let out = Command::new("git")
        .current_dir(cwd)
        .args(["show", &format!("{head}:{path}")])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(out.stdout)
}

/// Parse the NUL-separated output of `git status --porcelain=v1 -z`.
/// Yields (path, "XY") pairs. Renames are split at " -> " (v1 uses
/// arrow-separated old/new paths); we take the new path.
fn parse_porcelain_z(s: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for record in s.split('\0') {
        if record.len() < 3 {
            continue;
        }
        let code = &record[..2];
        let path = &record[3..];
        let path = path.rsplit(" -> ").next().unwrap_or(path);
        out.insert(path.to_owned(), code.to_owned());
    }
    out
}

/// UI-facing description of one applied revert.
#[derive(Clone, Debug, Serialize)]
pub struct AppliedUndo {
    pub seq: u64,
    pub path: String,
    pub op: UndoOp,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn pathdiff_or_abs(base: &Path, path: &Path) -> String {
    match path.strip_prefix(base) {
        Ok(rel) => rel.to_string_lossy().into_owned(),
        Err(_) => path.to_string_lossy().into_owned(),
    }
}

/// Turn a path fragment into something safe as a single filename — no
/// separators, no leading dots. We keep it human-readable so the undo dir
/// isn't a wall of hashes.
fn escape_for_fs(rel: &str) -> String {
    rel.chars()
        .map(|c| match c {
            '/' | '\\' => '_',
            c if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') => c,
            _ => '_',
        })
        .collect()
}
