//! What fills the context window — the data behind the context inspector.
//!
//! There is no tokenizer here: every provider counts differently, and
//! shipping one per model family isn't worth it for a breakdown. Instead
//! each part of a request is sized from its text, and the whole is
//! *calibrated* against what the provider actually reported: when a request
//! is sent its raw estimate is kept, and when the provider reports that
//! request's prompt tokens the ratio between the two scales every later
//! breakdown. So the parts add up to the provider's own count, whichever
//! provider it is, while their proportions come from what's in them.

use mira_ai::ToolSpec;
use mira_core::{Message, Role};
use serde::Serialize;

/// Rough text-to-token rate before calibration: ~4 characters per token
/// for English prose, the usual rule of thumb.
const CHARS_PER_TOKEN: f64 = 4.0;
/// Per-message framing (role markers, separators) providers add.
const MESSAGE_OVERHEAD: u64 = 4;
/// An image's cost before calibration. Real cost depends on its size,
/// which the history doesn't keep; this is a typical screenshot.
const IMAGE_TOKENS: u64 = 1_500;
/// How many tool results to list individually.
const LARGEST_SHOWN: usize = 15;

fn text_tokens(s: &str) -> u64 {
    (s.chars().count() as f64 / CHARS_PER_TOKEN).ceil() as u64
}

/// Raw size of one message, uncalibrated.
fn message_tokens(m: &Message) -> u64 {
    let mut n = MESSAGE_OVERHEAD + m.content.as_deref().map(text_tokens).unwrap_or(0);
    for c in &m.tool_calls {
        n += text_tokens(&c.function.name) + text_tokens(&c.function.arguments);
    }
    n + m.images.len() as u64 * IMAGE_TOKENS
}

fn tools_tokens(tools: &[ToolSpec]) -> u64 {
    tools
        .iter()
        .map(|t| {
            text_tokens(&t.name)
                + text_tokens(&t.description)
                + text_tokens(&t.parameters.to_string())
        })
        .sum()
}

/// Raw estimate of a whole request, as sent.
pub fn estimate_request(messages: &[Message], tools: &[ToolSpec]) -> u64 {
    messages.iter().map(message_tokens).sum::<u64>() + tools_tokens(tools)
}

/// The last request's raw estimate and what the provider counted for it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Calibration {
    pub estimated: u64,
    pub reported: Option<u64>,
}

impl Calibration {
    /// Reported over estimated, kept within sane bounds so one odd report
    /// (a cached-prompt provider counting differently, a partial stream)
    /// can't make the breakdown absurd.
    fn ratio(&self) -> Option<f64> {
        let reported = self.reported? as f64;
        if self.estimated == 0 || reported == 0.0 {
            return None;
        }
        Some((reported / self.estimated as f64).clamp(0.4, 3.0))
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ContextPart {
    /// `system`, `memory`, `tools`, `conversation`, `tool_results`.
    pub id: &'static str,
    pub label: &'static str,
    pub tokens: u64,
}

/// One tool result in the context, as the inspector lists it.
#[derive(Clone, Debug, Serialize)]
pub struct ToolResultSize {
    pub call_id: String,
    pub tool: String,
    /// What it was about: a path, a command, a query.
    pub label: String,
    pub tokens: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct ContextBreakdown {
    /// The parts, summing to `total`.
    pub parts: Vec<ContextPart>,
    pub total: u64,
    /// True when calibrated against a provider report; false means a plain
    /// estimate (no request sent yet this session).
    pub calibrated: bool,
    /// The provider's count for the last request, for reference.
    pub last_reported: Option<u64>,
    /// The largest tool results, biggest first.
    pub largest_results: Vec<ToolResultSize>,
}

/// A short description of what a tool call was about.
pub fn describe_call(name: &str, args: &str) -> String {
    let v: serde_json::Value = serde_json::from_str(args).unwrap_or_default();
    let pick = [
        "path",
        "file_path",
        "command",
        "pattern",
        "query",
        "url",
        "prompt",
        "action",
    ]
    .iter()
    .find_map(|k| v.get(*k).and_then(|x| x.as_str()))
    .unwrap_or(name);
    let one_line = pick.lines().next().unwrap_or("").trim();
    if one_line.chars().count() > 80 {
        format!("{}…", one_line.chars().take(79).collect::<String>())
    } else {
        one_line.to_string()
    }
}

/// Break a request down. `messages` is what would be sent now (memory
/// block included, old results already cleared), `tools` the specs.
pub fn breakdown(messages: &[Message], tools: &[ToolSpec], cal: Calibration) -> ContextBreakdown {
    // Which tool each result answers, from the assistant turns that asked.
    let mut calls = std::collections::HashMap::new();
    for m in messages {
        for c in &m.tool_calls {
            calls.insert(
                c.id.as_str().to_string(),
                (c.function.name.clone(), c.function.arguments.clone()),
            );
        }
    }

    let (mut system, mut memory, mut conversation, mut results) = (0u64, 0u64, 0u64, 0u64);
    let mut seen_system = false;
    let mut sizes = Vec::new();
    for m in messages {
        let n = message_tokens(m);
        match m.role {
            // The first system message is the system prompt; later ones are
            // injected blocks (memory and skills).
            Role::System if !seen_system => {
                seen_system = true;
                system += n;
            }
            Role::System => memory += n,
            Role::Tool => {
                results += n;
                let id = m
                    .tool_call_id
                    .as_ref()
                    .map(|i| i.as_str().to_string())
                    .unwrap_or_default();
                let (tool, args) = calls.get(&id).cloned().unwrap_or_default();
                sizes.push(ToolResultSize {
                    label: describe_call(&tool, &args),
                    tool,
                    call_id: id,
                    tokens: n,
                });
            }
            _ => conversation += n,
        }
    }
    let tools_n = tools_tokens(tools);

    let ratio = cal.ratio();
    let scale = |n: u64| (n as f64 * ratio.unwrap_or(1.0)).round() as u64;
    let parts = vec![
        ContextPart {
            id: "system",
            label: "System prompt",
            tokens: scale(system),
        },
        ContextPart {
            id: "tools",
            label: "Tools",
            tokens: scale(tools_n),
        },
        ContextPart {
            id: "memory",
            label: "Memory & skills",
            tokens: scale(memory),
        },
        ContextPart {
            id: "conversation",
            label: "Conversation",
            tokens: scale(conversation),
        },
        ContextPart {
            id: "tool_results",
            label: "Tool results",
            tokens: scale(results),
        },
    ];
    sizes.sort_by_key(|s| std::cmp::Reverse(s.tokens));
    sizes.truncate(LARGEST_SHOWN);
    for s in &mut sizes {
        s.tokens = scale(s.tokens);
    }
    ContextBreakdown {
        total: parts.iter().map(|p| p.tokens).sum(),
        parts,
        calibrated: ratio.is_some(),
        last_reported: cal.reported,
        largest_results: sizes,
    }
}

/// What a dropped result is replaced with: enough for the model to know
/// something was there and how to get it back.
pub fn dropped_stub(tool: &str, label: &str, tokens: u64) -> String {
    let what = if label.is_empty() || label == tool {
        tool.to_string()
    } else {
        format!("{tool} ({label})")
    };
    format!(
        "[Removed from context by the user to save space: ~{tokens} tokens of {what} output. \
         Run the tool again if you need it.]"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use mira_core::{ToolCall, ToolCallFunction, ToolCallId, ToolCallKind};

    fn call(id: &str, name: &str, args: &str) -> ToolCall {
        ToolCall {
            id: ToolCallId::from(id),
            kind: ToolCallKind::Function,
            function: ToolCallFunction {
                name: name.into(),
                arguments: args.into(),
            },
        }
    }

    fn convo() -> (Vec<Message>, Vec<ToolSpec>) {
        let mut asked = Message::assistant("");
        asked.tool_calls = vec![
            call("c1", "read_file", r#"{"path":"src/big.rs"}"#),
            call("c2", "bash", r#"{"command":"cargo test"}"#),
        ];
        let msgs = vec![
            Message::system("You are Mira.".repeat(50)),
            Message::system("## Memory\n".to_string() + &"fact ".repeat(100)),
            Message::user("fix the bug"),
            asked,
            Message::tool(ToolCallId::from("c1"), "x".repeat(8_000)),
            Message::tool(ToolCallId::from("c2"), "y".repeat(2_000)),
            Message::assistant("done"),
        ];
        let tools = vec![ToolSpec {
            name: "read_file".into(),
            description: "Read a file".into(),
            parameters: serde_json::json!({"type":"object"}),
        }];
        (msgs, tools)
    }

    #[test]
    fn parts_sum_to_the_reported_count() {
        let (msgs, tools) = convo();
        let raw = estimate_request(&msgs, &tools);
        // The provider counted 30% more than the raw estimate.
        let reported = (raw as f64 * 1.3) as u64;
        let b = breakdown(
            &msgs,
            &tools,
            Calibration {
                estimated: raw,
                reported: Some(reported),
            },
        );
        assert!(b.calibrated);
        let off = (b.total as f64 - reported as f64).abs() / reported as f64;
        assert!(off < 0.05, "total {} vs reported {reported}", b.total);
    }

    #[test]
    fn results_are_attributed_and_ranked() {
        let (msgs, tools) = convo();
        let b = breakdown(&msgs, &tools, Calibration::default());
        assert!(!b.calibrated);
        let top: Vec<_> = b
            .largest_results
            .iter()
            .map(|r| (r.call_id.as_str(), r.tool.as_str(), r.label.as_str()))
            .collect();
        assert_eq!(
            top,
            vec![
                ("c1", "read_file", "src/big.rs"),
                ("c2", "bash", "cargo test")
            ]
        );
        let part = |id| b.parts.iter().find(|p| p.id == id).unwrap().tokens;
        assert!(part("tool_results") > part("conversation"));
        assert!(part("memory") > 0 && part("system") > 0 && part("tools") > 0);
    }

    #[test]
    fn a_wild_report_cannot_distort_the_breakdown() {
        let (msgs, tools) = convo();
        let raw = estimate_request(&msgs, &tools);
        let b = breakdown(
            &msgs,
            &tools,
            Calibration {
                estimated: raw,
                reported: Some(raw * 100),
            },
        );
        assert!(b.total <= raw * 3 + 5);
    }
}
