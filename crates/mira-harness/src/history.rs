//! Session-history hygiene: dedup superseded reads + rolling
//! compaction.
//!
//! Both target the same failure mode — long sessions where old tool
//! results (especially fat file reads) sit in context forever, hogging
//! tokens the model no longer needs.
//!
//! ## Dedup (cheap, applied per tool result)
//!
//! When a `read_file` / `write_file` / `edit_file` for path P lands
//! in history, walk back and collapse any older `read_file` results
//! for P — the newer read (or write) supersedes them. Stubbed
//! messages keep their `tool_call_id` so the provider-side pairing
//! stays valid; only their content shrinks. Idempotent: an
//! already-stubbed message is left alone.
//!
//! ## Compaction (expensive, applied when history crosses a
//! threshold)
//!
//! Once the non-system history exceeds a message-count trigger, we
//! replace the older half with a single synthetic "memory of earlier
//! conversation" user message produced by summarizing the tail with
//! the same provider. Boundaries land on a `Role::User` edge so we
//! never sever an assistant→tool_result pairing — both OpenAI and
//! Anthropic reject a tool message that isn't answering a call in the
//! immediately-preceding assistant turn.

use std::ops::Range;

use anyhow::{anyhow, Result};
use futures::StreamExt;
use mira_ai::{ChatEvent, ChatProvider, ChatRequest};
use mira_core::{Message, Role, ToolCallId};
use serde_json::Value;

/// Non-system message count at which compaction fires. Roughly
/// "30ish tool-heavy rounds"; below this, compaction never touches
/// history.
const COMPACT_TRIGGER: usize = 60;

/// Number of most-recent non-system messages compaction keeps raw
/// after summarizing the older tail.
const COMPACT_KEEP_RECENT: usize = 30;

/// Cap on the summarizer's own reply. Enough for a few paragraphs of
/// memory; short enough that a runaway summarizer can't cost real
/// money.
const COMPACT_SUMMARY_TOKENS: u32 = 800;

/// The one tool whose results are worth stubbing when superseded.
/// Writes and edits already produce tiny confirmations — nothing to
/// gain by shortening them.
const STUBBABLE_TOOL: &str = "read_file";

/// Tools whose completion means "the content of path P is now
/// different from any prior read of P". A newer read overrides an
/// older one; a write/edit invalidates all older reads.
const SUPERSEDING_TOOLS: &[&str] = &["read_file", "write_file", "edit_file"];

/// Walk back through `history` and stub any prior `read_file` results
/// whose call targeted the same `path`. No-op when `tool_name` isn't
/// in `SUPERSEDING_TOOLS` or `path` is empty.
///
/// - `history` is mutated in place.
/// - `just_appended_call_id` identifies the call we just handled; we
///   never touch its result.
pub fn dedup_reads_for_path(
    history: &mut Vec<Message>,
    just_appended_call_id: &ToolCallId,
    tool_name: &str,
    path: &str,
) {
    if !SUPERSEDING_TOOLS.contains(&tool_name) || path.is_empty() {
        return;
    }
    // First pass: collect call_ids of prior `read_file` calls for the
    // same path. We look on the assistant messages (which carry the
    // ToolCall structs with name + arguments); the tool result
    // messages we'll mutate don't carry that info.
    let mut prior_read_call_ids: Vec<ToolCallId> = Vec::new();
    for msg in history.iter() {
        if msg.role != Role::Assistant {
            continue;
        }
        for tc in &msg.tool_calls {
            if tc.id == *just_appended_call_id {
                continue;
            }
            if tc.function.name != STUBBABLE_TOOL {
                continue;
            }
            if let Some(p) = path_from_args(&tc.function.arguments) {
                if p == path {
                    prior_read_call_ids.push(tc.id.clone());
                }
            }
        }
    }
    if prior_read_call_ids.is_empty() {
        return;
    }
    // Second pass: stub the matching tool results in-place.
    for msg in history.iter_mut() {
        if msg.role != Role::Tool {
            continue;
        }
        let Some(id) = &msg.tool_call_id else {
            continue;
        };
        if !prior_read_call_ids.iter().any(|x| x == id) {
            continue;
        }
        if msg.content.as_deref().is_some_and(is_stub) {
            continue;
        }
        msg.content = Some(stub_content(path));
    }
}

/// Pull the `path` argument out of a serialized JSON tool-call args
/// blob. `None` when the blob doesn't parse or has no string `path`
/// key. Public so `session.rs` can share the same convention when
/// deciding whether to fire dedup.
pub fn path_from_args(args_json: &str) -> Option<String> {
    let v: Value = serde_json::from_str(args_json).ok()?;
    v.get("path")?.as_str().map(|s| s.to_owned())
}

/// The replacement content used for a superseded read.
pub fn stub_content(path: &str) -> String {
    format!(
        "[superseded — `{path}` was re-read or edited in a later turn; content omitted to save context]"
    )
}

fn is_stub(content: &str) -> bool {
    content.starts_with("[superseded")
}

/// Return the range `[start..end)` of history indices that should be
/// summarized and replaced by a single synthetic message, or `None`
/// if compaction isn't needed / can't be done safely.
///
/// Rules:
/// - Skip the leading run of system messages (they stay put).
/// - Trigger only when the non-system tail exceeds `COMPACT_TRIGGER`.
/// - Preserve the last `COMPACT_KEEP_RECENT` messages raw.
/// - Walk `end` left until it points at a `Role::User` boundary — the
///   only place we can split without tearing apart an
///   assistant/tool_result pair. If no such boundary exists in the
///   older tail, return `None`.
pub fn find_compact_range(history: &[Message]) -> Option<Range<usize>> {
    let system_end = history
        .iter()
        .take_while(|m| m.role == Role::System)
        .count();
    let non_system_len = history.len() - system_end;
    if non_system_len <= COMPACT_TRIGGER {
        return None;
    }
    let start = system_end;
    let naive_end = history.len().saturating_sub(COMPACT_KEEP_RECENT);
    let mut end = naive_end;
    while end > start && history[end].role != Role::User {
        end -= 1;
    }
    if end <= start {
        return None;
    }
    Some(start..end)
}

/// If history exceeds the compaction threshold, summarize the older
/// tail via the provider and splice a single synthetic user message
/// into its place. Returns `Ok(Some(count))` with the number of
/// messages replaced when compaction happened, `Ok(None)` when no-op.
///
/// Errors surface provider failures — the caller decides to log and
/// continue with the uncompacted history.
pub async fn maybe_compact(
    history: &mut Vec<Message>,
    provider: &dyn ChatProvider,
    model: &str,
) -> Result<Option<usize>> {
    let Some(range) = find_compact_range(history) else {
        return Ok(None);
    };
    let count = range.end - range.start;
    let tail_text = render_tail(&history[range.clone()]);
    let summary = summarize_tail(provider, model, &tail_text).await?;
    let synthetic = Message::user(format!(
        "[MEMORY OF EARLIER CONVERSATION — the previous {count} messages were summarized to save context.]\n\n{summary}\n\n[END MEMORY]"
    ));
    history.splice(range, std::iter::once(synthetic));
    Ok(Some(count))
}

/// Render the compaction tail as plain text the summarizer can chew
/// on. Structured fields (tool_calls, tool_call_ids) are flattened
/// into readable prose — the summarizer describes what happened, it
/// doesn't have to reconstruct the wire shape.
fn render_tail(msgs: &[Message]) -> String {
    let mut out = String::new();
    for m in msgs {
        let role = match m.role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        };
        out.push_str(&format!("[{role}]\n"));
        if let Some(c) = &m.content {
            out.push_str(c);
            out.push('\n');
        }
        for tc in &m.tool_calls {
            out.push_str(&format!(
                "(tool call: {} {})\n",
                tc.function.name, tc.function.arguments
            ));
        }
        out.push('\n');
    }
    out
}

async fn summarize_tail(provider: &dyn ChatProvider, model: &str, tail: &str) -> Result<String> {
    let system = Message::system(
        "You are compressing the older portion of a chat between a user and an AI coding assistant so the conversation can continue without exceeding the model's context window. Your output goes back into the assistant's context as a memory of what happened.\n\n\
         Preserve, concisely:\n\
         - What the user was ultimately trying to accomplish.\n\
         - Concrete facts discovered (file paths, function names, line numbers, decisions).\n\
         - The current state — what's been changed, what's pending, what failed.\n\
         - Anything the assistant would need to remember in later turns.\n\n\
         Do NOT include greetings, filler, or verbatim message content. A few short paragraphs at most.",
    );
    let user = Message::user(format!(
        "Compress this conversation tail into a short memory block:\n\n{tail}"
    ));
    let req = ChatRequest {
        model: model.to_owned(),
        messages: vec![system, user],
        tools: Vec::new(),
        temperature: Some(0.0),
        max_tokens: Some(COMPACT_SUMMARY_TOKENS),
        reasoning_effort: None,
        response_format: None,
    };
    let mut stream = provider.stream(req).await?;
    let mut text = String::new();
    while let Some(evt) = stream.next().await {
        if let Ok(ChatEvent::TextDelta(t)) = evt {
            text.push_str(&t);
        }
    }
    if text.trim().is_empty() {
        return Err(anyhow!("summarizer returned empty"));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mira_core::message::{ToolCallFunction, ToolCallKind};
    use mira_core::ToolCall;

    fn cid(s: &str) -> ToolCallId {
        ToolCallId::from(s)
    }

    fn assistant_reads(id: &str, path: &str) -> Message {
        Message {
            role: Role::Assistant,
            content: None,
            tool_calls: vec![ToolCall {
                id: cid(id),
                kind: ToolCallKind::Function,
                function: ToolCallFunction {
                    name: "read_file".into(),
                    arguments: format!(r#"{{"path":"{path}"}}"#),
                },
            }],
            tool_call_id: None,
            name: None,
        }
    }

    fn assistant_writes(id: &str, path: &str) -> Message {
        Message {
            role: Role::Assistant,
            content: None,
            tool_calls: vec![ToolCall {
                id: cid(id),
                kind: ToolCallKind::Function,
                function: ToolCallFunction {
                    name: "write_file".into(),
                    arguments: format!(r#"{{"path":"{path}","content":"new"}}"#),
                },
            }],
            tool_call_id: None,
            name: None,
        }
    }

    fn tool_result(id: &str, content: &str) -> Message {
        Message::tool(cid(id), content)
    }

    #[test]
    fn dedup_stubs_prior_read_of_same_path() {
        let mut h = vec![
            Message::system("s"),
            Message::user("u1"),
            assistant_reads("c1", "src/foo.rs"),
            tool_result("c1", "first read content..."),
            Message::user("u2"),
            assistant_reads("c2", "src/foo.rs"),
            tool_result("c2", "second read content..."),
        ];
        dedup_reads_for_path(&mut h, &cid("c2"), "read_file", "src/foo.rs");
        assert!(is_stub(h[3].content.as_deref().unwrap()));
        assert_eq!(h[6].content.as_deref().unwrap(), "second read content...");
    }

    #[test]
    fn dedup_leaves_other_paths_alone() {
        let mut h = vec![
            Message::system("s"),
            Message::user("u1"),
            assistant_reads("c1", "src/foo.rs"),
            tool_result("c1", "foo content"),
            assistant_reads("c2", "src/bar.rs"),
            tool_result("c2", "bar content"),
        ];
        dedup_reads_for_path(&mut h, &cid("c2"), "read_file", "src/bar.rs");
        assert_eq!(h[3].content.as_deref().unwrap(), "foo content");
    }

    #[test]
    fn dedup_writes_supersede_prior_reads() {
        let mut h = vec![
            Message::system("s"),
            Message::user("u1"),
            assistant_reads("c1", "src/foo.rs"),
            tool_result("c1", "foo content"),
            assistant_writes("c2", "src/foo.rs"),
            tool_result("c2", "wrote 3 bytes"),
        ];
        dedup_reads_for_path(&mut h, &cid("c2"), "write_file", "src/foo.rs");
        assert!(is_stub(h[3].content.as_deref().unwrap()));
    }

    #[test]
    fn dedup_is_idempotent_on_stubbed() {
        let mut h = vec![
            Message::system("s"),
            Message::user("u1"),
            assistant_reads("c1", "src/foo.rs"),
            tool_result("c1", &stub_content("src/foo.rs")),
            assistant_reads("c2", "src/foo.rs"),
            tool_result("c2", "fresh content"),
        ];
        dedup_reads_for_path(&mut h, &cid("c2"), "read_file", "src/foo.rs");
        assert_eq!(h[3].content.as_deref().unwrap(), stub_content("src/foo.rs"));
    }

    #[test]
    fn compact_range_none_below_threshold() {
        let mut h = vec![Message::system("s")];
        for i in 0..COMPACT_TRIGGER {
            h.push(Message::user(format!("u{i}")));
        }
        assert_eq!(find_compact_range(&h), None);
    }

    #[test]
    fn compact_range_lands_on_user_boundary() {
        let mut h = vec![Message::system("s")];
        // Pattern: user, assistant, tool — repeat, so the naive
        // `end` may fall on an assistant or tool message and needs
        // to walk left to a user.
        for i in 0..(COMPACT_TRIGGER + 5) {
            h.push(Message::user(format!("u{i}")));
            h.push(Message::assistant(format!("a{i}")));
            h.push(Message::tool(cid(&format!("t{i}")), "tool result"));
        }
        let r = find_compact_range(&h).expect("should compact");
        assert!(r.start >= 1);
        assert_eq!(h[r.end].role, Role::User, "end must sit on user boundary");
    }
}
