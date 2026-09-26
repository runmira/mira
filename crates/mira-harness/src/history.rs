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
//! ## Clearing old tool results (cheap, no model call)
//!
//! Once the conversation fills half the model's context window, tool
//! results older than the last few are replaced with a short note in
//! what's *sent* to the model ([`clear_old_tool_results`]). Stored
//! history keeps them, so transcripts stay complete. The cutoff moves
//! in batches, so the provider's prompt cache is rebuilt rarely.
//!
//! ## Compaction (one model call, when that isn't enough)
//!
//! At ~80% of the window the whole conversation is summarized into one
//! structured message (what the user asked, files, errors and fixes,
//! pending work, current work…),
//! followed by fresh copies of the files most recently worked on. The
//! summary replaces the history sent to the model; the replaced
//! messages are returned so the session can keep them for display.
//! Compaction therefore happens rarely — once per filled window, not
//! every few dozen messages.

use std::path::Path;

use anyhow::{anyhow, Result};
use futures::StreamExt;
use mira_ai::{ChatEvent, ChatProvider, ChatRequest};
use mira_core::{Message, Role, ToolCallId};
use serde_json::Value;

/// Share of the context window at which the conversation is summarized.
/// Leaves room for the next reply and tool results, and for the 4
/// chars/token estimate being off.
const AUTO_COMPACT_FRACTION: f64 = 0.8;

/// Share of the window at which old tool results start being cleared.
const CLEAR_TOOL_RESULTS_FRACTION: f64 = 0.5;

/// Tool results always sent in full: the most recent ones.
const KEEP_TOOL_RESULTS: usize = 8;

/// Tool results shorter than this aren't worth clearing.
const CLEARABLE_MIN_CHARS: usize = 1_000;

/// What a cleared tool result says instead.
pub const CLEARED_TOOL_RESULT: &str =
    "[Old tool result cleared to save context. Run the tool again if you need it.]";

/// Cap on the summary the model writes.
const SUMMARY_MAX_TOKENS: u32 = 8_000;

/// Each message is cut to this many characters in what the summarizer
/// reads, so one huge tool result can't crowd out the rest.
const SUMMARIZER_MESSAGE_CHARS: usize = 4_000;

/// Files re-read into context after compaction.
const RESTORE_FILES: usize = 5;

/// A restored file larger than this is only named, not included.
const RESTORE_FILE_MAX_CHARS: usize = 20_000;

/// Starts every compaction summary message. UIs use it to show a
/// "Conversation compacted" divider instead of a user message.
pub const SUMMARY_PREFIX: &str = "<conversation-summary>";

/// Whether `m` is a compaction summary.
pub fn is_summary(m: &Message) -> bool {
    m.role == Role::User
        && m.content
            .as_deref()
            .is_some_and(|c| c.starts_with(SUMMARY_PREFIX))
}

/// The one tool whose results are worth stubbing when superseded.
/// Writes and edits already produce tiny confirmations — nothing to
/// gain by shortening them.
const STUBBABLE_TOOL: &str = "read_file";

/// Tools whose completion means "the content of path P is now
/// different from any prior read of P". A newer read overrides an
/// older one; a write/edit invalidates all older reads.
const SUPERSEDING_TOOLS: &[&str] = &["read_file", "write_file", "edit_file"];

/// How many image-bearing tool results stay intact in history. Older
/// ones lose their images (see [`prune_old_images`]). Three matches
/// Anthropic's computer-use reference loop: enough for the model to
/// compare "before" and "after" an action without paying for a whole
/// session of screenshots on every request.
pub const KEEP_RECENT_IMAGES: usize = 3;

/// Note appended to a tool result whose images were pruned.
const IMAGE_PRUNED_NOTE: &str = "[screenshot omitted from history — a newer one supersedes it]";

/// Drop the images from every image-bearing message except the `keep`
/// most recent. The message itself stays (tool_call_id pairing must
/// survive); its text gains a short note so the transcript still reads
/// coherently. Idempotent: already-pruned messages have no images left
/// and are skipped.
pub fn prune_old_images(history: &mut [Message], keep: usize) {
    let mut seen = 0usize;
    for msg in history.iter_mut().rev() {
        if msg.images.is_empty() {
            continue;
        }
        seen += 1;
        if seen <= keep {
            continue;
        }
        msg.images.clear();
        let text = msg.content.get_or_insert_with(String::new);
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(IMAGE_PRUNED_NOTE);
    }
}

/// Walk back through `history` and stub any prior `read_file` results
/// whose call targeted the same `path`. No-op when `tool_name` isn't
/// in `SUPERSEDING_TOOLS` or `path` is empty.
///
/// - `history` is mutated in place.
/// - `just_appended_call_id` identifies the call we just handled; we
///   never touch its result.
pub fn dedup_reads_for_path(
    history: &mut [Message],
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

/// The context window to plan for: `MIRA_CONTEXT_WINDOW` if set, else
/// [`model_context_window`].
pub fn context_window(model: &str) -> usize {
    std::env::var("MIRA_CONTEXT_WINDOW")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|n| *n >= 4_096)
        .unwrap_or_else(|| model_context_window(model))
}

/// Where tool-result clearing should start from now: the index before
/// which tool results are cleared in requests. Moves only forward, and
/// only once `history` (as sent, with `current` applied) passes half
/// the window; then it jumps to keep just the last
/// [`KEEP_TOOL_RESULTS`] results, so it changes rarely.
pub fn clear_tool_results_before(history: &[Message], model: &str, current: usize) -> usize {
    let budget = (context_window(model) as f64 * CLEAR_TOOL_RESULTS_FRACTION) as usize;
    if estimated_tokens(&clear_old_tool_results(history, current)) <= budget {
        return current;
    }
    let tool_idx: Vec<usize> = history
        .iter()
        .enumerate()
        .filter(|(_, m)| m.role == Role::Tool)
        .map(|(i, _)| i)
        .collect();
    if tool_idx.len() <= KEEP_TOOL_RESULTS {
        return current;
    }
    current.max(tool_idx[tool_idx.len() - KEEP_TOOL_RESULTS])
}

/// `history` as sent to the model: tool results before `before` that
/// are big enough are replaced by [`CLEARED_TOOL_RESULT`] (their
/// `tool_call_id` stays, so calls and results still pair up).
pub fn clear_old_tool_results(history: &[Message], before: usize) -> Vec<Message> {
    history
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let big = m
                .content
                .as_deref()
                .is_some_and(|c| c.len() >= CLEARABLE_MIN_CHARS);
            if i < before && m.role == Role::Tool && (big || !m.images.is_empty()) {
                let mut cleared = m.clone();
                cleared.content = Some(CLEARED_TOOL_RESULT.to_owned());
                cleared.images.clear();
                cleared
            } else {
                m.clone()
            }
        })
        .collect()
}

/// Whether the conversation (as sent: `history` with tool results
/// before `cleared_before` cleared) is full enough to summarize.
pub fn needs_compaction(history: &[Message], model: &str, cleared_before: usize) -> bool {
    let budget = (context_window(model) as f64 * AUTO_COMPACT_FRACTION) as usize;
    estimated_tokens(&clear_old_tool_results(history, cleared_before)) > budget
}

/// Rough char-based token estimate. Real tokenization varies by model
/// (BPE / SentencePiece / tiktoken); 4 chars ≈ 1 token is the
/// widely-cited approximation and is close enough for a *trigger*
/// decision — a factor-of-two error just means we compact a little
/// sooner or later. Structured fields (tool_calls, tool_call_id) are
/// counted so a batch of fat argument JSON also drives compaction.
pub fn estimated_tokens(msgs: &[Message]) -> usize {
    let mut chars: usize = 0;
    for m in msgs {
        if let Some(c) = &m.content {
            chars += c.len();
        }
        for tc in &m.tool_calls {
            chars += tc.function.name.len();
            chars += tc.function.arguments.len();
        }
        // Small fixed overhead per message for the role/wire wrapper —
        // ~4 tokens each is roughly what OpenAI's tokenizer adds.
        chars += 16;
    }
    chars / 4
}

/// Best-effort context-window lookup keyed on the model id. Prefix
/// match keeps the table small and forgiving of provider version
/// bumps — `claude-opus-4-7`, `claude-3-5-sonnet-latest`, etc. all
/// resolve via the "claude" branch. Unknown ids get a conservative
/// 128k default so we don't over-fill smaller windows we haven't
/// catalogued yet.
pub fn model_context_window(model: &str) -> usize {
    let m = model.to_ascii_lowercase();
    if m.contains("[1m]") || m.contains("gemini") || m.contains("gpt-4.1") {
        1_000_000
    } else if m.contains("gpt-5") {
        400_000
    } else if m.contains("claude")
        || m.starts_with("o3")
        || m.starts_with("o4")
        || m.contains("/o3")
        || m.contains("/o4")
    {
        200_000
    } else if m.contains("kimi") {
        256_000
    } else if m.contains("gpt-4o") || m.contains("gpt-4-turbo") {
        128_000
    } else if m.contains("gpt-4-32k") {
        32_768
    } else if m.contains("gpt-4") {
        8_192
    } else if m.contains("gpt-3.5") {
        16_385
    } else {
        128_000
    }
}

/// Summarize the whole conversation with `summarizer_model` and replace
/// it with one summary message (plus fresh copies of recently used
/// files from `cwd`). System messages stay. `focus` is what the user
/// asked the summary to keep (`/compact <focus>`).
///
/// Returns the messages that were replaced, for display; history is
/// only changed when a summary comes back. Errors leave it untouched.
pub async fn compact(
    history: &mut Vec<Message>,
    provider: &dyn ChatProvider,
    summarizer_model: &str,
    cwd: &Path,
    focus: Option<&str>,
) -> Result<Vec<Message>> {
    let system_end = history
        .iter()
        .take_while(|m| m.role == Role::System)
        .count();
    // A turn that just started keeps its new message after the summary,
    // word for word, instead of folding it in.
    let fresh_prompt = history
        .last()
        .is_some_and(|m| m.role == Role::User && !is_summary(m));
    let end = history.len() - usize::from(fresh_prompt);
    if end.saturating_sub(system_end) < 2 {
        return Err(anyhow!("not enough conversation to compact"));
    }
    let transcript = render_for_summary(&history[system_end..end]);
    let summary = summarize(provider, summarizer_model, &transcript, focus).await?;
    let files = restore_files(cwd, &recent_paths(&history[system_end..end]));
    let mut text = format!(
        "{SUMMARY_PREFIX}\nThis session continues from an earlier conversation that was summarized \
         to free up context. The summary:\n\n{}\n</conversation-summary>",
        summary.trim()
    );
    if !files.is_empty() {
        text.push_str("\n\nCurrent contents of files recently worked on:\n\n");
        text.push_str(&files);
    }
    text.push_str(
        "\n\nContinue from where things left off without asking the user to repeat \
         anything. If you were in the middle of a task, carry on with it.",
    );
    let removed: Vec<Message> = history.drain(system_end..end).collect();
    history.insert(system_end, Message::user(text));
    Ok(removed)
}

/// [`compact`] with `summarizer_model`, retried once on `session_model`
/// if that fails and is a different (cheaper) model.
pub async fn compact_with_fallback(
    history: &mut Vec<Message>,
    provider: &dyn ChatProvider,
    session_model: &str,
    summarizer_model: &str,
    cwd: &Path,
    focus: Option<&str>,
) -> Result<Vec<Message>> {
    match compact(history, provider, summarizer_model, cwd, focus).await {
        Err(e) if summarizer_model != session_model => {
            tracing::warn!(error = %e, model = summarizer_model, "compaction failed; retrying on the main model");
            compact(history, provider, session_model, cwd, focus).await
        }
        r => r,
    }
}

/// The conversation as plain text for the summarizer. Tool calls become
/// one line each; long messages are cut in the middle.
fn render_for_summary(msgs: &[Message]) -> String {
    let mut out = String::new();
    for m in msgs {
        let role = match m.role {
            Role::System => "system",
            Role::User if is_summary(m) => "earlier summary",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool result",
        };
        out.push_str(&format!("[{role}]\n"));
        if let Some(c) = &m.content {
            out.push_str(&clip_middle(c, SUMMARIZER_MESSAGE_CHARS));
            out.push('\n');
        }
        for tc in &m.tool_calls {
            out.push_str(&format!(
                "(calls {} {})\n",
                tc.function.name,
                clip_middle(&tc.function.arguments, 600)
            ));
        }
        out.push('\n');
    }
    out
}

/// `s` with its middle replaced by a marker when longer than `max` chars.
fn clip_middle(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        return s.to_owned();
    }
    let head: String = s.chars().take(max * 2 / 3).collect();
    let tail: String = s.chars().skip(n - max / 3).collect();
    format!("{head}\n[… {} characters cut …]\n{tail}", n - max)
}

/// Paths the conversation read or changed, most recent first, unique.
fn recent_paths(msgs: &[Message]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for m in msgs.iter().rev() {
        for tc in m.tool_calls.iter().rev() {
            if !SUPERSEDING_TOOLS.contains(&tc.function.name.as_str()) {
                continue;
            }
            if let Some(p) = path_from_args(&tc.function.arguments) {
                if !out.contains(&p) {
                    out.push(p);
                }
            }
        }
        if out.len() >= RESTORE_FILES {
            break;
        }
    }
    out.truncate(RESTORE_FILES);
    out
}

/// Current contents of `paths` (relative to `cwd`), as tagged blocks.
/// Missing files are skipped; big ones are named without contents.
fn restore_files(cwd: &Path, paths: &[String]) -> String {
    let mut out = String::new();
    for p in paths {
        let full = cwd.join(p);
        let Ok(text) = std::fs::read_to_string(&full) else {
            continue;
        };
        if text.len() > RESTORE_FILE_MAX_CHARS {
            out.push_str(&format!(
                "<file path=\"{p}\">(large file, {} lines; read it again if you need it)</file>\n",
                text.lines().count()
            ));
        } else {
            out.push_str(&format!("<file path=\"{p}\">\n{text}\n</file>\n"));
        }
    }
    out
}

const SUMMARY_INSTRUCTIONS: &str = "Write a detailed summary of the conversation below between a user and \
an AI coding assistant. It replaces the conversation in the assistant's memory, so the assistant must be \
able to continue the work from it alone. Pay close attention to the user's explicit requests and to the \
assistant's most recent work. If an earlier summary appears, fold its content in.

Use these sections:

1. Primary request and intent: everything the user asked for, in detail.
2. Key technical concepts: technologies, frameworks and conventions involved.
3. Files and code: files read, changed or created, why each matters, and the important code (short snippets).
4. Errors and fixes: what went wrong and how it was fixed, including anything the user said about it.
5. Problem solving: what was solved and what is still being worked out.
6. All user messages: list every message from the user (not tool results), briefly, in order.
7. Pending tasks: what was asked for and not done yet.
8. Current work: exactly what was being done right before this summary, with file names and code.
9. Next step: the next step, only if it follows directly from the user's latest request, quoting that request.

Be specific (paths, function names, commands, decisions). No preamble.";

async fn summarize(
    provider: &dyn ChatProvider,
    model: &str,
    transcript: &str,
    focus: Option<&str>,
) -> Result<String> {
    let mut ask = format!("{SUMMARY_INSTRUCTIONS}\n\n<conversation>\n{transcript}</conversation>");
    if let Some(f) = focus.map(str::trim).filter(|f| !f.is_empty()) {
        ask.push_str(&format!("\n\nThe user asked the summary to focus on: {f}"));
    }
    let req = ChatRequest {
        model: model.to_owned(),
        messages: vec![Message::user(ask)],
        tools: Vec::new(),
        temperature: Some(0.0),
        max_tokens: Some(SUMMARY_MAX_TOKENS),
        reasoning_effort: None,
        response_format: None,
    };
    let mut stream = provider.stream(req).await?;
    let mut text = String::new();
    while let Some(evt) = stream.next().await {
        match evt? {
            ChatEvent::TextDelta(t) => text.push_str(&t),
            ChatEvent::Done(_) => break,
            _ => {}
        }
    }
    if text.trim().is_empty() {
        return Err(anyhow!("the summarizer returned nothing"));
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
            images: Vec::new(),
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
            images: Vec::new(),
        }
    }

    fn tool_result(id: &str, content: &str) -> Message {
        Message::tool(cid(id), content)
    }

    #[test]
    fn prune_old_images_keeps_only_the_most_recent() {
        let img = || vec![mira_core::ImageData::png("AAAA")];
        let mut h = vec![
            tool_result("a", "shot a").with_images(img()),
            tool_result("b", "shot b").with_images(img()),
            tool_result("c", "no image"),
            tool_result("d", "shot d").with_images(img()),
        ];
        prune_old_images(&mut h, 2);
        assert!(h[0].images.is_empty());
        assert!(h[0]
            .content
            .as_deref()
            .unwrap()
            .contains("screenshot omitted"));
        assert_eq!(h[1].images.len(), 1);
        assert_eq!(h[2].content.as_deref(), Some("no image"));
        assert_eq!(h[3].images.len(), 1);
        // Idempotent.
        let before = h[0].content.clone();
        prune_old_images(&mut h, 2);
        assert_eq!(h[0].content, before);
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

    /// Fails every request for `tiny`; summarizes for anything else.
    struct PicksyProvider {
        asked: std::sync::Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl ChatProvider for PicksyProvider {
        async fn stream(
            &self,
            r: mira_ai::ChatRequest,
        ) -> std::result::Result<
            futures::stream::BoxStream<
                'static,
                std::result::Result<mira_ai::ChatEvent, mira_ai::ProviderError>,
            >,
            mira_ai::ProviderError,
        > {
            use futures::StreamExt;
            self.asked.lock().unwrap().push(r.model.clone());
            if r.model == "tiny" {
                return Err(mira_ai::ProviderError::Status {
                    status: 404,
                    body: "no such model".into(),
                    retry_after: None,
                });
            }
            let events = vec![
                Ok(mira_ai::ChatEvent::TextDelta("summary".into())),
                Ok(mira_ai::ChatEvent::Done(mira_ai::FinishReason::Stop)),
            ];
            Ok(futures::stream::iter(events).boxed())
        }
    }

    fn long_history() -> Vec<Message> {
        let mut h = vec![Message::system("s")];
        for i in 0..20 {
            h.push(Message::user(format!("u{i}")));
            h.push(Message::assistant(format!("a{i}")));
        }
        h
    }

    #[tokio::test]
    async fn failed_small_summarizer_falls_back_to_the_main_model() {
        let p = PicksyProvider {
            asked: Default::default(),
        };
        let mut h = long_history();
        let removed =
            compact_with_fallback(&mut h, &p, "claude-opus-4-7", "tiny", Path::new("."), None)
                .await
                .unwrap();
        assert_eq!(removed.len(), 40);
        assert_eq!(h.len(), 2, "system + summary");
        assert!(is_summary(&h[1]));
        assert_eq!(*p.asked.lock().unwrap(), ["tiny", "claude-opus-4-7"]);
    }

    #[tokio::test]
    async fn main_model_failure_is_not_retried() {
        let p = PicksyProvider {
            asked: Default::default(),
        };
        let mut h = long_history();
        let before = h.len();
        assert!(
            compact_with_fallback(&mut h, &p, "tiny", "tiny", Path::new("."), None)
                .await
                .is_err()
        );
        assert_eq!(h.len(), before, "history untouched on failure");
        assert_eq!(p.asked.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn summary_restores_recent_files_and_folds_in_old_summaries() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.rs"), "fn a() {}").unwrap();
        std::fs::write(tmp.path().join("big.rs"), "x\n".repeat(20_000)).unwrap();
        let p = PicksyProvider {
            asked: Default::default(),
        };
        let mut h = vec![
            Message::system("s"),
            Message::user(format!("{SUMMARY_PREFIX} old")),
        ];
        h.push(assistant_reads("c1", "a.rs"));
        h.push(tool_result("c1", "fn a() {}"));
        h.push(assistant_writes("c2", "big.rs"));
        h.push(tool_result("c2", "ok"));
        h.push(assistant_reads("c3", "gone.rs"));
        h.push(tool_result("c3", "missing"));
        let removed = compact(&mut h, &p, "m", tmp.path(), Some("the parser"))
            .await
            .unwrap();
        assert_eq!(removed.len(), 7);
        let text = h[1].content.as_deref().unwrap();
        assert!(text.starts_with(SUMMARY_PREFIX));
        assert!(text.contains("<file path=\"a.rs\">\nfn a() {}"));
        assert!(text.contains("<file path=\"big.rs\">(large file"));
        assert!(!text.contains("gone.rs"));
        assert_eq!(recent_paths(&removed), ["gone.rs", "big.rs", "a.rs"]);
        let rendered = render_for_summary(&removed);
        assert!(rendered.starts_with("[earlier summary]"));
    }

    #[tokio::test]
    async fn a_new_prompt_stays_after_the_summary() {
        let p = PicksyProvider {
            asked: Default::default(),
        };
        let mut h = long_history();
        h.push(Message::user("now do this"));
        let removed = compact(&mut h, &p, "m", Path::new("."), None)
            .await
            .unwrap();
        assert_eq!(removed.len(), 40);
        assert_eq!(h.len(), 3);
        assert!(is_summary(&h[1]));
        assert_eq!(h[2].content.as_deref(), Some("now do this"));
    }

    #[tokio::test]
    async fn too_little_to_compact() {
        let p = PicksyProvider {
            asked: Default::default(),
        };
        let mut h = vec![Message::system("s"), Message::user("hi")];
        assert!(compact(&mut h, &p, "m", Path::new("."), None)
            .await
            .is_err());
        assert!(p.asked.lock().unwrap().is_empty());
    }

    /// 15 rounds with a 40 KB tool result each: ~150k tokens.
    fn fat_history() -> Vec<Message> {
        let mut h = vec![Message::system("system prompt")];
        for i in 0..15 {
            h.push(Message::user(format!("u{i}")));
            h.push(assistant_reads(&format!("t{i}"), "f.rs"));
            h.push(Message::tool(cid(&format!("t{i}")), "x".repeat(40_000)));
        }
        h
    }

    #[test]
    fn old_tool_results_are_cleared_before_anything_is_summarized() {
        let h = fat_history();
        // 150k tokens against a 200k window: past half, not past 80%.
        let before = clear_tool_results_before(&h, "claude-opus-4-7", 0);
        assert!(before > 0);
        let sent = clear_old_tool_results(&h, before);
        let cleared = sent
            .iter()
            .filter(|m| m.content.as_deref() == Some(CLEARED_TOOL_RESULT))
            .count();
        assert_eq!(cleared, 15 - KEEP_TOOL_RESULTS);
        assert_eq!(sent.len(), h.len(), "pairing kept");
        assert!(!needs_compaction(&h, "claude-opus-4-7", before));
        // Stable: asking again doesn't move it.
        assert_eq!(
            clear_tool_results_before(&h, "claude-opus-4-7", before),
            before
        );
    }

    #[test]
    fn a_full_window_needs_compaction_and_a_small_chat_never_does() {
        let h = fat_history();
        assert!(needs_compaction(&h, "gpt-3.5-turbo", 0));
        let small = long_history();
        assert!(!needs_compaction(&small, "claude-opus-4-7", 0));
        assert_eq!(clear_tool_results_before(&small, "claude-opus-4-7", 0), 0);
        // Message count alone never triggers it any more.
        let mut many = vec![Message::system("s")];
        for i in 0..500 {
            many.push(Message::user(format!("u{i}")));
        }
        assert!(!needs_compaction(&many, "claude-opus-4-7", 0));
    }

    #[test]
    fn clipping_keeps_both_ends() {
        let s = format!("{}{}", "a".repeat(5000), "z".repeat(5000));
        let c = clip_middle(&s, 3000);
        assert!(c.starts_with("aaa") && c.ends_with("zzz"));
        assert!(c.contains("characters cut"));
        assert_eq!(clip_middle("short", 3000), "short");
    }

    #[test]
    fn model_context_window_prefix_matches() {
        assert_eq!(model_context_window("claude-opus-4-7"), 200_000);
        assert_eq!(model_context_window("Claude-3-5-Sonnet"), 200_000);
        assert_eq!(model_context_window("gemini-1.5-pro"), 1_000_000);
        assert_eq!(model_context_window("gpt-4o-mini"), 128_000);
        assert_eq!(model_context_window("openai/gpt-5-mini"), 400_000);
        assert_eq!(model_context_window("gpt-4.1"), 1_000_000);
        assert_eq!(model_context_window("gpt-4-turbo-2024-04-09"), 128_000);
        assert_eq!(model_context_window("gpt-4-32k"), 32_768);
        assert_eq!(model_context_window("gpt-4-0613"), 8_192);
        assert_eq!(model_context_window("gpt-3.5-turbo"), 16_385);
        assert_eq!(model_context_window("some-unknown-model"), 128_000);
    }
}
