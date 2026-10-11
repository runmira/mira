//! Claude prompt ownership and user-facing failures. Kept independent of I/O
//! so recorded protocol streams exercise the same rules as the live reader.
use std::collections::VecDeque;

use serde_json::Value;

use crate::events::{EventSource, MiraEvent, NormalizedEvent};

pub(crate) fn activity(kind: &str, title: &str, detail: String) -> NormalizedEvent {
    NormalizedEvent {
        source: EventSource::Acp {
            variant: kind.into(),
        },
        event: MiraEvent::Activity {
            kind: kind.into(),
            title: title.into(),
            detail,
        },
    }
}

#[derive(Default)]
pub(crate) struct PromptTracker {
    pending: VecDeque<String>,
    echoes: bool,
    lifecycle: bool,
    started: std::collections::HashSet<String>,
    held: Vec<Value>,
    held_bytes: usize,
}

impl PromptTracker {
    pub fn sent(&mut self, id: String) {
        self.pending.push_back(id);
    }

    pub fn forget(&mut self, id: &str) {
        self.pending.retain(|pending| pending != id);
        self.started.remove(id);
        self.held.clear();
        self.held_bytes = 0;
    }

    /// Gate root narration while a lifecycle-capable CLI is dequeuing the
    /// prompt. A background result discards its held narration; a matching
    /// start/result releases genuine prompt frames in their original order.
    pub fn route(&mut self, frame: &Value) -> (bool, Vec<Value>, Option<NormalizedEvent>) {
        let (ours, progress) = self.observe(frame);
        if !ours {
            self.held.clear();
            self.held_bytes = 0;
            return (false, vec![], progress);
        }
        let root = frame.get("parent_tool_use_id").is_none_or(Value::is_null);
        if root
            && matches!(frame["type"].as_str(), Some("assistant" | "stream_event"))
            && self.lifecycle
            && !self.pending.is_empty()
            && self.started.is_empty()
        {
            let bytes = frame.to_string().len();
            if self.held.len() < 256 && self.held_bytes.saturating_add(bytes) <= 1024 * 1024 {
                self.held_bytes += bytes;
                self.held.push(frame.clone());
            } else {
                tracing::warn!("Claude exceeded the pre-acknowledgement frame buffer");
            }
            return (true, vec![], progress);
        }
        let release = frame["type"] == "result"
            || (frame["type"] == "command_lifecycle"
                && frame["state"] == "started"
                && !self.started.is_empty());
        let mut frames = if release {
            {
                self.held_bytes = 0;
                std::mem::take(&mut self.held)
            }
        } else {
            vec![]
        };
        frames.push(frame.clone());
        (true, frames, progress)
    }

    /// Returns false only for a result known to belong to other work. Older
    /// CLIs without echoes still finish ordinary human turns as before.
    pub fn observe(&mut self, frame: &Value) -> (bool, Option<NormalizedEvent>) {
        if frame["type"] == "command_lifecycle" {
            let Some(id) = frame["command_uuid"].as_str() else {
                return (true, None);
            };
            if !self.pending.iter().any(|pending| pending == id) {
                return (true, None);
            }
            self.lifecycle = true;
            if frame["state"] == "started" {
                self.started.insert(id.into());
            }
            let (kind, title) = match frame["state"].as_str() {
                Some("queued") => ("prompt_queued", "Claude queued your message"),
                Some("started") => ("prompt_started", "Claude started your message"),
                Some("completed") => ("prompt_completed", "Claude completed your message"),
                _ => return (true, None),
            };
            return (true, Some(activity(kind, title, String::new())));
        }
        if frame["type"] != "result"
            || frame
                .get("parent_tool_use_id")
                .is_some_and(|p| !p.is_null())
        {
            return (true, None);
        }
        let mut ids: Vec<&str> = frame["user_message_uuids"]
            .as_array()
            .map(|ids| ids.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        if let Some(id) = frame["user_message_uuid"].as_str() {
            ids.push(id);
        }
        if !ids.is_empty() {
            self.echoes = true;
            if !self.pending.iter().any(|id| ids.contains(&id.as_str())) {
                return (false, None);
            }
            self.pending.retain(|id| !ids.contains(&id.as_str()));
            self.started.retain(|id| !ids.contains(&id.as_str()));
        } else {
            let non_human = frame["origin"]["kind"]
                .as_str()
                .is_some_and(|kind| kind != "human");
            if non_human || (self.echoes && !self.pending.is_empty()) {
                return (false, None);
            }
            self.pending.clear();
            self.started.clear();
        }
        (true, None)
    }
}

pub(crate) fn result_failure(frame: &Value) -> Option<NormalizedEvent> {
    let status = frame["api_error_status"].as_u64();
    let reason = frame["terminal_reason"].as_str().unwrap_or("");
    let subtype = frame["subtype"].as_str().unwrap_or("");
    let known_failure = matches!(
        reason,
        "api_error"
            | "blocking_limit"
            | "budget_exhausted"
            | "prompt_too_long"
            | "turn_setup_failed"
            | "rapid_refill_breaker"
            | "malformed_tool_use_exhausted"
            | "structured_output_retry_exhausted"
            | "tool_deferred_unavailable"
            | "image_error"
            | "model_error"
    );
    if frame["is_error"] != true
        && !subtype.starts_with("error")
        && !matches!(status, Some(429 | 529))
        && !known_failure
    {
        return None;
    }
    let retryable = matches!(status, Some(429 | 529)) || reason == "blocking_limit";
    let explanation = match (status, reason) {
        (Some(429), _) | (_, "blocking_limit") => {
            "Claude reached a usage limit. Try again after the limit resets."
        }
        (Some(529), _) => "Claude's service is overloaded. Try again shortly.",
        (_, "prompt_too_long") => {
            "This conversation exceeds Claude's context window. Compact it or start a new chat."
        }
        (_, "budget_exhausted") => "Claude stopped because the turn's token budget was exhausted.",
        (_, "turn_setup_failed") => {
            "Claude could not start this turn. Check its sign-in and configuration."
        }
        (_, "rapid_refill_breaker") => {
            "Claude stopped because the context filled again immediately after compaction."
        }
        (_, "malformed_tool_use_exhausted") => {
            "Claude stopped after repeated malformed tool calls."
        }
        (_, "structured_output_retry_exhausted") => {
            "Claude could not produce the requested structured output."
        }
        (_, "tool_deferred_unavailable") => {
            "Claude could not resume a tool call because the tool is no longer available."
        }
        (_, "image_error") => "Claude could not process an image in this conversation.",
        _ => frame["errors"]
            .as_array()
            .and_then(|errors| {
                errors
                    .iter()
                    .filter_map(Value::as_str)
                    .find(|error| !error.starts_with("[ede_diagnostic]"))
            })
            .or_else(|| frame["result"].as_str())
            .filter(|message| !message.is_empty())
            .unwrap_or("Claude could not complete this turn."),
    };
    let reset_at = frame["resets_at"].as_str().map(str::to_owned);
    let message = match &reset_at {
        Some(reset) => format!("{explanation} Resets {reset}."),
        None => explanation.to_owned(),
    };
    Some(NormalizedEvent {
        source: EventSource::Acp {
            variant: "failure".into(),
        },
        event: MiraEvent::Failure {
            code: status
                .map(|status| format!("api_error_{status}"))
                .unwrap_or_else(|| {
                    if reason.is_empty() {
                        subtype.into()
                    } else {
                        reason.into()
                    }
                }),
            message,
            retryable,
            reset_at,
        },
    })
}

pub(crate) fn retry_activity(frame: &Value) -> NormalizedEvent {
    let attempt = frame["attempt"]
        .as_u64()
        .or_else(|| frame["retry_attempt"].as_u64());
    let status = frame["error_status"].as_u64();
    let cause = match status {
        Some(429) => "Claude reached a rate limit",
        Some(529) => "Claude's service is overloaded",
        _ => "Claude is retrying a failed request",
    };
    let mut detail = frame["message"].as_str().unwrap_or(cause).to_string();
    if let Some(delay) = frame["retry_delay_ms"].as_u64() {
        detail.push_str(&format!(
            "; next attempt in {} seconds",
            delay.div_ceil(1000)
        ));
    }
    activity(
        "api_retry",
        &attempt
            .map(|attempt| format!("Retrying request (attempt {attempt})"))
            .unwrap_or_else(|| "Retrying request".into()),
        detail,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::{NativeAction, StreamState};

    fn replay(fixture: &str) -> (usize, usize, Vec<NormalizedEvent>) {
        let mut tracker = PromptTracker::default();
        let mut stream = StreamState::default();
        let mut ends = 0;
        let mut rejected = 0;
        let mut events = vec![];
        for line in fixture.lines().filter(|line| !line.trim().is_empty()) {
            let record: Value = serde_json::from_str(line).unwrap();
            match record["direction"].as_str().unwrap() {
                "outgoing" => tracker.sent(record["uuid"].as_str().unwrap().into()),
                "cancel" => tracker.forget(record["uuid"].as_str().unwrap()),
                "incoming" => {
                    let (ours, frames, progress) = tracker.route(&record["frame"]);
                    events.extend(progress);
                    if !ours {
                        rejected += 1;
                        continue;
                    }
                    for frame in frames {
                        for action in stream.map(&frame, None) {
                            match action {
                                NativeAction::TurnEnded { .. } => ends += 1,
                                NativeAction::Event(event) => events.push(event),
                                _ => {}
                            }
                        }
                    }
                }
                _ => panic!("unknown replay direction"),
            }
        }
        (ends, rejected, events)
    }

    #[test]
    fn replay_legacy_greeting_without_acknowledgements() {
        let (ends, rejected, events) = replay(include_str!(
            "../tests/fixtures/claude/legacy-greeting.jsonl"
        ));
        assert_eq!((ends, rejected), (1, 0));
        assert!(events.iter().any(
            |event| matches!(&event.event, MiraEvent::AssistantText { text, .. } if text == "Hi!")
        ));
        assert!(!events
            .iter()
            .any(|event| matches!(event.event, MiraEvent::Failure { .. })));
    }

    #[test]
    fn replay_background_result_does_not_complete_the_users_prompt() {
        let (ends, rejected, events) = replay(include_str!(
            "../tests/fixtures/claude/background-before-prompt.jsonl"
        ));
        assert_eq!((ends, rejected), (1, 1));
        let stages: Vec<_> = events
            .iter()
            .filter_map(|event| match &event.event {
                MiraEvent::Activity { kind, .. } => Some(kind.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(stages, ["prompt_queued", "prompt_started"]);
        assert!(!events.iter().any(|event| matches!(&event.event, MiraEvent::AssistantText { text, .. } if text.contains("Background wake"))));
        assert!(events.iter().any(|event| matches!(&event.event, MiraEvent::AssistantText { text, .. } if text == "Your answer")));
    }

    #[test]
    fn replay_cancelled_and_duplicate_results_cannot_end_a_new_turn() {
        let (ends, rejected, _) = replay(include_str!(
            "../tests/fixtures/claude/cancelled-late-result.jsonl"
        ));
        assert_eq!((ends, rejected), (1, 2));
    }

    #[test]
    fn replay_retries_are_progress_and_overload_is_a_failure() {
        let (ends, rejected, events) = replay(include_str!(
            "../tests/fixtures/claude/retry-then-failure.jsonl"
        ));
        assert_eq!((ends, rejected), (1, 0));
        assert!(events.iter().any(|event| matches!(&event.event, MiraEvent::Activity { detail, .. } if detail.contains("2 seconds"))));
        assert!(events.iter().any(|event| matches!(&event.event, MiraEvent::Failure { code, retryable: true, .. } if code == "api_error_529")));
    }

    #[test]
    fn successful_terminal_metadata_is_not_reported_as_a_failure() {
        assert!(result_failure(&serde_json::json!({"subtype":"success","is_error":false,"terminal_reason":"end_turn","api_error_status":200})).is_none());
    }

    #[test]
    fn lifecycle_only_cli_can_finish_without_result_echoes() {
        let mut tracker = PromptTracker::default();
        tracker.sent("p".into());
        tracker.observe(
            &serde_json::json!({"type":"command_lifecycle","command_uuid":"p","state":"started"}),
        );
        assert!(
            tracker
                .observe(&serde_json::json!({"type":"result","subtype":"success"}))
                .0
        );
    }
}
