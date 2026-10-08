//! Reading other coding agents' own chat history, so it can be brought into
//! Mira: Claude Code (`~/.claude/projects/*/*.jsonl`) and Codex
//! (`~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`).
//!
//! Only the conversation is taken — what the person asked and what the
//! agent answered. Tool calls, tool output, thinking, side conversations
//! (subagents), injected context and compaction summaries are left out:
//! they're noise when reading a chat back, and the agent still has them
//! when the session is resumed.
//!
//! Reading is defensive throughout. These files are written by other
//! programs, can be gigabytes (screenshots inline), and their formats change
//! between versions: unknown records are skipped, oversized lines aren't
//! parsed, and every scan is capped so a huge history can't stall it.

use std::io::BufRead;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// Lines longer than this aren't parsed: they're inline images or giant
/// tool output, never conversation text worth reading back.
const MAX_LINE_BYTES: usize = 2 * 1024 * 1024;
/// Newest transcripts considered per agent.
const MAX_FILES_PER_SOURCE: usize = 2000;
/// Messages kept per imported chat: the first (what it was about) and the
/// most recent rest.
pub const MAX_MESSAGES: usize = 200;
/// Longest single message kept.
const MAX_MESSAGE_CHARS: usize = 60_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, PartialOrd, Ord, Hash)]
pub enum Source {
    #[serde(rename = "claude-code")]
    ClaudeCode,
    #[serde(rename = "codex")]
    Codex,
}

impl Source {
    /// The driver that resumes it (`mira_acp::drivers`).
    pub fn driver_kind(self) -> &'static str {
        match self {
            Source::ClaudeCode => "claude-code",
            Source::Codex => "codex",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Source::ClaudeCode => "Claude Code",
            Source::Codex => "Codex",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "claude-code" => Some(Source::ClaudeCode),
            "codex" => Some(Source::Codex),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HistoryMessage {
    pub role: Role,
    pub text: String,
    /// Unix milliseconds, when recorded.
    pub at: Option<i64>,
}

/// One chat from another agent's history.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HistorySession {
    pub source: Source,
    /// The agent's own session id — what resumes it.
    pub id: String,
    /// The folder it ran in.
    pub cwd: String,
    /// The agent's own title for it, else the first thing asked.
    pub title: String,
    pub model: Option<String>,
    /// Unix milliseconds.
    pub started_at: Option<i64>,
    pub updated_at: Option<i64>,
    /// Messages in the whole chat (before the import cap).
    pub message_count: usize,
    /// Run by a program rather than a person at the agent (Claude Code's
    /// `sdk-*` entrypoints: test suites, scripts, other apps driving it —
    /// Mira included). Offered, but never picked by default.
    pub automated: bool,
    /// Empty from a scan; filled by [`read_session`].
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<HistoryMessage>,
    #[serde(skip)]
    pub path: PathBuf,
}

/// Where each agent keeps its history, honouring the same overrides the
/// CLIs do.
pub fn source_home(source: Source) -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    match source {
        Source::ClaudeCode => std::env::var_os("CLAUDE_CONFIG_DIR")
            .map(PathBuf::from)
            .or_else(|| home.map(|h| h.join(".claude"))),
        Source::Codex => std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .or_else(|| home.map(|h| h.join(".codex"))),
    }
}

/// One turn's token accounting, read out of an agent's own transcript so
/// the Usage page can show work Mira never drove (and never paid for
/// through its own harness).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct NativeUsageRow {
    pub source: Source,
    /// The model the agent reported for the turn. Codex's rollouts don't
    /// name one per event, so those rows say `unknown-codex` and price as
    /// unknown (tokens still shown).
    pub model: String,
    /// The session file the row came from, so repeat scans can dedupe.
    pub file_id: String,
    /// UTC date, `YYYY-MM-DD`, from the record's own timestamp.
    pub day: String,
    /// Prompt tokens including cached (the convention everywhere else).
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
}

/// Read an agent's own token accounting from its transcript files — the
/// same files a chat import reads, but yielding token counts instead of
/// messages.
///
/// Claude Code rows are turn-scoped already: every `assistant` record
/// carries `message.usage`. Codex reports a running `last_token_usage`
/// per `token_count` event, which is what one turn added.
///
/// Defensive by contract, like the rest of this module: unknown records
/// are skipped, oversized lines are never parsed, error rows Claude writes
/// in place of a reply are dropped, and the scan is capped by
/// [`transcript_files`] so a huge history can't stall it.
pub fn native_usage_rows(source: Source, home: &Path, since_day: &str) -> Vec<NativeUsageRow> {
    let mut out = Vec::new();
    for path in transcript_files(source, home) {
        let file_id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        // Claude Code writes one record per assistant content block, and
        // every one of them repeats the *same* cumulative `message.usage`.
        // Counting them all would inflate a day's tokens several-fold, so
        // each API message id contributes exactly once per file.
        let mut counted: std::collections::HashSet<String> = std::collections::HashSet::new();
        for line in read_capped_lines(&path, MAX_LINE_BYTES) {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
                continue;
            };
            if source == Source::ClaudeCode {
                if let Some(id) = claude_message_id(&v) {
                    if !counted.insert(id) {
                        continue;
                    }
                }
            }
            if let Some(row) = usage_row(source, &v, &file_id, since_day) {
                out.push(row);
            }
        }
    }
    out
}

/// The API message id of an assistant record, when it names one. Records
/// without an id (older writers) are kept, since there is nothing to
/// dedupe them by.
fn claude_message_id(v: &serde_json::Value) -> Option<String> {
    if v.get("type").and_then(|t| t.as_str()) != Some("assistant") {
        return None;
    }
    v.get("message")?.get("id")?.as_str().map(str::to_string)
}

/// One row from one record, or `None` when the record isn't usage (or is
/// usage we can't honestly account for).
fn usage_row(
    source: Source,
    v: &serde_json::Value,
    file_id: &str,
    since_day: &str,
) -> Option<NativeUsageRow> {
    let (model, usage) = match source {
        Source::ClaudeCode => {
            if v.get("type").and_then(|t| t.as_str()) != Some("assistant") {
                return None;
            }
            // Claude writes a synthetic message in place of a reply when
            // the turn failed (rate limit, API error). Its usage is zeros
            // and its model is a placeholder — never a billable turn.
            let message = v.get("message")?;
            let model = message.get("model")?.as_str()?;
            if model == "<synthetic>"
                || v.get("isApiErrorMessage").and_then(|b| b.as_bool()) == Some(true)
            {
                return None;
            }
            (model.to_string(), message.get("usage")?)
        }
        Source::Codex => {
            let p = v.get("payload")?;
            if v.get("type").and_then(|t| t.as_str()) != Some("event_msg")
                || p.get("type").and_then(|t| t.as_str()) != Some("token_count")
            {
                return None;
            }
            // `last_token_usage` is what the last turn added; the sibling
            // `total_token_usage` is a running total and would double-count.
            let last = p.get("info")?.get("last_token_usage")?;
            ("unknown-codex".to_string(), last)
        }
    };
    let day = day_of(v.get("timestamp")?);
    if day.as_str() < since_day {
        return None;
    }
    let (input_key, cached_key, output_key) = match source {
        Source::ClaudeCode => ("input_tokens", "cache_read_input_tokens", "output_tokens"),
        Source::Codex => ("input_tokens", "cached_input_tokens", "output_tokens"),
    };
    let num = |k: &str| usage.get(k).and_then(|n| n.as_u64()).unwrap_or(0);
    // Cache writes and reads both bill as input, and both count as cached.
    let cache_extra = match source {
        Source::ClaudeCode => usage
            .get("cache_creation_input_tokens")
            .and_then(|n| n.as_u64())
            .unwrap_or(0),
        Source::Codex => usage
            .get("cache_write_input_tokens")
            .and_then(|n| n.as_u64())
            .unwrap_or(0),
    };
    let cached = num(cached_key);
    let input = num(input_key) + cached + cache_extra;
    let output = num(output_key)
        + (if source == Source::Codex {
            // Reasoning is billed as output; keep it in the output column
            // rather than dropping it, so the row sums to what was used.
            usage
                .get("reasoning_output_tokens")
                .and_then(|n| n.as_u64())
                .unwrap_or(0)
        } else {
            0
        });
    if input == 0 && output == 0 {
        return None;
    }
    Some(NativeUsageRow {
        source,
        model,
        file_id: file_id.to_string(),
        day,
        input_tokens: input,
        cached_input_tokens: cached,
        output_tokens: output,
    })
}

/// `YYYY-MM-DD` (UTC) for an RFC 3339 timestamp, or empty when it isn't
/// one. An empty day can't be filtered by `since_day` meaningfully, so
/// such rows are treated as out of range rather than invented.
fn day_of(ts: &serde_json::Value) -> String {
    ts.as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.with_timezone(&chrono::Utc).format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

/// Lines of a transcript, skipping any longer than `max_bytes`: those are
/// inline images and giant tool output, never usage.
fn read_capped_lines(path: &Path, max_bytes: usize) -> impl Iterator<Item = String> + '_ {
    let cap = max_bytes;
    std::fs::File::open(path)
        .ok()
        .into_iter()
        .flat_map(move |f| {
            std::io::BufReader::new(f)
                .split(b'\n')
                .map_while(Result::ok)
                .filter(move |b| b.len() <= cap)
                .map(|b| String::from_utf8_lossy(&b).into_owned())
                .collect::<Vec<_>>()
        })
}

/// Read transcript files for a source, newest first, capped.
pub fn transcript_files(source: Source, home: &Path) -> Vec<PathBuf> {
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    let mut push = |p: PathBuf| {
        if let Ok(m) = std::fs::metadata(&p).and_then(|m| m.modified()) {
            files.push((m, p));
        }
    };
    let list = |d: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(d)
            .map(|r| r.flatten().map(|e| e.path()).collect())
            .unwrap_or_default()
    };
    match source {
        Source::ClaudeCode => {
            for dir in list(&home.join("projects")) {
                for f in list(&dir) {
                    if f.extension().is_some_and(|e| e == "jsonl") {
                        push(f);
                    }
                }
            }
        }
        Source::Codex => {
            // sessions/YYYY/MM/DD/rollout-*.jsonl
            for y in list(&home.join("sessions")) {
                for m in list(&y) {
                    for d in list(&m) {
                        for f in list(&d) {
                            let name = f.file_name().and_then(|n| n.to_str()).unwrap_or("");
                            if name.starts_with("rollout-") && name.ends_with(".jsonl") {
                                push(f);
                            }
                        }
                    }
                }
            }
        }
    }
    files.sort_by_key(|f| std::cmp::Reverse(f.0));
    files.truncate(MAX_FILES_PER_SOURCE);
    files.into_iter().map(|(_, p)| p).collect()
}

/// Every chat a source has, newest first, without their messages.
pub fn scan(source: Source, home: &Path) -> Vec<HistorySession> {
    transcript_files(source, home)
        .into_iter()
        .filter_map(|p| parse_file(source, &p, false))
        .collect()
}

/// One chat with its messages, for importing.
pub fn read_session(source: Source, path: &Path) -> Option<HistorySession> {
    parse_file(source, path, true)
}

fn iso_ms(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.timestamp_millis())
}

/// Text from a content field: a string, or blocks of `text` /
/// `input_text` / `output_text` (tool results, images and tool calls are
/// left out).
fn content_text(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(blocks) => blocks
            .iter()
            .filter(|b| {
                matches!(
                    b.get("type")
                        .and_then(|t| t.as_str())
                        .map(str::to_ascii_lowercase)
                        .as_deref(),
                    Some("text" | "input_text" | "output_text")
                )
            })
            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Text the agent or its host injected rather than something said:
/// context blocks, command wrappers, tool traffic flattened to text.
fn is_noise(text: &str) -> bool {
    let t = text.trim_start();
    t.is_empty()
        || t.starts_with("<environment_context")
        || t.starts_with("<user_instructions")
        || t.starts_with("<recommended_plugins")
        || t.starts_with("<app-context")
        || t.starts_with("<skill>")
        || t.starts_with("<local-command")
        || t.starts_with("<command-")
        || t.starts_with("<system-reminder")
        || t.starts_with("<conversation_so_far")
        || t.starts_with("[Request interrupted by user")
        || t.starts_with("<EXTERNAL SESSION IMPORTED>")
        || t.starts_with("[external_agent_tool_call")
        || t.starts_with("[external_agent_tool_result")
        || t.starts_with("Caveat: The messages below were generated")
}

fn clip(mut s: String) -> String {
    if s.chars().count() > MAX_MESSAGE_CHARS {
        s = s.chars().take(MAX_MESSAGE_CHARS).collect::<String>() + "\n\n[…trimmed on import]";
    }
    s
}

/// Collects a chat's messages: consecutive replies from the agent join into
/// one (they're one answer split across steps), and only the first plus the
/// latest `MAX_MESSAGES - 1` are kept when `keep` is set.
struct Collector {
    keep: bool,
    count: usize,
    first_user: Option<HistoryMessage>,
    recent: std::collections::VecDeque<HistoryMessage>,
    last_role: Option<Role>,
}

impl Collector {
    fn new(keep: bool) -> Self {
        Self {
            keep,
            count: 0,
            first_user: None,
            recent: Default::default(),
            last_role: None,
        }
    }

    fn push(&mut self, role: Role, text: String, at: Option<i64>) {
        let text = text.trim().to_string();
        if is_noise(&text) {
            return;
        }
        if role == Role::Assistant && self.last_role == Some(Role::Assistant) {
            if self.keep {
                if let Some(last) = self.recent.back_mut() {
                    last.text = clip(format!("{}\n\n{}", last.text, text));
                }
            }
            return;
        }
        self.count += 1;
        self.last_role = Some(role);
        let msg = HistoryMessage {
            role,
            text: clip(text),
            at,
        };
        if self.first_user.is_none() && role == Role::User {
            self.first_user = Some(msg.clone());
        }
        if self.keep {
            self.recent.push_back(msg);
            if self.recent.len() > MAX_MESSAGES {
                self.recent.pop_front();
            }
        }
    }

    fn messages(self) -> Vec<HistoryMessage> {
        let mut out: Vec<HistoryMessage> = self.recent.into();
        if let Some(first) = self.first_user {
            if out.first() != Some(&first) {
                if out.len() >= MAX_MESSAGES {
                    out.remove(0);
                }
                out.insert(0, first);
            }
        }
        out
    }
}

/// A title from the first thing said: its first sentence, whitespace
/// collapsed, about 80 characters. A short first line ("Hello,") reads on
/// into the next, so a pasted email still gets a useful title.
fn title_from(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let end = flat
        .char_indices()
        .find(|&(i, c)| i >= 20 && matches!(c, '.' | '?' | '!'))
        .map(|(i, _)| i + 1)
        .unwrap_or(flat.len());
    let sentence = &flat[..end];
    if sentence.chars().count() <= 80 {
        sentence.to_string()
    } else {
        let cut: String = sentence.chars().take(79).collect();
        // At a word boundary when there is one.
        match cut.rfind(' ') {
            Some(i) if i > 40 => format!("{}…", &cut[..i]),
            _ => format!("{cut}…"),
        }
    }
}

fn parse_file(source: Source, path: &Path, keep: bool) -> Option<HistorySession> {
    let file = std::fs::File::open(path).ok()?;
    let reader = std::io::BufReader::new(file);
    let mut c = Collector::new(keep);
    let mut id = String::new();
    let mut cwd = String::new();
    let mut title: Option<String> = None;
    let mut model: Option<String> = None;
    let mut automated = false;
    let (mut started, mut updated): (Option<i64>, Option<i64>) = (None, None);
    // Codex (newer) records each message twice: as a response item and as a
    // completed item. Use the completed items when there are any.
    let mut codex_items = false;
    let mut codex_fallback: Vec<(Role, String, Option<i64>)> = Vec::new();

    for line in reader.split(b'\n') {
        let Ok(line) = line else { break };
        if line.len() > MAX_LINE_BYTES || line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(&line) else {
            continue;
        };
        let at = v.get("timestamp").and_then(|t| t.as_str()).and_then(iso_ms);
        if let Some(t) = at {
            started = Some(started.map_or(t, |s| s.min(t)));
            updated = Some(updated.map_or(t, |u| u.max(t)));
        }
        let kind = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
        match source {
            Source::ClaudeCode => {
                if let Some(s) = v.get("sessionId").and_then(|s| s.as_str()) {
                    id = s.to_string();
                }
                if let Some(d) = v.get("cwd").and_then(|s| s.as_str()) {
                    cwd = d.to_string();
                }
                if let Some(t) = v.get("aiTitle").and_then(|s| s.as_str()) {
                    title = Some(t.to_string());
                }
                if let Some(e) = v.get("entrypoint").and_then(|s| s.as_str()) {
                    automated = e.starts_with("sdk");
                }
                if kind == "summary" {
                    if let Some(t) = v.get("summary").and_then(|s| s.as_str()) {
                        title.get_or_insert_with(|| t.to_string());
                    }
                    continue;
                }
                let skip = ["isSidechain", "isMeta", "isCompactSummary"]
                    .iter()
                    .any(|k| v.get(*k).and_then(|b| b.as_bool()) == Some(true));
                if skip || !(kind == "user" || kind == "assistant") {
                    continue;
                }
                let msg = v.get("message").cloned().unwrap_or_default();
                if kind == "assistant" {
                    if let Some(m) = msg.get("model").and_then(|m| m.as_str()) {
                        if !m.starts_with('<') {
                            model = Some(m.to_string());
                        }
                    }
                }
                let text = content_text(msg.get("content").unwrap_or(&serde_json::Value::Null));
                let role = if kind == "user" {
                    Role::User
                } else {
                    Role::Assistant
                };
                c.push(role, text, at);
            }
            Source::Codex => {
                let p = v.get("payload").cloned().unwrap_or_default();
                let pt = p.get("type").and_then(|t| t.as_str()).unwrap_or("");
                match (kind, pt) {
                    ("session_meta", _) => {
                        if let Some(s) =
                            p.get("id").or(p.get("session_id")).and_then(|s| s.as_str())
                        {
                            if id.is_empty() {
                                id = s.to_string();
                            }
                        }
                        if let Some(d) = p.get("cwd").and_then(|s| s.as_str()) {
                            cwd = d.to_string();
                        }
                    }
                    ("turn_context", _) => {
                        if let Some(m) = p.get("model").and_then(|m| m.as_str()) {
                            model = Some(m.to_string());
                        }
                        if cwd.is_empty() {
                            if let Some(d) = p.get("cwd").and_then(|s| s.as_str()) {
                                cwd = d.to_string();
                            }
                        }
                    }
                    ("event_msg", "item_completed") => {
                        let item = p.get("item").cloned().unwrap_or_default();
                        let role = match item.get("type").and_then(|t| t.as_str()) {
                            Some("UserMessage") => Role::User,
                            Some("AgentMessage") => Role::Assistant,
                            _ => continue,
                        };
                        codex_items = true;
                        c.push(
                            role,
                            content_text(item.get("content").unwrap_or(&serde_json::Value::Null)),
                            at,
                        );
                    }
                    // Older logs: the prompt as an event, answers as response items.
                    ("event_msg", "user_message") => {
                        if let Some(m) = p.get("message").and_then(|m| m.as_str()) {
                            codex_fallback.push((Role::User, m.to_string(), at));
                        }
                    }
                    ("response_item", "message")
                        if p.get("role").and_then(|r| r.as_str()) == Some("assistant") =>
                    {
                        codex_fallback.push((
                            Role::Assistant,
                            content_text(p.get("content").unwrap_or(&serde_json::Value::Null)),
                            at,
                        ));
                    }
                    _ => {}
                }
            }
        }
    }
    if source == Source::Codex && !codex_items {
        for (role, text, at) in codex_fallback {
            c.push(role, text, at);
        }
    }
    if id.is_empty() {
        // Claude Code names the file after the session.
        id = path.file_stem()?.to_str()?.to_string();
    }
    let first = c.first_user.as_ref().map(|m| m.text.clone())?;
    let count = c.count;
    let title = title
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| title_from(&first));
    Some(HistorySession {
        source,
        id,
        cwd,
        title: title.trim().to_string(),
        model,
        started_at: started,
        updated_at: updated,
        message_count: count,
        automated,
        messages: c.messages(),
        path: path.to_path_buf(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, rel: &str, lines: &[serde_json::Value]) -> PathBuf {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        let body: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
        std::fs::write(&p, body.join("\n")).unwrap();
        p
    }

    fn claude_usage_row(
        ts: &str,
        model: &str,
        inp: u64,
        out: u64,
        cache_read: u64,
    ) -> serde_json::Value {
        serde_json::json!({
            "type": "assistant",
            "timestamp": ts,
            "isSidechain": false,
            "message": {
                "role": "assistant",
                "model": model,
                "usage": {
                    "input_tokens": inp,
                    "output_tokens": out,
                    "cache_read_input_tokens": cache_read,
                    "cache_creation_input_tokens": 0,
                },
            },
        })
    }

    #[test]
    fn claude_usage_reads_every_turn_and_drops_the_placeholder() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path();
        write(
            home,
            "projects/p/s.jsonl",
            &[
                claude_usage_row("2026-10-04T12:00:00Z", "claude-sonnet-4-6", 10, 100, 0),
                claude_usage_row("2026-10-04T12:05:00Z", "claude-sonnet-4-6", 4, 40, 900),
                // Placeholder reply (rate limit / API error): not billable.
                claude_usage_row("2026-10-04T12:06:00Z", "<synthetic>", 0, 0, 0),
                // Outside the window.
                claude_usage_row("2026-01-01T00:00:00Z", "claude-sonnet-4-6", 9, 9, 0),
            ],
        );
        let rows = native_usage_rows(Source::ClaudeCode, home, "2026-10-01");
        assert_eq!(rows.len(), 2, "got {rows:?}");
        let r = &rows[0];
        assert_eq!(r.model, "claude-sonnet-4-6");
        assert_eq!(r.file_id, "s");
        assert_eq!(r.day, "2026-10-04");
        // Cached reads count as prompt input and as cached.
        assert_eq!(r.input_tokens, 10);
        assert_eq!(r.cached_input_tokens, 0);
        let r2 = &rows[1];
        assert_eq!(r2.input_tokens, 904, "fresh + cached");
        assert_eq!(r2.cached_input_tokens, 900);
        assert_eq!(r2.output_tokens, 40);
    }

    #[test]
    fn codex_usage_uses_the_per_turn_counter_not_the_running_total() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path();
        write(
            home,
            "sessions/2026/10/04/rollout-x.jsonl",
            &[
                serde_json::json!({
                    "timestamp": "2026-10-04T09:00:00Z",
                    "type": "event_msg",
                    "payload": {
                        "type": "token_count",
                        "info": {
                            "total_token_usage": {"input_tokens": 200, "output_tokens": 20},
                            "last_token_usage": {
                                "input_tokens": 200,
                                "cached_input_tokens": 150,
                                "cache_write_input_tokens": 10,
                                "output_tokens": 20,
                                "reasoning_output_tokens": 5,
                            },
                        },
                    },
                }),
                // A null info (nothing counted yet) is not a row.
                serde_json::json!({
                    "timestamp": "2026-10-04T09:00:01Z",
                    "type": "event_msg",
                    "payload": {"type": "token_count", "info": null},
                }),
            ],
        );
        let rows = native_usage_rows(Source::Codex, home, "2026-10-01");
        assert_eq!(rows.len(), 1, "got {rows:?}");
        let r = &rows[0];
        assert_eq!(r.file_id, "rollout-x");
        // Cached reads and writes both bill as input; reasoning bills as output.
        assert_eq!(r.input_tokens, 200 + 150 + 10);
        assert_eq!(r.cached_input_tokens, 150);
        assert_eq!(r.output_tokens, 25);
    }

    #[test]
    fn claude_conversation_is_read_without_noise() {
        use serde_json::json;
        let d = tempfile::tempdir().unwrap();
        let p = write(
            d.path(),
            "projects/-x/abc.jsonl",
            &[
                json!({"type":"user","sessionId":"abc","cwd":"/work/app","timestamp":"2026-09-01T10:00:00Z",
                   "message":{"role":"user","content":"<local-command-caveat>Caveat</local-command-caveat>"}}),
                json!({"type":"user","sessionId":"abc","cwd":"/work/app","timestamp":"2026-09-01T10:00:01Z",
                   "message":{"role":"user","content":[{"type":"text","text":"Fix the login bug"}]}}),
                json!({"type":"assistant","timestamp":"2026-09-01T10:00:05Z",
                   "message":{"model":"claude-opus-4-5","content":[{"type":"thinking","thinking":"hmm"},{"type":"text","text":"Looking at auth.ts."},{"type":"tool_use","name":"Read"}]}}),
                json!({"type":"user","timestamp":"2026-09-01T10:00:06Z",
                   "message":{"content":[{"type":"tool_result","content":"file text"}]}}),
                json!({"type":"assistant","timestamp":"2026-09-01T10:00:09Z",
                   "message":{"model":"<synthetic>","content":[{"type":"text","text":"Fixed it."}]}}),
                json!({"type":"assistant","isSidechain":true,"message":{"content":[{"type":"text","text":"subagent chatter"}]}}),
                json!({"type":"user","isCompactSummary":true,"message":{"content":"summary of before"}}),
                json!({"type":"summary","summary":"Login bug fix"}),
            ],
        );
        let s = read_session(Source::ClaudeCode, &p).unwrap();
        assert_eq!(
            (s.id.as_str(), s.cwd.as_str(), s.title.as_str()),
            ("abc", "/work/app", "Login bug fix")
        );
        assert_eq!(
            s.model.as_deref(),
            Some("claude-opus-4-5"),
            "synthetic isn't a model"
        );
        let got: Vec<_> = s
            .messages
            .iter()
            .map(|m| (m.role, m.text.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                (Role::User, "Fix the login bug"),
                (Role::Assistant, "Looking at auth.ts.\n\nFixed it.")
            ]
        );
        assert_eq!(s.message_count, 2);
        assert!(!s.automated);
    }

    #[test]
    fn scripted_claude_runs_are_flagged() {
        use serde_json::json;
        let d = tempfile::tempdir().unwrap();
        let p = write(
            d.path(),
            "projects/-x/t.jsonl",
            &[
                json!({"type":"user","entrypoint":"sdk-cli","sessionId":"t","cwd":"/w","message":{"content":"Reply with exactly: PONG"}}),
                json!({"type":"assistant","entrypoint":"sdk-cli","message":{"content":[{"type":"text","text":"PONG"}]}}),
            ],
        );
        assert!(read_session(Source::ClaudeCode, &p).unwrap().automated);
    }

    #[test]
    fn codex_items_win_and_tool_text_is_dropped() {
        use serde_json::json;
        let d = tempfile::tempdir().unwrap();
        let p = write(
            d.path(),
            "sessions/2026/09/07/rollout-2026-09-07T20-47-12-x.jsonl",
            &[
                json!({"type":"session_meta","timestamp":"2026-09-07T20:47:12Z","payload":{"id":"th-1","cwd":"/work/luno"}}),
                json!({"type":"turn_context","payload":{"model":"gpt-5-codex","cwd":"/work/luno"}}),
                json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context> cwd </environment_context>"}]}}),
                json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Fix the paywall"}]}}),
                json!({"type":"event_msg","timestamp":"2026-09-07T20:47:13Z","payload":{"type":"item_completed","item":{"type":"UserMessage","content":[{"type":"text","text":"Fix the paywall"}]}}}),
                json!({"type":"event_msg","payload":{"type":"item_completed","item":{"type":"AgentMessage","content":[{"type":"Text","text":"Let me look."}]}}}),
                json!({"type":"event_msg","payload":{"type":"item_completed","item":{"type":"AgentMessage","content":[{"type":"Text","text":"[external_agent_tool_call: Bash]"}]}}}),
                json!({"type":"event_msg","payload":{"type":"item_completed","item":{"type":"Reasoning","summary_text":["thinking"]}}}),
                json!({"type":"event_msg","payload":{"type":"item_completed","item":{"type":"AgentMessage","content":[{"type":"Text","text":"Done — the paywall shows."}]}}}),
                json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Done — the paywall shows."}]}}),
            ],
        );
        let s = read_session(Source::Codex, &p).unwrap();
        assert_eq!(
            (s.id.as_str(), s.cwd.as_str(), s.model.as_deref()),
            ("th-1", "/work/luno", Some("gpt-5-codex"))
        );
        let got: Vec<_> = s
            .messages
            .iter()
            .map(|m| (m.role, m.text.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                (Role::User, "Fix the paywall"),
                (Role::Assistant, "Let me look.\n\nDone — the paywall shows.")
            ]
        );
        assert_eq!(s.title, "Fix the paywall");
    }

    #[test]
    fn older_codex_logs_use_events_and_response_items() {
        use serde_json::json;
        let d = tempfile::tempdir().unwrap();
        let p = write(
            d.path(),
            "sessions/2025/01/02/rollout-old.jsonl",
            &[
                json!({"type":"session_meta","payload":{"id":"th-old","cwd":"/w"}}),
                json!({"type":"event_msg","payload":{"type":"user_message","message":"hello"}}),
                json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"hi there"}]}}),
            ],
        );
        let s = read_session(Source::Codex, &p).unwrap();
        assert_eq!(s.messages.len(), 2);
        assert_eq!(s.messages[1].text, "hi there");
    }

    #[test]
    fn titles_read_like_a_title() {
        assert_eq!(
            title_from(
                "Hello,\n\nThank you for submitting an update to the app. We found an issue."
            ),
            "Hello, Thank you for submitting an update to the app."
        );
        assert_eq!(title_from("fix the login bug"), "fix the login bug");
        let long = "word ".repeat(40);
        let t = title_from(&long);
        assert!(t.ends_with('…') && t.chars().count() <= 80, "{t}");
    }

    #[test]
    fn long_chats_keep_the_first_message_and_the_latest() {
        let mut c = Collector::new(true);
        for i in 0..(MAX_MESSAGES + 50) {
            let role = if i % 2 == 0 {
                Role::User
            } else {
                Role::Assistant
            };
            c.push(role, format!("m{i}"), None);
        }
        assert_eq!(c.count, MAX_MESSAGES + 50);
        let m = c.messages();
        assert_eq!(m.len(), MAX_MESSAGES);
        assert_eq!(m[0].text, "m0");
        assert_eq!(m.last().unwrap().text, format!("m{}", MAX_MESSAGES + 49));
    }

    #[test]
    fn a_chat_with_nothing_said_is_skipped() {
        use serde_json::json;
        let d = tempfile::tempdir().unwrap();
        let p = write(
            d.path(),
            "projects/-x/empty.jsonl",
            &[json!({"type":"user","message":{"content":"<command-name>/clear</command-name>"}})],
        );
        assert!(read_session(Source::ClaudeCode, &p).is_none());
        // Discovery finds Claude's files by their layout.
        let files = transcript_files(Source::ClaudeCode, d.path());
        assert_eq!(files.len(), 1);
    }
}

/// Reads the machine's real history and prints a summary. Read-only; run by
/// hand: `cargo test -p mira-acp -- --ignored real_history --nocapture`.
#[cfg(test)]
#[test]
#[ignore]
fn real_history() {
    for source in [Source::ClaudeCode, Source::Codex] {
        let Some(home) = source_home(source) else {
            continue;
        };
        let t = std::time::Instant::now();
        let all = scan(source, &home);
        println!(
            "{}: {} chats in {:?}",
            source.label(),
            all.len(),
            t.elapsed()
        );
        for s in all.iter().take(4) {
            println!(
                "  {:<60} {:>4} msgs  {}",
                s.title.chars().take(58).collect::<String>(),
                s.message_count,
                s.cwd
            );
        }
        if let Some(s) = all.first() {
            let full = read_session(source, &s.path).unwrap();
            println!(
                "  first chat: {} kept messages, first: {:?}",
                full.messages.len(),
                full.messages[0].text.chars().take(70).collect::<String>()
            );
        }
    }
}
