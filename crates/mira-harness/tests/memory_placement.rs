//! Memory goes out attached to each turn's prompt, chosen once per turn,
//! so what the provider has cached stays a prefix of the next request.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use mira_ai::{ChatEvent, ChatProvider, ChatRequest, FinishReason, ProviderError};
use mira_core::message::{ToolCallFunction, ToolCallKind};
use mira_core::{Message, Role, ToolCall, ToolResult};
use mira_harness::{AutoApprover, Session, SessionConfig};
use mira_memory::{MemoryQuery, MemorySnapshot};
use mira_policy::{Mode, Policy, PolicyConfig};
use mira_tools::{Action, Registry, Tool, ToolContext, ToolError};
use serde_json::json;
use tokio::sync::Mutex;

/// The first request of each turn calls `look`; the next one answers.
#[derive(Default)]
struct Scripted {
    seen: StdMutex<Vec<ChatRequest>>,
}

#[async_trait]
impl ChatProvider for Scripted {
    async fn stream(
        &self,
        request: ChatRequest,
    ) -> Result<BoxStream<'static, Result<ChatEvent, ProviderError>>, ProviderError> {
        let call = request
            .messages
            .last()
            .is_some_and(|m| m.role == Role::User);
        self.seen.lock().unwrap().push(request);
        let events = if call {
            vec![
                ChatEvent::ToolCalls(vec![ToolCall {
                    id: format!("c{}", self.seen.lock().unwrap().len()).into(),
                    kind: ToolCallKind::Function,
                    function: ToolCallFunction {
                        name: "look".into(),
                        arguments: "{}".into(),
                    },
                }]),
                ChatEvent::Done(FinishReason::ToolCalls),
            ]
        } else {
            vec![
                ChatEvent::TextDelta("ok".into()),
                ChatEvent::Done(FinishReason::Stop),
            ]
        };
        Ok(stream::iter(events.into_iter().map(Ok)).boxed())
    }
}

struct Look;

#[async_trait]
impl Tool for Look {
    fn spec(&self) -> mira_ai::ToolSpec {
        mira_ai::ToolSpec {
            name: "look".into(),
            description: "Look around".into(),
            parameters: json!({"type": "object"}),
        }
    }
    fn action(&self) -> Action {
        Action::Read
    }
    async fn invoke(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        Ok(ToolResult::ok(call.id.clone(), "seen"))
    }
}

/// Memory whose text the test sets, counting how often it's chosen.
#[derive(Default)]
struct Memory {
    text: StdMutex<String>,
    renders: AtomicUsize,
}

#[async_trait]
impl MemorySnapshot for Memory {
    async fn render(&self, _query: Option<&MemoryQuery>) -> Option<String> {
        self.renders.fetch_add(1, Ordering::SeqCst);
        Some(self.text.lock().unwrap().clone())
    }
}

fn shape(messages: &[Message]) -> Vec<(Role, Option<String>)> {
    messages
        .iter()
        .map(|m| (m.role, m.content.clone()))
        .collect()
}

fn mentions(m: &Message, text: &str) -> bool {
    m.content.as_deref().is_some_and(|c| c.contains(text))
}

#[tokio::test]
async fn memory_rides_on_each_turns_prompt_and_never_changes_the_prefix() {
    let tmp = tempfile::tempdir().unwrap();
    let provider = Arc::new(Scripted::default());
    let memory = Arc::new(Memory::default());
    *memory.text.lock().unwrap() = "uses pnpm".into();
    let mut registry = Registry::new();
    registry.register(Look);
    let sess = Session::new(
        SessionConfig::new("test-model"),
        "sys",
        provider.clone(),
        Arc::new(registry),
        Arc::new(Mutex::new(
            Policy::from_config(&PolicyConfig {
                mode: Mode::Yolo,
                ..Default::default()
            })
            .unwrap(),
        )),
        Arc::new(AutoApprover { approve_asks: true }),
        ToolContext::new(
            tmp.path(),
            Arc::new(mira_sandbox::Sandbox::default_scrubbed()),
        ),
    )
    .with_memory_snapshot(memory.clone());

    let _: Vec<_> = sess.send("first").await.collect().await;
    let _: Vec<_> = sess.send("second").await.collect().await;
    *memory.text.lock().unwrap() = "uses pnpm; deploys on Fridays".into();
    let _: Vec<_> = sess.send("third").await.collect().await;

    let seen = provider.seen.lock().unwrap();
    assert_eq!(seen.len(), 6, "two requests per turn");
    // Chosen once per turn, not once per request.
    assert_eq!(memory.renders.load(Ordering::SeqCst), 3);
    // Every request extends the one before it: nothing already sent changes.
    for pair in seen.windows(2) {
        let (before, after) = (shape(&pair[0].messages), shape(&pair[1].messages));
        assert_eq!(before, after[..before.len()], "a sent message changed");
    }
    let last = &seen[5].messages;
    // Never in the system prompt.
    assert!(last
        .iter()
        .filter(|m| m.role == Role::System)
        .all(|m| !mentions(m, "pnpm")));
    let prompts: Vec<&Message> = last
        .iter()
        .filter(|m| m.role == Role::User && !mentions(m, "seen"))
        .collect();
    assert_eq!(prompts.len(), 3);
    // On the first prompt; not repeated while unchanged; again once it changed.
    assert!(mentions(
        prompts[0],
        "<memory-context>\nuses pnpm\n</memory-context>"
    ));
    assert!(!mentions(prompts[1], "<memory-context>"));
    assert!(mentions(prompts[2], "deploys on Fridays"));
}
