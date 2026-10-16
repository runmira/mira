//! Which files a turn itself changed.
//!
//! A turn's checkpoint diff compares the whole working tree before and
//! after the turn, so anything else that wrote to the folder meanwhile —
//! another chat in the same repo, most often, while this one waited on a
//! question — was credited to the turn. While a turn runs, its own tool
//! calls are watched on the slot's event channel: files its edit tools
//! name are its own, and a shell command (which could have changed
//! anything) lets it keep the remaining changes except those another
//! chat's edit tools claimed during the turn.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::Value;

use crate::protocol::ServerMsg;
use crate::slot::SessionSlot;

#[derive(Default)]
struct Touch {
    paths: HashSet<PathBuf>,
    shell: bool,
}

struct Watch {
    touch: Arc<Mutex<Touch>>,
    task: tokio::task::JoinHandle<()>,
}

/// Session id → the turn being watched.
fn watches() -> &'static Mutex<HashMap<String, Watch>> {
    static W: OnceLock<Mutex<HashMap<String, Watch>>> = OnceLock::new();
    W.get_or_init(Default::default)
}

/// Absolute path → (session that edited it, unix seconds).
fn claims() -> &'static Mutex<HashMap<PathBuf, (String, u64)>> {
    static C: OnceLock<Mutex<HashMap<PathBuf, (String, u64)>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Start watching `slot`'s next turn, replacing any watch left over.
pub fn begin(slot: &SessionSlot, cwd: PathBuf) {
    let session = slot.id.to_string();
    let touch = Arc::new(Mutex::new(Touch::default()));
    let mut rx = slot.events_tx.subscribe();
    let task = {
        let touch = touch.clone();
        let session = session.clone();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(frame) => observe(&touch, &session, &cwd, &frame),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        // Steps were missed: be as generous as a shell.
                        touch.lock().unwrap_or_else(|e| e.into_inner()).shell = true;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        })
    };
    if let Some(old) = watches()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(session, Watch { touch, task })
    {
        old.task.abort();
    }
}

/// What the finished turn may claim, as paths relative to `cwd`; `None`
/// when the turn wasn't watched (keep the whole diff, as before).
pub struct Scope {
    own: HashSet<String>,
    shell: bool,
    others: HashSet<String>,
}

impl Scope {
    pub fn keeps(&self, path: &str) -> bool {
        self.own.contains(path) || (self.shell && !self.others.contains(path))
    }
}

/// Stop watching `session`'s turn (started at `since`, unix seconds).
pub fn end(session: &str, cwd: &Path, since: u64) -> Option<Scope> {
    let watch = watches()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(session)?;
    watch.task.abort();
    let touch = std::mem::take(&mut *watch.touch.lock().unwrap_or_else(|e| e.into_inner()));
    let root = canonical(cwd);
    let rel = |p: &Path| p.strip_prefix(&root).ok().map(|r| r.to_string_lossy().into_owned());
    let others = claims()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .filter(|(_, (by, at))| by != session && *at >= since)
        .filter_map(|(p, _)| rel(p))
        .collect();
    Some(Scope {
        own: touch.paths.iter().filter_map(|p| rel(p)).collect(),
        shell: touch.shell,
        others,
    })
}

/// Drop claims older than a day so the map can't grow without bound.
fn prune(claims: &mut HashMap<PathBuf, (String, u64)>) {
    let cutoff = now_secs().saturating_sub(24 * 60 * 60);
    claims.retain(|_, (_, at)| *at >= cutoff);
}

fn canonical(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// `raw` resolved against `cwd`, canonical where it exists (a deleted
/// file's parent still does).
fn resolve(cwd: &Path, raw: &str) -> Option<PathBuf> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let p = Path::new(raw);
    let abs = if p.is_absolute() { p.to_path_buf() } else { cwd.join(p) };
    if let Ok(c) = std::fs::canonicalize(&abs) {
        return Some(c);
    }
    let parent = abs.parent().map(canonical)?;
    Some(parent.join(abs.file_name()?))
}

fn observe(touch: &Mutex<Touch>, session: &str, cwd: &Path, frame: &ServerMsg) {
    let (paths, shell) = match frame {
        ServerMsg::ToolStart { call } => {
            let args: Value = serde_json::from_str(&call.function.arguments).unwrap_or(Value::Null);
            mira_tool_paths(&call.function.name, &args)
        }
        ServerMsg::AcpToolCall { call } => agent_tool_paths(call),
        _ => return,
    };
    if paths.is_empty() && !shell {
        return;
    }
    let resolved: Vec<PathBuf> = paths.iter().filter_map(|p| resolve(cwd, p)).collect();
    {
        let mut t = touch.lock().unwrap_or_else(|e| e.into_inner());
        t.shell |= shell;
        t.paths.extend(resolved.iter().cloned());
    }
    if !resolved.is_empty() {
        let mut c = claims().lock().unwrap_or_else(|e| e.into_inner());
        prune(&mut c);
        let at = now_secs();
        for p in resolved {
            c.insert(p, (session.to_owned(), at));
        }
    }
}

/// Files one of Mira's own tools edits, and whether it can change files it
/// doesn't name.
fn mira_tool_paths(name: &str, args: &Value) -> (Vec<String>, bool) {
    let s = |k: &str| args.get(k).and_then(Value::as_str).map(str::to_owned);
    match name {
        "write_file" | "edit_file" => (s("path").into_iter().collect(), false),
        "apply_patch" => (s("patch").map(|p| patch_paths(&p)).unwrap_or_default(), false),
        "bash" | "run_background" | "rustfmt" | "ast_grep" => (Vec::new(), true),
        n if n.starts_with("git") => (Vec::new(), true),
        _ => (Vec::new(), false),
    }
}

fn patch_paths(patch: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in patch.lines() {
        for prefix in ["*** Add File: ", "*** Update File: ", "*** Delete File: "] {
            if let Some(p) = line.strip_prefix(prefix) {
                out.push(p.trim().to_owned());
            }
        }
        if let Some(rest) = line.strip_prefix("*** Move File: ") {
            out.extend(rest.split(" -> ").map(|p| p.trim().to_owned()));
        }
    }
    out
}

/// Files an external agent's tool call edits (its locations, diffs and
/// usual path arguments), and whether it ran a command.
fn agent_tool_paths(call: &mira_acp::events::ToolCallState) -> (Vec<String>, bool) {
    use mira_acp::events::{AcpToolKind as ToolKind, ToolContent};
    match call.kind {
        Some(ToolKind::Execute) => return (Vec::new(), true),
        Some(ToolKind::Edit) | Some(ToolKind::Delete) | Some(ToolKind::Move) => {}
        _ => return (Vec::new(), false),
    }
    let mut out: Vec<String> = call.locations.iter().map(|l| l.path.clone()).collect();
    for c in &call.content {
        if let ToolContent::Diff { path, .. } = c {
            out.push(path.clone());
        }
    }
    if let Some(input) = &call.raw_input {
        for k in ["file_path", "path", "notebook_path"] {
            if let Some(p) = input.get(k).and_then(Value::as_str) {
                out.push(p.to_owned());
            }
        }
    }
    (out, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_sections_name_every_file() {
        let p = "*** Begin Patch\n*** Update File: a.rs\n-x\n+y\n*** Add File: b.rs\n+z\n\
                 *** Move File: c.rs -> d.rs\n*** End Patch";
        assert_eq!(patch_paths(p), ["a.rs", "b.rs", "c.rs", "d.rs"]);
    }

    #[test]
    fn a_scope_keeps_its_own_files_and_shell_changes_nobody_else_claimed() {
        let scope = Scope {
            own: ["a.rs".to_owned()].into(),
            shell: false,
            others: ["b.rs".to_owned()].into(),
        };
        assert!(scope.keeps("a.rs"));
        assert!(!scope.keeps("b.rs"));
        assert!(!scope.keeps("c.rs"));
        let shell = Scope { shell: true, ..scope };
        assert!(shell.keeps("c.rs"));
        assert!(!shell.keeps("b.rs"));
    }
}
