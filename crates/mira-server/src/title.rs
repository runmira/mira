//! Post-hoc session-nickname generation.
//!
//! After the first assistant reply lands, we spawn a short model call to pick
//! a 3–6 word nickname for the session (like Codex's "Set up Swift macOS app
//! project"). The nickname persists via `Session::set_title`; the next
//! sidebar refresh picks it up. Runs on the same provider the session uses
//! so it stays consistent with the user's configured backend.

use std::sync::Arc;

use futures::StreamExt;
use mira_ai::{ChatEvent, ChatProvider, ChatRequest};
use mira_core::{Message, Role};
use mira_harness::Session;
use tokio::sync::broadcast;
use tracing::{debug, warn};

use crate::protocol::ServerMsg;

const TITLE_CHAR_CAP: usize = 60;

/// Fire-and-forget: if the session lacks a title and has enough context,
/// ask the model for one. Broadcasts `SessionTitleUpdated` on success so
/// connected UIs refresh the sidebar row without waiting for the next
/// `done`. Silent no-op if the session already has a title or lacks
/// enough context.
pub fn spawn_if_needed(
    session: Session,
    provider: Arc<dyn ChatProvider>,
    model: String,
    events_tx: broadcast::Sender<ServerMsg>,
) {
    tokio::spawn(async move {
        if session.title().await.is_some() {
            return;
        }
        let history = session.history().await;
        let user_msg = history
            .iter()
            .find(|m| m.role == Role::User)
            .and_then(|m| m.content.clone());
        let assistant_msg = history
            .iter()
            .find(|m| {
                m.role == Role::Assistant
                    && m.content
                        .as_deref()
                        .map(|s| !s.trim().is_empty())
                        .unwrap_or(false)
            })
            .and_then(|m| m.content.clone());
        let (Some(user), Some(assistant)) = (user_msg, assistant_msg) else {
            // Not enough context — leave title unset. We'll try again after
            // the next turn.
            return;
        };

        match generate(&*provider, &model, &user, &assistant).await {
            Ok(title) if !title.is_empty() => {
                debug!(session = %session.id, %title, "title generated");
                session.set_title(&title).await;
                let _ = events_tx.send(ServerMsg::SessionTitleUpdated {
                    session_id: session.id.to_string(),
                    title,
                });
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

pub async fn generate(
    provider: &dyn ChatProvider,
    model: &str,
    user: &str,
    assistant: &str,
) -> Result<String, String> {
    let system = "You generate short, specific nicknames for chat sessions. You return ONLY the \
                  nickname — no punctuation, no quotes, no explanation. 3-6 words. Prefer imperative \
                  or noun-phrase style like a git commit subject.";
    let user_prompt = format!(
        "First user message:\n\n{}\n\nFirst assistant reply (first 800 chars):\n\n{}\n\nReturn ONLY the nickname.",
        truncate(user, 800),
        truncate(assistant, 800),
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
    };
    let mut stream = provider.stream(req).await.map_err(|e| e.to_string())?;
    let mut out = String::new();
    while let Some(ev) = stream.next().await {
        match ev.map_err(|e| e.to_string())? {
            ChatEvent::TextDelta(t) => out.push_str(&t),
            ChatEvent::ToolCalls(_) => {}
            ChatEvent::Usage(_) => {}
            ChatEvent::Done(_) => break,
        }
    }
    Ok(sanitize(&out))
}

fn sanitize(raw: &str) -> String {
    // Take the first non-empty line — models sometimes append explanation
    // despite instructions, and some emit a leading blank/newline before the
    // real title. Strip surrounding quotes, trailing punctuation, and any
    // wrapping markdown emphasis.
    let line = raw
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let stripped: String = line
        .trim_matches(|c: char| c == '"' || c == '\'' || c == '`' || c == '*' || c == '_')
        .trim_end_matches(|c: char| c == '.' || c == ',' || c == ':' || c == ';')
        .trim()
        .to_string();
    if stripped.chars().count() > TITLE_CHAR_CAP {
        stripped.chars().take(TITLE_CHAR_CAP).collect()
    } else {
        stripped
    }
}

/// Best-effort title derived from the first user message. Used as a fallback
/// when the model extractor returns an empty string, so the user gets *some*
/// nickname instead of a 502.
pub fn heuristic_from_user_message(user: &str) -> String {
    let first_line = user
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let words: Vec<&str> = first_line.split_whitespace().take(6).collect();
    let joined = words.join(" ");
    let trimmed = joined.trim_end_matches(|c: char| c == '.' || c == ',' || c == ':' || c == ';');
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
        assert_eq!(sanitize("Respond to greeting"), "Respond to greeting");
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
}
