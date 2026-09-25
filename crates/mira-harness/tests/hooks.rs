//! Lifecycle hooks through a real turn: SessionStart and
//! UserPromptSubmit add context, PreToolUse denies and rewrites calls,
//! PostToolUse adds feedback, and Stop keeps the agent going once.

use std::sync::{Arc, Mutex as StdMutex};

use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use mira_ai::{ChatEvent, ChatProvider, ChatRequest, FinishReason, ProviderError, ToolSpec};
use mira_core::message::{ToolCallFunction, ToolCallKind};
use mira_core::{Role, ToolCall, ToolResult};
use mira_harness::{
    AutoApprover, HarnessEvent, HookEvent, HookOutcome, HookPermission, HookRunner, Session,
    SessionConfig,
};
use mira_policy::{Mode, Policy, PolicyConfig};
use mira_tools::{Action, Registry, Tool, ToolContext, ToolError};
use serde_json::{json, Value};
use tokio::sync::Mutex;

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
        let call = |id: &str, name: &str, args: Value| ToolCall {
            id: id.into(),
            kind: ToolCallKind::Function,
            function: ToolCallFunction {
                name: name.into(),
                arguments: args.to_string(),
            },
        };
        let events = match n {
            1 => vec![
                ChatEvent::ToolCalls(vec![
                    call("a", "echo", json!({"text": "original"})),
                    call("b", "danger", json!({})),
                ]),
                ChatEvent::Done(FinishReason::ToolCalls),
            ],
            _ => vec![
                ChatEvent::TextDelta(format!("reply {n}")),
                ChatEvent::Done(FinishReason::Stop),
            ],
        };
        Ok(stream::iter(events.into_iter().map(Ok)).boxed())
    }
}

struct Echo;

#[async_trait]
impl Tool for Echo {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "echo".into(),
            description: "echo".into(),
            parameters: json!({"type": "object"}),
        }
    }
    fn action(&self) -> Action {
        Action::Pure
    }
    async fn invoke(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let v: Value = serde_json::from_str(&call.function.arguments).unwrap();
        Ok(ToolResult::ok(
            call.id.clone(),
            v["text"].as_str().unwrap_or("").to_owned(),
        ))
    }
}

/// Must never run: a hook denies it.
struct Danger {
    ran: Arc<StdMutex<bool>>,
}

#[async_trait]
impl Tool for Danger {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "danger".into(),
            description: "danger".into(),
            parameters: json!({"type": "object"}),
        }
    }
    fn action(&self) -> Action {
        Action::Pure
    }
    async fn invoke(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        *self.ran.lock().unwrap() = true;
        Ok(ToolResult::ok(call.id.clone(), "ran".to_owned()))
    }
}

#[derive(Default)]
struct TestHooks {
    seen: StdMutex<Vec<(HookEvent, String, Value)>>,
}

#[async_trait]
impl HookRunner for TestHooks {
    async fn run(&self, event: HookEvent, target: &str, input: Value) -> HookOutcome {
        self.seen
            .lock()
            .unwrap()
            .push((event, target.to_owned(), input.clone()));
        let mut out = HookOutcome::default();
        match event {
            HookEvent::SessionStart => out.context.push("session context".into()),
            HookEvent::UserPromptSubmit => out.context.push("prompt context".into()),
            HookEvent::PreToolUse if target == "danger" => {
                out.permission = Some(HookPermission::Deny("not allowed here".into()))
            }
            HookEvent::PreToolUse => {
                out.updated_input = Some(json!({"text": "rewritten"}));
                out.messages.push("rewrote echo".into());
            }
            HookEvent::PostToolUse => out.context.push("looks fine".into()),
            HookEvent::Stop => {
                if input["stop_hook_active"] == false {
                    out.block = Some("run the tests first".into());
                }
            }
        }
        out
    }

    fn has(&self, _event: HookEvent) -> bool {
        true
    }
}

#[tokio::test]
async fn hooks_shape_a_turn() {
    let tmp = tempfile::tempdir().unwrap();
    let ran = Arc::new(StdMutex::new(false));
    let mut registry = Registry::new();
    registry.register(Echo);
    registry.register(Danger { ran: ran.clone() });
    let provider = Arc::new(Scripted {
        requests: StdMutex::new(Vec::new()),
    });
    let hooks = Arc::new(TestHooks::default());
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
        Arc::new(AutoApprover { approve_asks: true }),
        ToolContext::new(
            tmp.path(),
            Arc::new(mira_sandbox::Sandbox::default_scrubbed()),
        ),
    )
    .with_hooks(hooks.clone());

    let mut warnings = Vec::new();
    let mut events = sess.send("do the thing").await;
    while let Some(e) = events.next().await {
        if let HarnessEvent::Warning(w) = e {
            warnings.push(w);
        }
    }

    assert!(!*ran.lock().unwrap(), "the denied tool never ran");
    assert!(
        warnings.iter().any(|w| w.contains("rewrote echo")),
        "{warnings:?}"
    );

    let requests = provider.requests.lock().unwrap();
    // 1: tools · 2: reply (Stop blocks) · 3: reply after "keep going".
    assert_eq!(requests.len(), 3);
    let first_user = requests[0]
        .messages
        .iter()
        .find(|m| m.role == Role::User)
        .and_then(|m| m.content.clone())
        .unwrap();
    assert!(first_user.contains("do the thing"));
    assert!(first_user.contains("session context"));
    assert!(first_user.contains("prompt context"));

    let tools: Vec<String> = requests[1]
        .messages
        .iter()
        .filter(|m| m.role == Role::Tool)
        .filter_map(|m| m.content.clone())
        .collect();
    assert!(tools[0].starts_with("rewritten"), "{tools:?}");
    assert!(
        tools[0].contains("[hook feedback]\nlooks fine"),
        "{tools:?}"
    );
    assert!(
        tools[1].contains("blocked by a hook: not allowed here"),
        "{tools:?}"
    );

    let last_user = requests[2]
        .messages
        .iter()
        .rev()
        .find(|m| m.role == Role::User)
        .and_then(|m| m.content.clone())
        .unwrap();
    assert!(last_user.contains("run the tests first"), "{last_user}");

    // Hook input carries the common fields.
    let seen = hooks.seen.lock().unwrap();
    let pre = seen
        .iter()
        .find(|(e, t, _)| *e == HookEvent::PreToolUse && t == "echo")
        .unwrap();
    assert_eq!(pre.2["tool_input"]["text"], "original");
    assert_eq!(pre.2["permission_mode"], "yolo");
    assert!(pre.2["session_id"].is_string());
    // SessionStart fires once, on the first message only.
    assert_eq!(
        seen.iter()
            .filter(|(e, _, _)| *e == HookEvent::SessionStart)
            .count(),
        1
    );
}
