//! The newer hook events through real sessions: subagents fire
//! `SubagentStop` (and skip the user-facing events), a tool waiting for
//! approval fires `Notification`, `end()` fires `SessionEnd`, and
//! compaction fires `PreCompact` first.

use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use mira_ai::{ChatEvent, ChatProvider, ChatRequest, FinishReason, ProviderError, ToolSpec};
use mira_core::message::{ToolCallFunction, ToolCallKind};
use mira_core::{Message, SessionId, ToolCall, ToolResult};
use mira_harness::{
    AutoApprover, HarnessEvent, HookEvent, HookOutcome, HookRunner, Session, SessionConfig,
    SessionRecord,
};
use mira_policy::{Mode, Policy, PolicyConfig};
use mira_tools::{Action, Registry, Tool, ToolContext, ToolError};
use serde_json::{json, Value};
use tokio::sync::Mutex;

/// First request: calls `shell` if `call_tool`. Everything else: text.
struct Scripted {
    call_tool: bool,
    calls: StdMutex<usize>,
}

#[async_trait]
impl ChatProvider for Scripted {
    async fn stream(
        &self,
        _request: ChatRequest,
    ) -> Result<BoxStream<'static, Result<ChatEvent, ProviderError>>, ProviderError> {
        let n = {
            let mut c = self.calls.lock().unwrap();
            *c += 1;
            *c
        };
        let events = if self.call_tool && n == 1 {
            vec![
                ChatEvent::ToolCalls(vec![ToolCall {
                    id: "c1".into(),
                    kind: ToolCallKind::Function,
                    function: ToolCallFunction {
                        name: "shell".into(),
                        arguments: "{}".into(),
                    },
                }]),
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

/// A tool the policy asks about in manual mode.
struct Shell;

#[async_trait]
impl Tool for Shell {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "shell".into(),
            description: "shell".into(),
            parameters: json!({"type": "object"}),
        }
    }
    fn action(&self) -> Action {
        Action::Bash
    }
    async fn invoke(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        Ok(ToolResult::ok(call.id.clone(), "ok".to_owned()))
    }
}

#[derive(Default)]
struct Recorder {
    seen: StdMutex<Vec<(HookEvent, String, Value)>>,
}

impl Recorder {
    fn events(&self) -> Vec<HookEvent> {
        self.seen.lock().unwrap().iter().map(|s| s.0).collect()
    }
    fn find(&self, e: HookEvent) -> Option<(String, Value)> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .find(|s| s.0 == e)
            .map(|s| (s.1.clone(), s.2.clone()))
    }
}

#[async_trait]
impl HookRunner for Recorder {
    async fn run(&self, event: HookEvent, target: &str, input: Value) -> HookOutcome {
        self.seen
            .lock()
            .unwrap()
            .push((event, target.to_owned(), input));
        HookOutcome::default()
    }
    fn has(&self, _event: HookEvent) -> bool {
        true
    }
}

fn session(
    provider: Arc<Scripted>,
    mode: Mode,
    dir: &std::path::Path,
    hooks: Arc<Recorder>,
) -> Session {
    let mut registry = Registry::new();
    registry.register(Shell);
    Session::new(
        SessionConfig::new("test-model"),
        "sys",
        provider,
        Arc::new(registry),
        Arc::new(Mutex::new(
            Policy::from_config(&PolicyConfig {
                mode,
                ..Default::default()
            })
            .unwrap(),
        )),
        Arc::new(AutoApprover { approve_asks: true }),
        ToolContext::new(dir, Arc::new(mira_sandbox::Sandbox::default_scrubbed())),
    )
    .with_hooks(hooks)
}

fn scripted(call_tool: bool) -> Arc<Scripted> {
    Arc::new(Scripted {
        call_tool,
        calls: StdMutex::new(0),
    })
}

async fn drain(sess: &Session, prompt: &str) -> Vec<HarnessEvent> {
    sess.send(prompt).await.collect().await
}

#[tokio::test]
async fn top_level_sessions_start_stop_and_end() {
    let tmp = tempfile::tempdir().unwrap();
    let hooks = Arc::new(Recorder::default());
    let sess = session(scripted(false), Mode::Yolo, tmp.path(), hooks.clone());
    drain(&sess, "hi").await;
    assert_eq!(
        hooks.events(),
        [
            HookEvent::SessionStart,
            HookEvent::UserPromptSubmit,
            HookEvent::Stop
        ]
    );

    sess.end("prompt_input_exit").await;
    let (target, input) = hooks.find(HookEvent::SessionEnd).expect("SessionEnd ran");
    assert_eq!(target, "prompt_input_exit");
    assert_eq!(input["reason"], "prompt_input_exit");
}

#[tokio::test]
async fn subagents_fire_subagent_stop_only() {
    let tmp = tempfile::tempdir().unwrap();
    let hooks = Arc::new(Recorder::default());
    let sess = session(scripted(false), Mode::Yolo, tmp.path(), hooks.clone())
        .with_parent_id(SessionId::new());
    assert!(sess.is_subagent());
    drain(&sess, "look around").await;
    sess.end("prompt_input_exit").await;
    assert_eq!(hooks.events(), [HookEvent::SubagentStop]);
}

#[tokio::test]
async fn approval_prompts_notify() {
    let tmp = tempfile::tempdir().unwrap();
    let hooks = Arc::new(Recorder::default());
    let sess = session(scripted(true), Mode::Manual, tmp.path(), hooks.clone());
    drain(&sess, "run it").await;
    // Notification runs in the background.
    for _ in 0..50 {
        if hooks.find(HookEvent::Notification).is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let (target, input) = hooks
        .find(HookEvent::Notification)
        .expect("Notification ran");
    assert_eq!(target, "permission_prompt");
    assert_eq!(input["notification_type"], "permission_prompt");
    assert!(input["message"].as_str().unwrap().contains("shell"));

    // Nothing to ask in yolo mode, so no notification.
    let quiet = Arc::new(Recorder::default());
    let sess = session(scripted(true), Mode::Yolo, tmp.path(), quiet.clone());
    drain(&sess, "run it").await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(quiet.find(HookEvent::Notification).is_none());
}

#[tokio::test]
async fn compaction_fires_pre_compact_first() {
    let tmp = tempfile::tempdir().unwrap();
    let hooks = Arc::new(Recorder::default());
    // A resumed session long enough to be compacted on the next turn.
    let mut messages = vec![Message::system("sys")];
    // ~180k tokens: past 80% of a 200k window.
    for i in 0..70 {
        messages.push(Message::user(format!("u{i} {}", "x".repeat(10_000))));
        messages.push(Message::assistant(format!("a{i}")));
    }
    let record: SessionRecord = serde_json::from_value(json!({
        "id": SessionId::new(),
        "cwd": tmp.path(),
        "cfg": SessionConfig::new("claude-opus-4-7"),
        "messages": messages,
        "created_at": 0,
        "updated_at": 0,
    }))
    .unwrap();
    let mut registry = Registry::new();
    registry.register(Shell);
    let sess = Session::resume_from(
        record,
        scripted(false),
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
    .with_hooks(hooks.clone());

    let events = drain(&sess, "one more").await;
    assert!(events
        .iter()
        .any(|e| matches!(e, HarnessEvent::Compacted { .. })));
    // The model now sees system + summary + the new turn; the transcript
    // still has every earlier message, then the summary as a divider.
    let history = sess.history().await;
    assert!(
        mira_harness::history::is_summary(&history[1]),
        "{:?}",
        history[1].role
    );
    let transcript = sess.transcript().await;
    let summaries: Vec<usize> = transcript
        .iter()
        .enumerate()
        .filter(|(_, m)| mira_harness::history::is_summary(m))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(
        summaries,
        [141],
        "one divider, after the 140 earlier messages"
    );
    assert_eq!(
        transcript[1]
            .content
            .as_deref()
            .map(|c| c.starts_with("u0 ")),
        Some(true)
    );
    let (target, input) = hooks.find(HookEvent::PreCompact).expect("PreCompact ran");
    assert_eq!(target, "auto");
    assert_eq!(input["trigger"], "auto");
    let order = hooks.events();
    let pre = order
        .iter()
        .position(|e| *e == HookEvent::PreCompact)
        .unwrap();
    let stop = order.iter().position(|e| *e == HookEvent::Stop).unwrap();
    assert!(pre < stop);
}
