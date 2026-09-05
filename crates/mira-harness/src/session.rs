use std::sync::Arc;

use futures::{stream::BoxStream, StreamExt};
use mira_ai::{ChatEvent, ChatProvider, ChatRequest, FinishReason};
use mira_core::{Message, SessionId, ToolCall, ToolResult};
use mira_policy::{Decision, Policy, Request as PolicyRequest};
use mira_tools::{Registry, ToolContext};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, Mutex};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{error, info, warn};

use crate::approver::Approver;
use crate::event::HarnessEvent;

/// Runtime configuration for a session. Everything the loop needs besides
/// mutable state.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionConfig {
    pub model: String,
    /// Number of tool-call rounds allowed within one user turn. Guards
    /// against runaway loops (model calling itself forever).
    #[serde(default = "default_max_rounds")]
    pub max_rounds: usize,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
}

fn default_max_rounds() -> usize {
    24
}

impl SessionConfig {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            max_rounds: default_max_rounds(),
            temperature: None,
            max_tokens: None,
        }
    }
}

/// A single conversation.
///
/// All mutable state (history, policy) is behind `Arc<Mutex<_>>` so [`send`]
/// can spawn its loop on a background task and stream events out without
/// holding a mutable borrow on `Session`. Cheap to `Clone` — every field is
/// an `Arc` or a small `Clone` value.
///
/// [`send`]: Session::send
#[derive(Clone)]
pub struct Session {
    pub id: SessionId,
    cfg: Arc<Mutex<SessionConfig>>,
    history: Arc<Mutex<Vec<Message>>>,

    provider: Arc<dyn ChatProvider>,
    registry: Arc<Registry>,
    policy: Arc<Mutex<Policy>>,
    approver: Arc<dyn Approver>,
    tool_ctx: ToolContext,
}

impl Session {
    pub fn new(
        cfg: SessionConfig,
        system_prompt: impl Into<String>,
        provider: Arc<dyn ChatProvider>,
        registry: Arc<Registry>,
        policy: Arc<Mutex<Policy>>,
        approver: Arc<dyn Approver>,
        tool_ctx: ToolContext,
    ) -> Self {
        Self {
            id: SessionId::new(),
            cfg: Arc::new(Mutex::new(cfg)),
            history: Arc::new(Mutex::new(vec![Message::system(system_prompt)])),
            provider,
            registry,
            policy,
            approver,
            tool_ctx,
        }
    }

    /// Snapshot the current history.
    pub async fn history(&self) -> Vec<Message> {
        self.history.lock().await.clone()
    }

    pub async fn config(&self) -> SessionConfig {
        self.cfg.lock().await.clone()
    }

    /// Run one user turn to completion.
    ///
    /// The returned stream ends with [`HarnessEvent::Done`]. Callers may
    /// drop it early to cancel — the task keeps mutating history until it
    /// hits a checkpoint, then exits when the send channel closes.
    pub async fn send(&self, user_input: impl Into<String>) -> BoxStream<'static, HarnessEvent> {
        self.history.lock().await.push(Message::user(user_input));

        let cfg = self.cfg.lock().await.clone();
        let this = self.clone();
        let (tx, rx) = mpsc::channel::<HarnessEvent>(64);

        tokio::spawn(async move { run_loop(this, cfg, tx).await });

        ReceiverStream::new(rx).boxed()
    }
}

// ---- the loop ----

async fn run_loop(sess: Session, cfg: SessionConfig, tx: mpsc::Sender<HarnessEvent>) {
    for round in 0..cfg.max_rounds {
        info!(round, "harness: model turn");

        let req = ChatRequest {
            model: cfg.model.clone(),
            messages: sess.history.lock().await.clone(),
            tools: sess.registry.specs(),
            temperature: cfg.temperature,
            max_tokens: cfg.max_tokens,
        };

        let mut stream = match sess.provider.stream(req).await {
            Ok(s) => s,
            Err(e) => {
                let _ = tx
                    .send(HarnessEvent::Warning(format!("provider error: {e}")))
                    .await;
                let _ = tx.send(HarnessEvent::Done).await;
                return;
            }
        };

        let mut assistant_text = String::new();
        let mut pending_calls: Vec<ToolCall> = Vec::new();
        let mut finish: FinishReason = FinishReason::Other;

        while let Some(evt) = stream.next().await {
            match evt {
                Ok(ChatEvent::TextDelta(t)) => {
                    assistant_text.push_str(&t);
                    if tx.send(HarnessEvent::Token(t)).await.is_err() {
                        return;
                    }
                }
                Ok(ChatEvent::ToolCalls(calls)) => {
                    pending_calls = calls;
                }
                Ok(ChatEvent::Done(reason)) => {
                    finish = reason;
                    break;
                }
                Err(e) => {
                    let _ = tx
                        .send(HarnessEvent::Warning(format!("stream error: {e}")))
                        .await;
                    break;
                }
            }
        }

        // Record the assistant turn — may carry text, tool calls, or both.
        let assistant_msg = if pending_calls.is_empty() {
            Message::assistant(assistant_text.clone())
        } else if assistant_text.is_empty() {
            Message::assistant_calls(pending_calls.clone())
        } else {
            let mut m = Message::assistant(assistant_text.clone());
            m.tool_calls = pending_calls.clone();
            m
        };
        sess.history.lock().await.push(assistant_msg);
        let _ = tx.send(HarnessEvent::TurnComplete).await;

        if pending_calls.is_empty() || finish == FinishReason::Stop {
            let _ = tx.send(HarnessEvent::Done).await;
            return;
        }

        // Dispatch calls. Denials become tool-result messages so the model
        // sees WHY it didn't get a result and can adapt.
        for call in pending_calls {
            let Some(tool) = sess.registry.get(&call.function.name) else {
                let msg = format!("no such tool: {}", call.function.name);
                warn!(tool = %call.function.name, "unknown tool call");
                let result = ToolResult::err(call.id.clone(), msg);
                sess.history
                    .lock()
                    .await
                    .push(Message::tool(result.call_id.clone(), &result.content));
                let _ = tx.send(HarnessEvent::ToolEnd(result)).await;
                continue;
            };

            let target = tool.policy_target(&call);
            let decision = sess.policy.lock().await.evaluate(&PolicyRequest {
                action: tool.action(),
                target: &target,
            });

            let allowed = match decision {
                Decision::Allow => true,
                Decision::Deny => false,
                Decision::Ask => sess.approver.approve(&call, decision).await,
            };

            let _ = tx.send(HarnessEvent::ToolStart(call.clone())).await;

            let result = if !allowed {
                ToolResult::err(
                    call.id.clone(),
                    format!(
                        "denied by policy: {} on `{}`",
                        format_action(tool.action()),
                        target
                    ),
                )
            } else {
                match tool.invoke(&call, &sess.tool_ctx).await {
                    Ok(r) => r,
                    Err(e) => {
                        error!(tool = %call.function.name, %e, "tool invocation failed");
                        ToolResult::err(call.id.clone(), e.to_string())
                    }
                }
            };

            sess.history
                .lock()
                .await
                .push(Message::tool(result.call_id.clone(), &result.content));
            let _ = tx.send(HarnessEvent::ToolEnd(result)).await;
        }
    }

    let _ = tx
        .send(HarnessEvent::Warning(format!(
            "hit max_rounds ({}); stopping",
            cfg.max_rounds
        )))
        .await;
    let _ = tx.send(HarnessEvent::Done).await;
}

fn format_action(a: mira_tools::Action) -> &'static str {
    match a {
        mira_tools::Action::Read => "Read",
        mira_tools::Action::Edit => "Edit",
        mira_tools::Action::Write => "Write",
        mira_tools::Action::Bash => "Bash",
        mira_tools::Action::Pure => "Pure",
    }
}
