//! Automatic compaction through real sessions: it is measured against the
//! provider's own token counts (as Codex and OpenCode do), and a provider
//! refusing a request as too long gets one compact-and-retry.

use std::sync::{Arc, Mutex as StdMutex};

use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use mira_ai::{ChatEvent, ChatProvider, ChatRequest, FinishReason, ProviderError, TokenUsage};
use mira_core::message::{ToolCallFunction, ToolCallKind};
use mira_core::{Message, SessionId, ToolCall, ToolResult};
use mira_harness::{AutoApprover, HarnessEvent, Session, SessionConfig, SessionRecord};
use mira_policy::{Mode, Policy, PolicyConfig};
use mira_tools::{Action, Registry, Tool, ToolContext, ToolError};
use serde_json::json;
use tokio::sync::Mutex;

/// How each model request (not a summary request) is answered.
#[derive(Clone, Copy)]
enum Script {
    /// First request calls `look` and reports `prompt_tokens`; then text.
    ToolThenText { prompt_tokens: u32 },
    /// The first model request is refused as too long; then text.
    TooLongOnce,
    /// Every model request is refused as too long.
    AlwaysTooLong,
}

struct Scripted {
    script: Script,
    model_requests: StdMutex<usize>,
    /// In order: `model` or `summary`.
    log: StdMutex<Vec<&'static str>>,
}

fn too_long() -> ProviderError {
    ProviderError::Status {
        status: 400,
        body: "invalid_request_error: prompt is too long: 250000 tokens > 200000 maximum".into(),
        retry_after: None,
    }
}

#[async_trait]
impl ChatProvider for Scripted {
    async fn stream(
        &self,
        request: ChatRequest,
    ) -> Result<BoxStream<'static, Result<ChatEvent, ProviderError>>, ProviderError> {
        // The summarizer is sent no tools; a model turn always has some.
        if request.tools.is_empty() {
            self.log.lock().unwrap().push("summary");
            let events = vec![
                ChatEvent::TextDelta("Summary of the work so far.".into()),
                ChatEvent::Done(FinishReason::Stop),
            ];
            return Ok(stream::iter(events.into_iter().map(Ok)).boxed());
        }
        self.log.lock().unwrap().push("model");
        let n = {
            let mut c = self.model_requests.lock().unwrap();
            *c += 1;
            *c
        };
        let events = match self.script {
            Script::ToolThenText { prompt_tokens } if n == 1 => vec![
                ChatEvent::ToolCalls(vec![ToolCall {
                    id: "c1".into(),
                    kind: ToolCallKind::Function,
                    function: ToolCallFunction {
                        name: "look".into(),
                        arguments: "{}".into(),
                    },
                }]),
                ChatEvent::Usage(TokenUsage {
                    prompt_tokens,
                    completion_tokens: 20,
                    ..Default::default()
                }),
                ChatEvent::Done(FinishReason::ToolCalls),
            ],
            Script::TooLongOnce if n == 1 => return Err(too_long()),
            Script::AlwaysTooLong => return Err(too_long()),
            _ => vec![
                ChatEvent::TextDelta("ok".into()),
                ChatEvent::Done(FinishReason::Stop),
            ],
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
        Ok(ToolResult::ok(call.id.clone(), "nothing new"))
    }
}

/// A resumed Claude session (200k window) with `chars` of earlier history.
fn session(provider: Arc<Scripted>, chars: usize, dir: &std::path::Path) -> Session {
    let mut messages = vec![Message::system("sys")];
    let turns = 20;
    for i in 0..turns {
        messages.push(Message::user(format!("u{i} {}", "x".repeat(chars / turns))));
        messages.push(Message::assistant(format!("a{i}")));
    }
    let record: SessionRecord = serde_json::from_value(json!({
        "id": SessionId::new(),
        "cwd": dir,
        "cfg": SessionConfig::new("claude-opus-4-7"),
        "messages": messages,
        "created_at": 0,
        "updated_at": 0,
    }))
    .unwrap();
    let mut registry = Registry::new();
    registry.register(Look);
    Session::resume_from(
        record,
        provider,
        Arc::new(registry),
        Arc::new(Mutex::new(
            Policy::from_config(&PolicyConfig {
                mode: Mode::Yolo,
                ..Default::default()
            })
            .unwrap(),
        )),
        Arc::new(AutoApprover { approve_asks: true }),
        ToolContext::new(dir, Arc::new(mira_sandbox::Sandbox::default_scrubbed())),
    )
}

fn scripted(script: Script) -> Arc<Scripted> {
    Arc::new(Scripted {
        script,
        model_requests: StdMutex::new(0),
        log: StdMutex::new(Vec::new()),
    })
}

fn compactions(events: &[HarnessEvent]) -> usize {
    events
        .iter()
        .filter(|e| matches!(e, HarnessEvent::Compacted { .. }))
        .count()
}

#[tokio::test]
async fn the_providers_own_count_triggers_compaction_that_the_raw_estimate_would_miss() {
    let tmp = tempfile::tempdir().unwrap();
    // ~60k tokens by the 4-chars rule: a third of the window. The provider
    // counts the same request as 190k (its tokenizer, the system prompt and
    // tool definitions), so the next request is near the limit.
    let provider = scripted(Script::ToolThenText {
        prompt_tokens: 190_000,
    });
    let sess = session(provider.clone(), 240_000, tmp.path());
    let events: Vec<_> = sess.send("go on").await.collect().await;
    assert_eq!(compactions(&events), 1, "{events:?}");
    // Summarized before the second model request was sent, not after.
    assert_eq!(*provider.log.lock().unwrap(), ["model", "summary", "model"]);

    // The same history with the provider agreeing with the estimate fits.
    let calm = scripted(Script::ToolThenText {
        prompt_tokens: 60_000,
    });
    let sess = session(calm.clone(), 240_000, tmp.path());
    let events: Vec<_> = sess.send("go on").await.collect().await;
    assert_eq!(compactions(&events), 0);
}

#[tokio::test]
async fn a_request_refused_as_too_long_is_compacted_and_sent_again() {
    let tmp = tempfile::tempdir().unwrap();
    let provider = scripted(Script::TooLongOnce);
    let sess = session(provider.clone(), 40_000, tmp.path());
    let events: Vec<_> = sess.send("go on").await.collect().await;
    assert_eq!(compactions(&events), 1, "{events:?}");
    assert_eq!(*provider.log.lock().unwrap(), ["model", "summary", "model"]);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, HarnessEvent::Token(t) if t == "ok")),
        "the retried request answered: {events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, HarnessEvent::Warning(w) if w.contains("too long"))),
        "a recovered refusal is not reported as a failure"
    );
}

#[tokio::test]
async fn a_second_too_long_refusal_is_reported_instead_of_looping() {
    let tmp = tempfile::tempdir().unwrap();
    let provider = scripted(Script::AlwaysTooLong);
    let sess = session(provider.clone(), 40_000, tmp.path());
    let events: Vec<_> = sess.send("go on").await.collect().await;
    assert_eq!(compactions(&events), 1);
    assert_eq!(*provider.log.lock().unwrap(), ["model", "summary", "model"]);
    assert!(events
        .iter()
        .any(|e| matches!(e, HarnessEvent::Warning(w) if w.contains("too long"))));
}
