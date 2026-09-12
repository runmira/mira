//! Agent-facing tools for reading and mutating `MIRA.md`.
//!
//! Four surfaces, all keyed off a `scope` (`"user"` = `~/.mira/MIRA.md`,
//! `"project"` = `<cwd>/.mira/MIRA.md`):
//!
//! - `memory_read`   — dump the current contents
//! - `memory_search` — grep for a substring, line-anchored hits
//! - `memory_append` — add a bullet (multi-line becomes indented continuations)
//! - `memory_edit`   — targeted find-and-replace, empty replacement = delete
//!
//! Every write goes through the shared [`MemoryStore`] handle on
//! [`ToolContext`], so this path shares per-scope locking with the HTTP
//! `/api/memory/append` endpoint and any future writer. Missing handle
//! (`ctx.memory == None`) surfaces as a clear `Failed` — meant to catch
//! misconfiguration in dev / tests rather than being a routine error.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use mira_ai::{ChatProvider, ToolSpec};
use mira_core::{ToolCall, ToolResult};
use mira_memory::{search, EpisodicEntry, EpisodicSource, MemoryError, MemoryScope};
use serde::Deserialize;
use serde_json::json;

use crate::consolidate::consolidate_bullets;
use crate::context::ToolContext;
use crate::tool::{spec, Action, Tool, ToolError};

const SCOPE_DESC: &str = "Which memory file to target. `user` = ~/.mira/MIRA.md \
(global preferences, applies everywhere). `project` = <cwd>/.mira/MIRA.md \
(rules and facts scoped to the current repo).";

#[derive(Deserialize)]
struct ScopeArg {
    scope: WireScope,
}

#[derive(Deserialize)]
struct SearchArgs {
    scope: WireScope,
    query: String,
}

#[derive(Deserialize)]
struct AppendArgs {
    scope: WireScope,
    text: String,
}

#[derive(Deserialize)]
struct EditArgs {
    scope: WireScope,
    old: String,
    #[serde(default)]
    new: String,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum WireScope {
    User,
    Project,
}

impl From<WireScope> for MemoryScope {
    fn from(w: WireScope) -> Self {
        match w {
            WireScope::User => MemoryScope::User,
            WireScope::Project => MemoryScope::Project,
        }
    }
}

fn store<'a>(ctx: &'a ToolContext) -> Result<&'a std::sync::Arc<dyn mira_memory::MemoryStore>, ToolError> {
    ctx.memory
        .as_ref()
        .ok_or_else(|| ToolError::Failed("memory store not wired into ToolContext".into()))
}

fn map_err(e: MemoryError) -> ToolError {
    ToolError::Failed(e.to_string())
}

// ---------- read ----------

pub struct MemoryRead;

#[async_trait]
impl Tool for MemoryRead {
    fn spec(&self) -> ToolSpec {
        spec(
            "memory_read",
            "Read the current contents of a memory file (user or project MIRA.md). \
             Missing files come back as an empty string — that's normal for a fresh \
             repo or a user who hasn't set global memory yet.",
            json!({
                "type": "object",
                "properties": {
                    "scope": { "type": "string", "enum": ["user", "project"], "description": SCOPE_DESC }
                },
                "required": ["scope"],
                "additionalProperties": false
            }),
        )
    }

    fn action(&self) -> Action {
        Action::Pure
    }

    async fn invoke(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let args: ScopeArg = call.parse_arguments()?;
        let body = store(ctx)?.read(args.scope.into()).await.map_err(map_err)?;
        Ok(ToolResult::ok(call.id.clone(), body))
    }
}

// ---------- search ----------

pub struct MemorySearch;

#[async_trait]
impl Tool for MemorySearch {
    fn spec(&self) -> ToolSpec {
        spec(
            "memory_search",
            "Case-insensitive substring search over a memory file. Returns matching \
             lines with their 1-based line numbers, one per line. Use this instead of \
             `memory_read` when you just want to check whether the user has recorded \
             something on a specific topic.",
            json!({
                "type": "object",
                "properties": {
                    "scope": { "type": "string", "enum": ["user", "project"], "description": SCOPE_DESC },
                    "query": { "type": "string", "description": "Substring to search for. Case-insensitive." }
                },
                "required": ["scope", "query"],
                "additionalProperties": false
            }),
        )
    }

    fn action(&self) -> Action {
        Action::Pure
    }

    async fn invoke(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let args: SearchArgs = call.parse_arguments()?;
        let hits = search(&**store(ctx)?, args.scope.into(), &args.query)
            .await
            .map_err(map_err)?;
        let body = if hits.is_empty() {
            format!("no matches for {:?}", args.query)
        } else {
            hits.into_iter()
                .map(|m| format!("{:>4}: {}", m.line_no, m.line))
                .collect::<Vec<_>>()
                .join("\n")
        };
        Ok(ToolResult::ok(call.id.clone(), body))
    }
}

// ---------- append ----------

pub struct MemoryAppend;

#[async_trait]
impl Tool for MemoryAppend {
    fn spec(&self) -> ToolSpec {
        spec(
            "memory_append",
            "Add a bullet to a memory file. Multi-line text becomes one bullet with \
             continuation lines indented two spaces. Use for durable facts the user \
             would want you to remember — preferences, project conventions, gotchas \
             the code doesn't document. Ephemeral session state does not belong here.",
            json!({
                "type": "object",
                "properties": {
                    "scope": { "type": "string", "enum": ["user", "project"], "description": SCOPE_DESC },
                    "text":  { "type": "string", "description": "The fact or note to remember. Keep it concise; one idea per call." }
                },
                "required": ["scope", "text"],
                "additionalProperties": false
            }),
        )
    }

    fn action(&self) -> Action {
        Action::Pure
    }

    async fn invoke(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let args: AppendArgs = call.parse_arguments()?;
        let scope: MemoryScope = args.scope.into();
        let s = store(ctx)?;
        let bytes = s.append(scope, &args.text).await.map_err(map_err)?;
        let path = s.path(scope);
        Ok(ToolResult::ok(
            call.id.clone(),
            format!("appended to {} ({} bytes)", path.display(), bytes),
        ))
    }
}

// ---------- edit ----------

pub struct MemoryEdit;

#[async_trait]
impl Tool for MemoryEdit {
    fn spec(&self) -> ToolSpec {
        spec(
            "memory_edit",
            "Replace exactly one occurrence of `old` with `new` in a memory file. \
             Errors if `old` is missing or appears more than once — widen the anchor \
             with surrounding text to make it unique, same rule as `edit_file`. \
             Setting `new` to an empty string deletes the matched fragment (drop a \
             leading `- ` and trailing newline if you want the whole bullet gone).",
            json!({
                "type": "object",
                "properties": {
                    "scope": { "type": "string", "enum": ["user", "project"], "description": SCOPE_DESC },
                    "old":   { "type": "string", "description": "Exact fragment to find. Must be unique in the file." },
                    "new":   { "type": "string", "description": "Replacement. Empty string deletes.", "default": "" }
                },
                "required": ["scope", "old"],
                "additionalProperties": false
            }),
        )
    }

    fn action(&self) -> Action {
        Action::Pure
    }

    async fn invoke(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let args: EditArgs = call.parse_arguments()?;
        let scope: MemoryScope = args.scope.into();
        let s = store(ctx)?;
        let bytes = s
            .replace(scope, &args.old, &args.new)
            .await
            .map_err(map_err)?;
        let path = s.path(scope);
        Ok(ToolResult::ok(
            call.id.clone(),
            format!("updated {} ({} bytes)", path.display(), bytes),
        ))
    }
}

// ---------- remember (cross-session episodic) ----------

#[derive(Deserialize)]
struct RememberArgs {
    text: String,
}

pub struct MemoryRemember;

#[async_trait]
impl Tool for MemoryRemember {
    fn spec(&self) -> ToolSpec {
        spec(
            "memory_remember",
            "Record a durable fact about this project that should survive into \
             FUTURE sessions. Use for insights, decisions, patterns, or gotchas \
             you discovered that a future you (or the user) would want to \
             reference next time — 'we use pnpm not npm', 'the test suite is \
             flaky on macOS without SKIP_LINT=1', 'auth is handled by the \
             `session` middleware, not the JWT one'. \
             \
             DO NOT use for ephemeral turn state (current file, current TODO), \
             for facts the code already documents, or for information already \
             in MIRA.md. Entries are appended to `.mira/episodic.jsonl` and \
             rendered into every future session's system prompt.",
            json!({
                "type": "object",
                "properties": {
                    "text": {
                        "type": "string",
                        "description": "The fact to remember. One idea per call; keep it concise and self-contained (a future session sees this line without context)."
                    }
                },
                "required": ["text"],
                "additionalProperties": false
            }),
        )
    }

    fn action(&self) -> Action {
        Action::Pure
    }

    async fn invoke(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let args: RememberArgs = call.parse_arguments()?;
        let episodic = ctx
            .episodic
            .as_ref()
            .ok_or_else(|| ToolError::Failed("episodic store not wired into ToolContext".into()))?;
        let mut entry = EpisodicEntry::now(args.text, EpisodicSource::Tool);
        if let Some(sid) = ctx.session_id.as_ref() {
            entry = entry.with_session_id(sid.to_string());
        }
        episodic
            .append(entry)
            .await
            .map_err(|e| ToolError::Failed(e.to_string()))?;
        Ok(ToolResult::ok(
            call.id.clone(),
            format!("remembered ({})", episodic.path().display()),
        ))
    }
}

// ---------- consolidate (dedup + merge via cheap model) ----------

/// Agent-facing tool that dedupes / merges / cleans up a memory file by
/// calling a cheap model to summarise the current content, then swaps
/// the file's contents with the model's reply. The original is backed
/// up to a `.pre-consolidate-<unix_ts>.bak` next to the source so a bad
/// consolidation is a rename away from reverting.
///
/// Constructed with a `ChatProvider` handle + model name at server
/// boot — same pattern as `AgentTool`. The tool wouldn't otherwise have
/// a way to reach a provider through `ToolContext`.
pub struct MemoryConsolidate {
    provider: Arc<dyn ChatProvider>,
    model: String,
}

impl MemoryConsolidate {
    pub fn new(provider: Arc<dyn ChatProvider>, model: String) -> Self {
        Self { provider, model }
    }
}

#[derive(Deserialize)]
struct ConsolidateArgs {
    scope: ConsolidateWireScope,
    /// Optional free-text nudge the model reads alongside its system
    /// prompt — e.g. "be aggressive about pruning", "group by topic",
    /// "keep every entry mentioning caching". Empty = default rules.
    #[serde(default)]
    instruction: Option<String>,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum ConsolidateWireScope {
    User,
    Project,
    Episodic,
}

#[async_trait]
impl Tool for MemoryConsolidate {
    fn spec(&self) -> ToolSpec {
        spec(
            "memory_consolidate",
            "Consolidate a memory file — dedup, merge, and clean up entries \
             by calling a cheap model to summarise the current content. The \
             model preserves every distinct fact but merges duplicates, \
             resolves contradictions, and drops noise. The original content \
             is saved to a `.pre-consolidate-<timestamp>.bak` file next to \
             the source so you can revert. \
             \
             Use ONLY when the user asks (`/memory consolidate`, \"clean up \
             memory\", \"the MIRA.md is getting messy\"). This costs a \
             provider call and rewrites a durable file — never run it \
             unprompted.",
            json!({
                "type": "object",
                "properties": {
                    "scope": {
                        "type": "string",
                        "enum": ["user", "project", "episodic"],
                        "description": "Which store to consolidate. `user` = ~/.mira/MIRA.md, `project` = <cwd>/.mira/MIRA.md, `episodic` = <cwd>/.mira/episodic.jsonl."
                    },
                    "instruction": {
                        "type": "string",
                        "description": "Optional free-text nudge threaded into the consolidator's system prompt (e.g., 'group by topic')."
                    }
                },
                "required": ["scope"],
                "additionalProperties": false
            }),
        )
    }

    fn action(&self) -> Action {
        // Rewrites a file on disk — surface as `Write` so the same
        // permission-gate the write tools use applies here.
        Action::Write
    }

    async fn invoke(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let args: ConsolidateArgs = call.parse_arguments()?;
        match args.scope {
            ConsolidateWireScope::User | ConsolidateWireScope::Project => {
                let scope = match args.scope {
                    ConsolidateWireScope::User => MemoryScope::User,
                    ConsolidateWireScope::Project => MemoryScope::Project,
                    _ => unreachable!(),
                };
                self.consolidate_md(call, ctx, scope, args.instruction.as_deref())
                    .await
            }
            ConsolidateWireScope::Episodic => {
                self.consolidate_episodic(call, ctx, args.instruction.as_deref())
                    .await
            }
        }
    }
}

impl MemoryConsolidate {
    async fn consolidate_md(
        &self,
        call: &ToolCall,
        ctx: &ToolContext,
        scope: MemoryScope,
        instruction: Option<&str>,
    ) -> Result<ToolResult, ToolError> {
        let s = store(ctx)?;
        let path = s.path(scope);
        let current = s.read(scope).await.map_err(map_err)?;
        let before_bytes = current.len();
        if current.trim().is_empty() {
            return Ok(ToolResult::err(
                call.id.clone(),
                format!("consolidate: {} is empty; nothing to do", path.display()),
            ));
        }

        let consolidated = consolidate_bullets(
            self.provider.as_ref(),
            &self.model,
            &current,
            instruction,
        )
        .await
        .map_err(|e| ToolError::Failed(format!("consolidate: {e}")))?;

        // Backup — `.pre-consolidate-<unix_ts>.bak` next to the source.
        // Skip the backup silently on IO error rather than failing the
        // whole call; the source is still on disk in the store until we
        // overwrite it, so the worst case is "no backup" not "no memory".
        let backup_path = backup_path_for(&path);
        if let Err(e) = std::fs::write(&backup_path, current.as_bytes()) {
            tracing::warn!(?e, backup = %backup_path.display(), "consolidate: backup write failed");
        }

        let after_bytes = s
            .overwrite(scope, &consolidated)
            .await
            .map_err(map_err)?;

        Ok(ToolResult::ok(
            call.id.clone(),
            format!(
                "consolidated {} · {} → {} bytes · backup at {}",
                path.display(),
                before_bytes,
                after_bytes,
                backup_path.display(),
            ),
        ))
    }

    async fn consolidate_episodic(
        &self,
        call: &ToolCall,
        ctx: &ToolContext,
        instruction: Option<&str>,
    ) -> Result<ToolResult, ToolError> {
        let episodic = ctx
            .episodic
            .as_ref()
            .ok_or_else(|| ToolError::Failed("episodic store not wired into ToolContext".into()))?;
        let path = episodic.path();
        let entries = episodic
            .recent(usize::MAX)
            .await
            .map_err(|e| ToolError::Failed(e.to_string()))?;
        if entries.is_empty() {
            return Ok(ToolResult::err(
                call.id.clone(),
                format!("consolidate: {} is empty; nothing to do", path.display()),
            ));
        }

        let before_count = entries.len();
        let serialised = entries
            .iter()
            .map(|e| format!("- {}", e.text.trim()))
            .collect::<Vec<_>>()
            .join("\n");

        let consolidated = consolidate_bullets(
            self.provider.as_ref(),
            &self.model,
            &serialised,
            instruction,
        )
        .await
        .map_err(|e| ToolError::Failed(format!("consolidate: {e}")))?;

        // Backup the raw JSONL before overwriting.
        let backup_path = backup_path_for(&path);
        if let Ok(current) = std::fs::read_to_string(&path) {
            if let Err(e) = std::fs::write(&backup_path, current.as_bytes()) {
                tracing::warn!(?e, backup = %backup_path.display(), "consolidate: backup write failed");
            }
        }

        // Parse the consolidated bullet list back into episodic entries.
        // Multi-line thoughts (indented continuations) are folded into
        // the parent bullet so one bullet = one entry.
        let now = now_secs();
        let new_entries: Vec<EpisodicEntry> = parse_bullets(&consolidated)
            .into_iter()
            .map(|text| EpisodicEntry {
                text,
                timestamp: now,
                session_id: None,
                source: EpisodicSource::Auto,
            })
            .collect();
        let after_count = new_entries.len();

        episodic
            .overwrite_all(new_entries)
            .await
            .map_err(|e| ToolError::Failed(e.to_string()))?;

        Ok(ToolResult::ok(
            call.id.clone(),
            format!(
                "consolidated {} · {} → {} entries · backup at {}",
                path.display(),
                before_count,
                after_count,
                backup_path.display(),
            ),
        ))
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn backup_path_for(path: &std::path::Path) -> std::path::PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(format!(".pre-consolidate-{}.bak", now_secs()));
    match path.parent() {
        Some(p) => p.join(name),
        None => std::path::PathBuf::from(name),
    }
}

/// Fold a Markdown bullet list into one `String` per bullet. Indented
/// continuation lines join back into the parent bullet with a space
/// separator; blank lines close the current bullet without opening a
/// new one.
fn parse_bullets(content: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current: Option<String> = None;
    for line in content.lines() {
        let ts = line.trim_start();
        if ts.starts_with("- ") || ts.starts_with("* ") {
            if let Some(prev) = current.take() {
                push_nonempty(&mut out, prev);
            }
            current = Some(ts[2..].trim().to_string());
            continue;
        }
        if current.is_some() && (line.starts_with("  ") || line.starts_with('\t')) {
            if let Some(buf) = current.as_mut() {
                let piece = ts.trim();
                if !piece.is_empty() {
                    if !buf.is_empty() {
                        buf.push(' ');
                    }
                    buf.push_str(piece);
                }
            }
            continue;
        }
        if ts.is_empty() {
            if let Some(prev) = current.take() {
                push_nonempty(&mut out, prev);
            }
        }
    }
    if let Some(prev) = current.take() {
        push_nonempty(&mut out, prev);
    }
    out
}

fn push_nonempty(out: &mut Vec<String>, s: String) {
    let t = s.trim().to_string();
    if !t.is_empty() {
        out.push(t);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mira_core::message::{ToolCallFunction, ToolCallKind};
    use mira_core::{ToolCall, ToolCallId};
    use mira_memory::FileMemoryStore;
    use mira_sandbox::Sandbox;
    use std::sync::Arc;
    use tempfile::tempdir;

    fn call(name: &str, args: serde_json::Value) -> ToolCall {
        ToolCall {
            id: ToolCallId::from("test-1"),
            kind: ToolCallKind::Function,
            function: ToolCallFunction {
                name: name.into(),
                arguments: args.to_string(),
            },
        }
    }

    fn ctx_with_store(tmp: &std::path::Path) -> ToolContext {
        let store: Arc<dyn mira_memory::MemoryStore> = Arc::new(FileMemoryStore::new(
            tmp.join("user.md"),
            tmp.join("project.md"),
        ));
        ToolContext::new(tmp, Arc::new(Sandbox::default_scrubbed())).with_memory(store)
    }

    #[tokio::test]
    async fn append_then_read_roundtrips() {
        let tmp = tempdir().unwrap();
        let ctx = ctx_with_store(tmp.path());

        MemoryAppend
            .invoke(
                &call("memory_append", json!({"scope": "project", "text": "hello"})),
                &ctx,
            )
            .await
            .unwrap();

        let r = MemoryRead
            .invoke(
                &call("memory_read", json!({"scope": "project"})),
                &ctx,
            )
            .await
            .unwrap();
        assert!(r.content.contains("- hello"));
    }

    #[tokio::test]
    async fn search_returns_line_hits() {
        let tmp = tempdir().unwrap();
        let ctx = ctx_with_store(tmp.path());
        for line in ["alpha thing", "beta thing", "gamma"] {
            MemoryAppend
                .invoke(
                    &call("memory_append", json!({"scope": "user", "text": line})),
                    &ctx,
                )
                .await
                .unwrap();
        }
        let r = MemorySearch
            .invoke(
                &call("memory_search", json!({"scope": "user", "query": "thing"})),
                &ctx,
            )
            .await
            .unwrap();
        assert!(r.content.contains("alpha"));
        assert!(r.content.contains("beta"));
        assert!(!r.content.contains("gamma"));
    }

    #[tokio::test]
    async fn edit_unique_hit_rewrites() {
        let tmp = tempdir().unwrap();
        let ctx = ctx_with_store(tmp.path());
        MemoryAppend
            .invoke(
                &call("memory_append", json!({"scope": "project", "text": "old bullet"})),
                &ctx,
            )
            .await
            .unwrap();

        MemoryEdit
            .invoke(
                &call(
                    "memory_edit",
                    json!({"scope": "project", "old": "old bullet", "new": "new bullet"}),
                ),
                &ctx,
            )
            .await
            .unwrap();

        let r = MemoryRead
            .invoke(&call("memory_read", json!({"scope": "project"})), &ctx)
            .await
            .unwrap();
        assert!(r.content.contains("new bullet"));
        assert!(!r.content.contains("old bullet"));
    }

    #[tokio::test]
    async fn edit_ambiguous_returns_error() {
        let tmp = tempdir().unwrap();
        let ctx = ctx_with_store(tmp.path());
        for _ in 0..2 {
            MemoryAppend
                .invoke(
                    &call("memory_append", json!({"scope": "user", "text": "dup"})),
                    &ctx,
                )
                .await
                .unwrap();
        }
        let err = MemoryEdit
            .invoke(
                &call(
                    "memory_edit",
                    json!({"scope": "user", "old": "dup", "new": "unique"}),
                ),
                &ctx,
            )
            .await
            .unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("2") || msg.contains("times"));
    }

    #[tokio::test]
    async fn remember_writes_to_episodic_with_session_id() {
        use mira_core::SessionId;
        use mira_memory::{EpisodicStore, FileEpisodicStore};
        let tmp = tempdir().unwrap();
        let epi: Arc<dyn EpisodicStore> =
            Arc::new(FileEpisodicStore::new(tmp.path().join("episodic.jsonl")));
        let ctx = ToolContext::new(tmp.path(), Arc::new(Sandbox::default_scrubbed()))
            .with_episodic(epi.clone())
            .with_session_id(SessionId::from("sess_abc"));

        MemoryRemember
            .invoke(
                &call("memory_remember", json!({"text": "we use pnpm not npm"})),
                &ctx,
            )
            .await
            .unwrap();

        let recent = epi.recent(10).await.unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].text, "we use pnpm not npm");
        assert_eq!(recent[0].session_id.as_deref(), Some("sess_abc"));
        assert_eq!(recent[0].source, mira_memory::EpisodicSource::Tool);
    }

    #[tokio::test]
    async fn remember_without_store_errors() {
        let tmp = tempdir().unwrap();
        let ctx = ToolContext::new(tmp.path(), Arc::new(Sandbox::default_scrubbed()));
        let err = MemoryRemember
            .invoke(
                &call("memory_remember", json!({"text": "nope"})),
                &ctx,
            )
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("episodic store not wired"));
    }

    #[tokio::test]
    async fn missing_store_surfaces_clear_error() {
        // Explicit smoke test for the misconfig path — no memory handle,
        // so the tool should refuse rather than panic.
        let tmp = tempdir().unwrap();
        let ctx = ToolContext::new(tmp.path(), Arc::new(Sandbox::default_scrubbed()));
        let err = MemoryRead
            .invoke(&call("memory_read", json!({"scope": "user"})), &ctx)
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("memory store not wired"));
    }
}
