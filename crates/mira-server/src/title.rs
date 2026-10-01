//! Post-hoc session-nickname generation.
//!
//! After a turn lands, we spawn a short model call to pick a nickname for
//! the session.:
//! - Title from the **user's messages only** — the assistant's opening
//!   greeting ("No worries! I'm here and ready…") is generic filler that
//!   drags every title toward "Respond to greeting" blandness.
//! - Skip insubstantial openers ("huh?", "hi", "thanks") and wait for a
//!   message with actual content instead of titling trash forever.
//! - Prompt for specificity (entity + action/goal), same language as the
//!   user, 15-40 chars.
//!
//! The nickname persists via `Session::set_title`; the next sidebar
//! refresh picks it up. Runs on the cheap background model so it stays
//! fast regardless of the session's main model.

use std::sync::Arc;

use futures::StreamExt;
use mira_ai::{ChatEvent, ChatProvider, ChatRequest};
use mira_core::{Message, Role};
use mira_harness::Session;
use tracing::{debug, warn};

use crate::protocol::ServerMsg;
use crate::state::AppState;

const TITLE_CHAR_CAP: usize = 60;

/// Fire-and-forget: if the session lacks a title and has a substantive
/// user message, ask the model for one. Broadcasts `SessionTitleUpdated`
/// on success so connected UIs refresh the sidebar row without waiting
/// for the next `done`. Silent no-op if the session already has a title
/// or the conversation is still smalltalk — we'll try again after the
/// next turn.
///
/// `model` is the cheap model to try; `fallback` (the session's main
/// model, when different) gets one retry if that call fails.
pub fn spawn_if_needed(
    session: Session,
    provider: Arc<dyn ChatProvider>,
    model: String,
    fallback: Option<String>,
    state: AppState,
) {
    tokio::spawn(async move {
        if session.title().await.is_some() {
            return;
        }
        let history = session.transcript().await;
        let users: Vec<String> = history
            .iter()
            .filter(|m| m.role == Role::User && !mira_harness::history::is_summary(m))
            .filter_map(|m| m.content.as_deref())
            .map(|c| {
                let stripped = mira_harness::history::strip_hook_context(c);
                strip_attachments(stripped)
            })
            .filter(|c| is_substantive(c))
            .take(2)
            .map(str::to_owned)
            .collect();
        if users.is_empty() {
            // Nothing titlable yet (greetings, "huh?", empty). Leave the
            // title unset — we'll try again after the next turn.
            return;
        }

        let mut generated = generate(&*provider, &model, &users).await;
        if let (Err(e), Some(main)) = (&generated, &fallback) {
            warn!(session = %session.id, error = %e, %model, "title: failed; retrying on the main model");
            generated = generate(&*provider, main, &users).await;
        }
        match generated {
            Ok(title) if !title.is_empty() => {
                debug!(session = %session.id, %title, "title generated");
                session.set_title(&title).await;
                // Fan out — a client watching a different session still
                // needs to refresh the sidebar so the renamed row lands
                // without a manual reload.
                state
                    .broadcast_all(ServerMsg::SessionTitleUpdated {
                        session_id: session.id.to_string(),
                        title,
                    })
                    .await;
            }
            Ok(_) => {
                // Model returned an empty title (whitespace / punctuation only).
                // Don't broadcast — leaves the fallback in place.
            }
            Err(e) => {
                warn!(session = %session.id, %e, "title generation failed");
            }
        }
    });
}

/// True when a user message carries titlable content: long enough to mean
/// something and not bare smalltalk. Slash commands count ("/review the
/// diff" is a perfectly good title seed once the slash is stripped).
fn is_substantive(text: &str) -> bool {
    let t = text.trim();
    if t.chars().count() < 12 {
        return false;
    }
    let lower = t.to_lowercase();
    const SMALLTALK: &[&str] = &[
        "hi",
        "hey",
        "hello",
        "yo",
        "huh",
        "thanks",
        "thank you",
        "thx",
        "ok",
        "okay",
        "yes",
        "no",
        "sure",
        "please",
        "sorry",
    ];
    let bare: String = lower
        .trim_end_matches(['!', '.', '?', '…'])
        .trim()
        .to_owned();
    !SMALLTALK.contains(&bare.as_str())
}

/// Strip fenced attachment blobs (`## Attached files` sections) and
/// `@skill:` mention chips down to plain prose for titling.
fn strip_attachments(text: &str) -> &str {
    // Attachment dumps are appended after a recognizable marker; the
    // human-written part comes first.
    for marker in [
        "## Attached files",
        "## attached files",
        "<attachments>",
        "[attachments]",
    ] {
        if let Some(idx) = text.find(marker) {
            return text[..idx].trim_end();
        }
    }
    text
}

pub async fn generate(
    provider: &dyn ChatProvider,
    model: &str,
    users: &[String],
) -> Result<String, String> {
    let system = "You generate short, descriptive titles for chat sessions so the user can \
                  find them later in a sidebar list. Return ONLY the title text — no quotes, \
                  no trailing punctuation, no explanation. Use the SAME language as the user's \
                  messages. Be specific, not vague: name the key entity (file, component, \
                  feature, error) plus the action or goal. Aim for 15-40 characters, never \
                  exceed 60.";
    let numbered: Vec<String> = users
        .iter()
        .take(2)
        .enumerate()
        .map(|(i, u)| format!("{}. {}", i + 1, truncate(u, 800)))
        .collect();
    let user_prompt = format!(
        "User messages:\n\n{}\n\nReturn ONLY the title.",
        numbered.join("\n")
    );
    let req = ChatRequest {
        model: model.to_owned(),
        messages: vec![Message::system(system), Message::user(user_prompt)],
        tools: Vec::new(),
        temperature: Some(0.2),
        // Reasoning-capable models (o-series, gpt-5, etc.) count thinking
        // tokens against this budget; 24 was tight enough that some turns
        // exhausted the cap before emitting any TextDelta and the extractor
        // returned an empty string. 64 leaves headroom without meaningfully
        // changing cost.
        max_tokens: Some(64),
        // Title-generation is a short, low-signal task — don't burn thinking
        // tokens on it even if the current session has effort dialled up.
        reasoning_effort: None,
        service_tier: None,
        response_format: None,
    };
    let mut stream = provider.stream(req).await.map_err(|e| e.to_string())?;
    let mut out = String::new();
    while let Some(ev) = stream.next().await {
        match ev.map_err(|e| e.to_string())? {
            ChatEvent::TextDelta(t) => out.push_str(&t),
            ChatEvent::ToolCalls(_) => {}
            ChatEvent::Usage(_)
            | ChatEvent::RateLimit(_)
            | ChatEvent::ReasoningDelta(_)
            | ChatEvent::Reasoning(_) => {}
            ChatEvent::Done(_) => break,
        }
    }
    Ok(sanitize(&out))
}

fn sanitize(raw: &str) -> String {
    // Take the first non-empty line — models sometimes append explanation
    // despite instructions, and some emit a leading blank/newline before the
    // real title. Strip surrounding quotes, markdown emphasis/heading marks,
    // a "Title:" prefix, and trailing punctuation.
    let line = raw
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let line = line.trim_start_matches(['#']).trim();
    let line = line
        .strip_prefix("Title:")
        .or_else(|| line.strip_prefix("title:"))
        .map(str::trim)
        .unwrap_or(line);
    let stripped: String = line
        .trim_matches(|c: char| c == '"' || c == '\'' || c == '`' || c == '*' || c == '_')
        .trim_end_matches(['.', ',', ':', ';'])
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if stripped.chars().count() > TITLE_CHAR_CAP {
        stripped.chars().take(TITLE_CHAR_CAP).collect()
    } else {
        stripped
    }
}

/// Best-effort title derived from the first substantive user message.
/// Used as a fallback when the model extractor returns an empty string,
/// so the user gets *some* nickname instead of nothing. Leading slash
/// commands are verbalized ("/review the diff" → "Review the diff").
pub fn heuristic_from_user_message(user: &str) -> String {
    let first_line = user
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let without_slash = first_line
        .strip_prefix('/')
        .map(|rest| {
            let mut words = rest.split_whitespace();
            let cmd = words.next().unwrap_or("").replace(['-', '_'], " ");
            let rest = words.collect::<Vec<_>>().join(" ");
            let verbal = if rest.is_empty() {
                cmd
            } else {
                format!("{cmd} {rest}")
            };
            verbal
        })
        .unwrap_or_else(|| first_line.to_owned());
    let words: Vec<&str> = without_slash.split_whitespace().take(6).collect();
    let joined = words.join(" ");
    let trimmed = joined.trim_end_matches(['.', ',', ':', ';']);
    let mut chars = trimmed.chars();
    let cased = match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    };
    if cased.chars().count() > TITLE_CHAR_CAP {
        cased.chars().take(TITLE_CHAR_CAP).collect()
    } else {
        cased
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let head: String = s.chars().take(n).collect();
        format!("{head}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_quotes_and_trailing_punct() {
        assert_eq!(
            sanitize("\"Set up Swift macOS app project.\""),
            "Set up Swift macOS app project"
        );
        assert_eq!(
            sanitize("**Fix video aspect ratios**"),
            "Fix video aspect ratios"
        );
        assert_eq!(
            sanitize("Fix login redirect loop"),
            "Fix login redirect loop"
        );
    }

    #[test]
    fn sanitize_strips_heading_and_title_prefix() {
        assert_eq!(
            sanitize("## Fix login redirect loop"),
            "Fix login redirect loop"
        );
        assert_eq!(
            sanitize("Title: Fix login redirect loop"),
            "Fix login redirect loop"
        );
    }

    #[test]
    fn sanitize_collapses_inner_whitespace() {
        assert_eq!(sanitize("Fix   login\nredirect loop"), "Fix login");
    }

    #[test]
    fn sanitize_takes_first_line() {
        assert_eq!(
            sanitize("Respond to greeting\nExplanation here"),
            "Respond to greeting"
        );
    }

    #[test]
    fn sanitize_skips_leading_blank_lines() {
        assert_eq!(sanitize("\n\nRespond to greeting"), "Respond to greeting");
        assert_eq!(sanitize("   \n\tFix bug\nExtra"), "Fix bug");
    }

    #[test]
    fn heuristic_takes_first_six_words_cased() {
        assert_eq!(
            heuristic_from_user_message("find memory leaks in the map service today"),
            "Find memory leaks in the map"
        );
        assert_eq!(
            heuristic_from_user_message("\n\nfix bug in login."),
            "Fix bug in login"
        );
        assert_eq!(heuristic_from_user_message(""), "");
    }

    #[test]
    fn heuristic_verbalizes_slash_commands() {
        assert_eq!(
            heuristic_from_user_message("/review the diff for race conditions now please"),
            "Review the diff for race conditions"
        );
    }

    #[test]
    fn substantive_rejects_smalltalk_and_shorts() {
        assert!(!is_substantive("huh?"));
        assert!(!is_substantive("hi"));
        assert!(!is_substantive("thanks!"));
        assert!(!is_substantive("ok"));
        assert!(!is_substantive("fix it"));
        assert!(is_substantive("find memory leaks in the map service"));
        assert!(is_substantive("/review the diff for race conditions"));
    }
}
