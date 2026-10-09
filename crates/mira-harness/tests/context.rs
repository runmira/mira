//! The context inspector against a real session: the breakdown sees a big
//! tool result, and dropping it shrinks the next request the provider gets.

use std::sync::{Arc, Mutex as StdMutex};

use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use mira_ai::{ChatEvent, ChatProvider, ChatRequest, FinishReason, ProviderError};
use mira_core::message::{ToolCallFunction, ToolCallKind};
use mira_core::{Role, ToolCall, ToolResult};
use mira_harness::{AutoApprover, Session, SessionConfig};
use mira_policy::{Mode, Policy, PolicyConfig};
use mira_tools::{Action, Registry, Tool, ToolContext, ToolError};
use serde_json::json;
use tokio::sync::Mutex;

/// First request: call `dump`. Every request is recorded.
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
        let first = {
            let mut seen = self.seen.lock().unwrap();
            seen.push(request);
            seen.len() == 1
        };
        let events = if first {
            vec![
                ChatEvent::ToolCalls(vec![ToolCall {
                    id: "c1".into(),
                    kind: ToolCallKind::Function,
                    function: ToolCallFunction {
                        name: "dump".into(),
                        arguments: r#"{"path":"logs/huge.log"}"#.into(),
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

/// Returns a lot of text.
struct Dump;

#[async_trait]
impl Tool for Dump {
    fn spec(&self) -> mira_ai::ToolSpec {
        mira_ai::ToolSpec {
            name: "dump".into(),
            description: "Print a big file".into(),
            parameters: json!({"type": "object"}),
        }
    }
    fn action(&self) -> Action {
        Action::Read
    }
    async fn invoke(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        Ok(ToolResult::ok(call.id.clone(), "log line\n".repeat(5_000)))
    }
}

fn tool_bytes(req: &ChatRequest) -> usize {
    req.messages
        .iter()
        .filter(|m| m.role == Role::Tool)
        .map(|m| m.content.as_deref().map(str::len).unwrap_or(0))
        .sum()
}

#[tokio::test]
async fn dropping_a_result_shrinks_the_next_request() {
    let tmp = tempfile::tempdir().unwrap();
    let provider = Arc::new(Scripted::default());
    let mut registry = Registry::new();
    registry.register(Dump);
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
    );

    let _: Vec<_> = sess.send("show me the log").await.collect().await;

    // The inspector names the big result, and it dominates the context.
    let b = sess.context_breakdown().await;
    let top = &b.largest_results[0];
    assert_eq!(
        (top.call_id.as_str(), top.tool.as_str(), top.label.as_str()),
        ("c1", "dump", "logs/huge.log")
    );
    let part = |id| b.parts.iter().find(|p| p.id == id).unwrap().tokens;
    assert!(part("tool_results") > part("system") + part("conversation"));

    let before = tool_bytes(provider.seen.lock().unwrap().last().unwrap());
    let dropped = sess.drop_tool_result("c1").await.expect("drop");
    assert_eq!(dropped.tool, "dump");
    assert!(
        dropped.tokens > 1_000,
        "freed only {} tokens",
        dropped.tokens
    );

    let _: Vec<_> = sess.send("thanks").await.collect().await;
    let after = provider
        .seen
        .lock()
        .unwrap()
        .last()
        .map(tool_bytes)
        .unwrap();
    assert!(
        after * 20 < before,
        "next request still carries the result: {after} vs {before} bytes"
    );
    // The model is told what was there and how to get it back.
    let stub = sess
        .history()
        .await
        .into_iter()
        .find(|m| m.role == Role::Tool)
        .and_then(|m| m.content)
        .unwrap();
    assert!(
        stub.contains("Removed from context") && stub.contains("logs/huge.log"),
        "{stub}"
    );

    assert!(sess.drop_tool_result("nope").await.is_err());
}

#[tokio::test]
async fn concurrent_title_and_resume_checkpoints_preserve_both_changes() {
    use mira_harness::persist::{file_store::FileStore, SessionStore, SettleMarks};
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(FileStore::at(tmp.path().join("sessions")).unwrap());
    let sess = Session::new(
        SessionConfig::new("test-model"),
        "sys",
        Arc::new(Scripted::default()),
        Arc::new(Registry::new()),
        Arc::new(Mutex::new(
            Policy::from_config(&PolicyConfig::default()).unwrap(),
        )),
        Arc::new(AutoApprover { approve_asks: true }),
        ToolContext::new(
            tmp.path(),
            Arc::new(mira_sandbox::Sandbox::default_scrubbed()),
        ),
    )
    .with_store(store.clone());
    sess.save_now().await;
    let settle = SettleMarks::set(false);
    tokio::join!(sess.set_title("Generated title"), async {
        sess.set_sidebar_flags(false, None, settle).await;
        sess.save_now().await;
    });
    let saved = store.load(&sess.id).await.unwrap();
    assert_eq!(saved.title.as_deref(), Some("Generated title"));
    assert_eq!(saved.settle, settle);
}
