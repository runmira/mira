//! Consolidate memory bullets by calling a cheap model.
//!
//! One helper for both the MIRA.md files and the episodic tail. The
//! caller serialises whatever it has into bullet-list text, hands it to
//! the model, and gets back a consolidated bullet list — with the
//! system prompt tuned to preserve every distinct fact while merging
//! duplicates and resolving contradictions.
//!
//! Kept in `mira-tools` (not `mira-memory`) because it needs a
//! `ChatProvider`, which `mira-memory` doesn't and shouldn't depend on.
//! The tool wrapper lives in `builtin::memory`; the CLI subcommand in
//! `mira-cli` calls this helper directly, sharing the same prompt.

use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use futures::StreamExt;
use mira_ai::{ChatEvent, ChatProvider, ChatRequest};
use mira_core::Message;

/// Wall-clock cap on one consolidation call. Consolidation runs on
/// demand (user-triggered), so a slower model is fine — this is a
/// backstop, not a per-round budget.
pub const CONSOLIDATE_TIMEOUT: Duration = Duration::from_secs(90);

/// Cap on how much text we hand the model in one call. Larger memory
/// files get chunked by the caller (episodic-tail helper below) before
/// hitting this ceiling. Approximate: 32 KB ≈ 8000 tokens of English
/// prose, well inside a small model's context.
pub const CONSOLIDATE_INPUT_MAX: usize = 32 * 1024;

/// Cap on the consolidated reply. Should always be smaller than the
/// input (that's the point) but we keep some slack for models that
/// verbose the surviving entries.
pub const CONSOLIDATE_MAX_TOKENS: u32 = 4096;

/// Run the consolidation model call. Returns the merged bullet list as
/// one string, one bullet per line (with multi-line thoughts as indented
/// continuations). Rejects empty content up front so we never waste a
/// provider call.
pub async fn consolidate_bullets(
    provider: &dyn ChatProvider,
    model: &str,
    content: &str,
    instruction: Option<&str>,
) -> Result<String> {
    if content.trim().is_empty() {
        bail!("consolidate: input is empty");
    }
    if content.len() > CONSOLIDATE_INPUT_MAX {
        bail!(
            "consolidate: input too large ({} bytes; cap {CONSOLIDATE_INPUT_MAX}). \
             Consolidate a smaller section first, or split the memory file.",
            content.len()
        );
    }
    let instruction_line = instruction
        .and_then(|s| {
            let t = s.trim();
            if t.is_empty() {
                None
            } else {
                Some(t)
            }
        })
        .map(|s| format!("\n\nAdditional user instruction: {s}"))
        .unwrap_or_default();

    let system = format!(
        "You are curating a coding agent's persistent memory. You are given \
         the current memory entries — some may be redundant, duplicated in \
         different phrasings, or contradictory. Consolidate them into a \
         clean bullet list following these rules:\n\n\
         - Preserve every distinct fact. When two entries say the same thing, \
           merge them into one clearer bullet.\n\
         - Resolve contradictions in favour of the more recent entry (later \
           in the list = newer).\n\
         - Remove entries that are pure noise (session-specific state that \
           slipped in, empty bullets, etc.).\n\
         - Do NOT invent new facts. Do NOT drop facts you are unsure about.\n\
         - Keep the bullet-list Markdown format. One idea per bullet. \
           Multi-line thoughts use indented continuation lines.\n\n\
         Return ONLY the consolidated bullet list. No preamble, no code fences, \
         no commentary.{instruction_line}"
    );
    let user = format!("--- CURRENT MEMORY ---\n\n{content}");

    let req = ChatRequest {
        model: model.to_owned(),
        messages: vec![Message::system(system), Message::user(user)],
        tools: Vec::new(),
        temperature: Some(0.0),
        max_tokens: Some(CONSOLIDATE_MAX_TOKENS),
        reasoning_effort: None,
        response_format: None,
    };

    let stream = tokio::time::timeout(CONSOLIDATE_TIMEOUT, provider.stream(req))
        .await
        .map_err(|_| anyhow!("consolidate: stream setup timed out"))?
        .map_err(|e| anyhow!("consolidate: provider error: {e}"))?;

    let buf = tokio::time::timeout(CONSOLIDATE_TIMEOUT, read_text(stream))
        .await
        .map_err(|_| anyhow!("consolidate: stream read timed out"))??;

    let cleaned = strip_code_fence(buf.trim());
    if cleaned.trim().is_empty() {
        bail!("consolidate: model returned empty content");
    }
    Ok(cleaned.to_owned())
}

async fn read_text(
    mut stream: futures::stream::BoxStream<
        'static,
        Result<ChatEvent, mira_ai::ProviderError>,
    >,
) -> Result<String> {
    let mut buf = String::new();
    while let Some(evt) = stream.next().await {
        match evt.map_err(|e| anyhow!("stream error: {e}"))? {
            ChatEvent::TextDelta(t) => buf.push_str(&t),
            ChatEvent::Done(_) => break,
            _ => {}
        }
    }
    Ok(buf)
}

/// Some models wrap their reply in a `\`\`\`markdown … \`\`\`` fence
/// even when the system prompt says not to. Strip a single leading /
/// trailing fence pair; leave internal fences alone.
fn strip_code_fence(s: &str) -> &str {
    let trimmed = s.trim();
    let Some(after_open) = trimmed.strip_prefix("```") else {
        return trimmed;
    };
    // Skip the language tag on the same line as the opening fence.
    let after_lang = match after_open.find('\n') {
        Some(nl) => &after_open[nl + 1..],
        None => after_open,
    };
    let stripped = after_lang
        .strip_suffix("```")
        .or_else(|| after_lang.strip_suffix("```\n"))
        .unwrap_or(after_lang);
    stripped.trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_fence_bare_markdown() {
        let s = "```markdown\n- a\n- b\n```";
        assert_eq!(strip_code_fence(s), "- a\n- b");
    }

    #[test]
    fn strip_fence_no_lang() {
        let s = "```\n- a\n- b\n```";
        assert_eq!(strip_code_fence(s), "- a\n- b");
    }

    #[test]
    fn strip_fence_absent_is_pass_through() {
        let s = "- a\n- b\n";
        assert_eq!(strip_code_fence(s), "- a\n- b");
    }
}
