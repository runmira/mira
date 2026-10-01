//! Agent sessions beyond the current process: resume cursors and history import.
//!
//! Two separate needs, one file:
//!
//! * **Cursors.** When a Mira session drives an agent, the agent's own
//!   session id is recorded keyed by Mira session. Starting an agent for a
//!   slot with a cursor resumes it instead of starting blank — which is what
//!   makes agent sessions survive a Mira restart.
//! * **Import.** The CLIs keep their own history (`~/.claude/projects` for
//!   Claude Code). Listing it lets the user pick up a terminal session in
//!   Mira via `--resume`, instead of that history being stranded.
//!
//! The store is a single JSON file, read-modify-written by the server, which
//! is its only writer. Paths take an explicit base dir so tests never touch
//! the real home.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Where an agent session left off.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentCursor {
    pub mira_session: String,
    pub driver_kind: String,
    pub agent_session_id: String,
    pub updated_at: u64,
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn store_path(home: &Path) -> PathBuf {
    home.join(".mira/agent-cursors.json")
}

/// One cursor per (session, driver): two agents alternating in one session
/// must not overwrite each other's resume point.
fn key(mira_session: &str, driver_kind: &str) -> String {
    format!("{mira_session}\0{driver_kind}")
}

fn read_all(home: &Path) -> BTreeMap<String, AgentCursor> {
    std::fs::read_to_string(store_path(home))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// Record where an agent session left off. Overwrites any previous cursor
/// for the Mira session: there is exactly one agent per slot.
pub fn record_in(home: &Path, mira_session: &str, driver_kind: &str, agent_session_id: &str) {
    let mut all = read_all(home);
    all.insert(
        key(mira_session, driver_kind),
        AgentCursor {
            mira_session: mira_session.to_string(),
            driver_kind: driver_kind.to_string(),
            agent_session_id: agent_session_id.to_string(),
            updated_at: now_secs(),
        },
    );
    if let Some(parent) = store_path(home).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string_pretty(&all) {
        let _ = std::fs::write(store_path(home), text);
    }
}

/// The recorded cursor for a Mira session and driver.
pub fn read_in(home: &Path, mira_session: &str, driver_kind: &str) -> Option<AgentCursor> {
    read_all(home).remove(&key(mira_session, driver_kind))
}

/// Production wrappers, rooted at `$HOME`.
pub fn record(mira_session: &str, driver_kind: &str, agent_session_id: &str) {
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        record_in(&home, mira_session, driver_kind, agent_session_id);
    }
}

/// Forget a cursor. Used when history is deliberately ended (revert):
/// keeping it would resurrect the exact session just left behind.
pub fn remove_in(home: &Path, mira_session: &str, driver_kind: &str) {
    let mut all = read_all(home);
    if all.remove(&key(mira_session, driver_kind)).is_some() {
        let _ = std::fs::write(
            store_path(home),
            serde_json::to_string_pretty(&all).unwrap_or_default(),
        );
    }
}

pub fn remove(mira_session: &str, driver_kind: &str) {
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        remove_in(&home, mira_session, driver_kind);
    }
}

pub fn read(mira_session: &str, driver_kind: &str) -> Option<AgentCursor> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .and_then(|home| read_in(&home, mira_session, driver_kind))
}

/// A session from the agent's own history, resumable via its id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExternalSession {
    pub id: String,
    pub cwd: String,
    pub model: Option<String>,
    /// User + assistant message count.
    pub messages: usize,
    /// First user text, for display. Skips local-command caveats, which are
    /// noise the CLI injects, not something the user said.
    pub first_text: Option<String>,
    /// Latest entry timestamp, ISO-8601 as recorded.
    pub updated_at: Option<String>,
}

/// Decode a Claude projects dir name back to a path: leading `-` is the root
/// slash, every other `-` was a `/`.
pub fn decode_project_dir(name: &str) -> String {
    let stripped = name.strip_prefix('-').unwrap_or(name);
    format!("/{}", stripped.replace('-', "/"))
}

/// List Claude Code sessions from its own history dir.
///
/// Reads `$HOME/.claude/projects/*/*.jsonl`. Files are small (KBs); each is
/// scanned once for counts, model, timestamps and the first user text.
pub fn list_claude_sessions() -> Vec<ExternalSession> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    list_claude_sessions_in(&home.join(".claude/projects"))
}

pub fn list_claude_sessions_in(projects: &Path) -> Vec<ExternalSession> {
    let mut out = Vec::new();
    let Ok(dirs) = std::fs::read_dir(projects) else {
        return out;
    };
    for dir in dirs.flatten() {
        let dirname = dir.file_name().to_string_lossy().into_owned();
        let cwd = decode_project_dir(&dirname);
        let Ok(files) = std::fs::read_dir(dir.path()) else {
            continue;
        };
        for file in files.flatten() {
            if file.path().extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let id = file
                .path()
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            if id.is_empty() {
                continue;
            }
            if let Some(s) = summarize_file(&file.path(), &id, &cwd) {
                out.push(s);
            }
        }
    }
    // Most recently active first. ISO-8601 UTC strings sort lexicographically.
    out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    out
}

fn summarize_file(path: &Path, id: &str, cwd: &str) -> Option<ExternalSession> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut messages = 0;
    let mut model: Option<String> = None;
    let mut first_text: Option<String> = None;
    let mut updated_at: Option<String> = None;
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if let Some(ts) = v.get("timestamp").and_then(|t| t.as_str()) {
            if updated_at.as_deref().is_none_or(|cur| ts > cur) {
                updated_at = Some(ts.to_string());
            }
        }
        match v.get("type").and_then(|t| t.as_str()) {
            Some("user") => {
                messages += 1;
                if first_text.is_none() {
                    let text = match v.get("message").and_then(|m| m.get("content")) {
                        Some(serde_json::Value::String(c)) => c.clone(),
                        // Real prompts are content blocks; the caveat is a
                        // bare string, which is why only one shape worked.
                        Some(serde_json::Value::Array(blocks)) => blocks
                            .iter()
                            .filter_map(|b| {
                                (b.get("type").and_then(|t| t.as_str()) == Some("text"))
                                    .then(|| b.get("text").and_then(|t| t.as_str()))
                                    .flatten()
                            })
                            .collect::<Vec<_>>()
                            .join("\n"),
                        _ => String::new(),
                    };
                    if !text.starts_with("<local-command-caveat>") && !text.trim().is_empty() {
                        first_text = Some(text.chars().take(120).collect());
                    }
                }
            }
            Some("assistant") => {
                messages += 1;
                if let Some(m) = v
                    .get("message")
                    .and_then(|m| m.get("model"))
                    .and_then(|m| m.as_str())
                {
                    // `<synthetic>` marks machine-generated entries, not the
                    // model that did the work — never let it win.
                    if !m.starts_with('<') {
                        model = Some(m.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    if messages == 0 {
        return None;
    }
    Some(ExternalSession {
        id: id.to_string(),
        cwd: cwd.to_string(),
        model,
        messages,
        first_text,
        updated_at,
    })
}

/// Read back transcript lines, oldest first. Corrupt lines are skipped, not
/// fatal: one bad line must not lose the session.
pub fn read_lines(path: &Path) -> Vec<serde_json::Value> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|l| {
            let line = l.trim();
            (!line.is_empty())
                .then(|| serde_json::from_str(line).ok())
                .flatten()
        })
        .collect()
}

/// Append one JSON line, creating parent dirs. Failures are silent by design:
/// transcript persistence must never fail a turn.
pub fn append_line_to(path: &Path, line: &serde_json::Value) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "{}", line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().to_path_buf();
        (dir, home)
    }

    #[test]
    fn cursors_are_keyed_by_session_and_driver() {
        let (_d, home) = tmp();
        record_in(&home, "s1", "claude-code", "abc");
        record_in(&home, "s1", "codex", "th-9");
        // Same session, different drivers: neither clobbers the other.
        assert_eq!(
            read_in(&home, "s1", "claude-code")
                .unwrap()
                .agent_session_id,
            "abc"
        );
        assert_eq!(
            read_in(&home, "s1", "codex").unwrap().agent_session_id,
            "th-9"
        );
        assert!(read_in(&home, "s1", "grok").is_none());
        assert!(read_in(&home, "s2", "claude-code").is_none());
    }

    #[test]
    fn cursors_round_trip_per_session() {
        let (_d, home) = tmp();
        assert!(read_in(&home, "s1", "claude-code").is_none());
        record_in(&home, "s1", "claude-code", "abc");
        let c = read_in(&home, "s1", "claude-code").unwrap();
        assert_eq!(c.agent_session_id, "abc");
        assert_eq!(c.driver_kind, "claude-code");
        // Re-recording replaces: one cursor per (session, driver).
        record_in(&home, "s1", "claude-code", "def");
        assert_eq!(
            read_in(&home, "s1", "claude-code")
                .unwrap()
                .agent_session_id,
            "def"
        );
    }

    fn write_jsonl(dir: &Path, project: &str, name: &str, lines: &[&str]) {
        let p = dir.join(project);
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(p.join(name), lines.join("\n")).unwrap();
    }

    #[test]
    fn claude_history_lists_and_decodes() {
        let (_d, home) = tmp();
        let projects = home.join(".claude/projects");
        write_jsonl(
            &projects,
            "-Users-test-proj",
            "sess-1.jsonl",
            &[
                r#"{"type":"user","timestamp":"2026-01-01T00:00:00Z","message":{"role":"user","content":"hello world"}}"#,
                r#"{"type":"assistant","timestamp":"2026-01-01T00:01:00Z","message":{"model":"claude-opus-4-7","content":[{"type":"text","text":"hi"}]}}"#,
                r#"{"type":"user","timestamp":"2026-01-01T00:02:00Z","message":{"role":"user","content":"<local-command-caveat>noise"}}"#,
            ],
        );
        // Empty sessions are skipped.
        write_jsonl(
            &projects,
            "-Users-test-proj",
            "empty.jsonl",
            &[r#"{"type":"queue-operation","timestamp":"2026-01-01T00:00:00Z"}"#],
        );

        let found = list_claude_sessions_in(&projects);
        assert_eq!(found.len(), 1);
        let s = &found[0];
        assert_eq!(s.id, "sess-1");
        assert_eq!(s.cwd, "/Users/test/proj");
        assert_eq!(s.model.as_deref(), Some("claude-opus-4-7"));
        assert_eq!(s.messages, 3);
        assert_eq!(s.first_text.as_deref(), Some("hello world"));
        assert_eq!(s.updated_at.as_deref(), Some("2026-01-01T00:02:00Z"));
    }

    #[test]
    fn transcript_lines_round_trip_and_skip_garbage() {
        let (_d, home) = tmp();
        let path = home.join("s1.agent.jsonl");
        let line = serde_json::json!({"t": 1, "driver": "claude-code", "frame": {"type": "acp_text", "text": "hi"}});
        append_line_to(&path, &line);
        // Garbage appended by hand must not break the read.
        std::fs::write(
            &path,
            "not json\n".to_string() + &std::fs::read_to_string(&path).unwrap(),
        )
        .unwrap();
        let back = read_lines(&path);
        assert_eq!(back.len(), 1);
        assert_eq!(back[0]["frame"]["text"], "hi");
        assert!(read_lines(&home.join("missing.jsonl")).is_empty());
    }

    #[test]
    fn cursor_removal_forgets() {
        let (_d, home) = tmp();
        record_in(&home, "s1", "claude-code", "abc");
        record_in(&home, "s1", "codex", "th-9");
        remove_in(&home, "s1", "claude-code");
        assert!(read_in(&home, "s1", "claude-code").is_none());
        // The other driver's cursor survives.
        assert!(read_in(&home, "s1", "codex").is_some());
        remove_in(&home, "missing", "claude-code"); // idempotent
    }

    #[test]
    fn missing_history_dir_lists_nothing() {
        let (_d, home) = tmp();
        assert!(list_claude_sessions_in(&home.join(".claude/projects")).is_empty());
    }
}
