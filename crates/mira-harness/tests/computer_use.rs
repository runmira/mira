//! End-to-end: a scripted model asks for a screenshot and a click; the
//! harness gates the click (even in yolo), and the screenshot reaches the
//! next provider request as an image.

use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use mira_ai::{ChatEvent, ChatProvider, ChatRequest, FinishReason, ProviderError};
use mira_computer::backend::mock::MockBackend;
use mira_computer::{Computer, ComputerOptions};
use mira_core::message::{ToolCallFunction, ToolCallKind};
use mira_core::{Role, ToolCall};
use mira_harness::{Approver, Session, SessionConfig};
use mira_policy::{Decision, Mode, Policy, PolicyConfig};
use mira_tools::builtin::computer::ComputerTool;
use mira_tools::{Registry, ToolContext};
use tokio::sync::Mutex;

/// Replies with scripted tool calls on the first request, text after,
/// and records every request it sees.
struct Scripted {
    requests: StdMutex<Vec<ChatRequest>>,
}

#[async_trait]
impl ChatProvider for Scripted {
    async fn stream(
        &self,
        request: ChatRequest,
    ) -> Result<BoxStream<'static, Result<ChatEvent, ProviderError>>, ProviderError> {
        let n = {
            let mut r = self.requests.lock().unwrap();
            r.push(request);
            r.len()
        };
        let call = |id: &str, args: &str| ToolCall {
            id: id.into(),
            kind: ToolCallKind::Function,
            function: ToolCallFunction {
                name: "computer".into(),
                arguments: args.into(),
            },
        };
        let events = if n == 1 {
            vec![
                ChatEvent::ToolCalls(vec![
                    call("s1", r#"{"action":"screenshot"}"#),
                    call("c1", r#"{"action":"left_click","coordinate":[10,10]}"#),
                ]),
                ChatEvent::Done(FinishReason::ToolCalls),
            ]
        } else {
            vec![
                ChatEvent::TextDelta("done".into()),
                ChatEvent::Done(FinishReason::Stop),
            ]
        };
        Ok(stream::iter(events.into_iter().map(Ok)).boxed())
    }
}

/// Records which calls it was asked about; denies them all.
#[derive(Default)]
struct Recorder {
    asked: StdMutex<Vec<String>>,
}

#[async_trait]
impl Approver for Recorder {
    async fn approve(&self, call: &ToolCall, _d: Decision) -> bool {
        self.asked.lock().unwrap().push(call.id.to_string());
        false
    }
}

#[tokio::test]
async fn click_is_gated_in_yolo_and_screenshot_reaches_the_model() {
    let tmp = tempfile::tempdir().unwrap();
    let backend = Arc::new(MockBackend::new((800, 600), (800, 600)));
    let computer = Arc::new(Computer::new(
        backend.clone(),
        ComputerOptions {
            settle: Duration::ZERO,
            ..Default::default()
        },
    ));
    let mut registry = Registry::new();
    registry.register(ComputerTool::new(computer, Some((800, 600))));

    let provider = Arc::new(Scripted {
        requests: StdMutex::new(Vec::new()),
    });
    let approver = Arc::new(Recorder::default());
    let policy = Policy::from_config(&PolicyConfig {
        mode: Mode::Yolo,
        ..Default::default()
    })
    .unwrap();
    let sess = Session::new(
        SessionConfig::new("test-model"),
        "sys",
        provider.clone(),
        Arc::new(registry),
        Arc::new(Mutex::new(policy)),
        approver.clone(),
        ToolContext::new(
            tmp.path(),
            Arc::new(mira_sandbox::Sandbox::default_scrubbed()),
        ),
    );

    let mut events = sess.send("look at the screen").await;
    while events.next().await.is_some() {}

    // Yolo auto-allows the screenshot but still asks about the click.
    assert_eq!(*approver.asked.lock().unwrap(), vec!["c1".to_owned()]);
    // The denied click never reached the backend.
    assert!(backend.events().is_empty(), "{:?}", backend.events());

    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    let tool_msgs: Vec<_> = requests[1]
        .messages
        .iter()
        .filter(|m| m.role == Role::Tool)
        .collect();
    assert_eq!(tool_msgs.len(), 2);
    assert_eq!(tool_msgs[0].images.len(), 1, "screenshot attached");
    assert_eq!(tool_msgs[0].images[0].media_type, "image/png");
    assert!(tool_msgs[1]
        .content
        .as_deref()
        .unwrap()
        .contains("denied by policy"));
}
