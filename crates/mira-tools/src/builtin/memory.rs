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

use async_trait::async_trait;
use mira_ai::ToolSpec;
use mira_core::{ToolCall, ToolResult};
use mira_memory::{search, EpisodicEntry, EpisodicSource, MemoryError, MemoryScope};
use serde::Deserialize;
use serde_json::json;

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
