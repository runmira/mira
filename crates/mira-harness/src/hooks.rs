//! Lifecycle hooks (Claude Code's model): commands that run at points in
//! a turn and can block, allow, or add context.
//!
//! The harness only knows this interface; `mira-plugins` implements it
//! (plugins' `hooks/hooks.json` and `hooks:` in `~/.mira/mira.yaml`).

use async_trait::async_trait;
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookEvent {
    /// First message of a session. Output is added as context.
    SessionStart,
    /// A user message is about to be sent. Can block it or add context.
    UserPromptSubmit,
    /// Before a tool runs. Can deny, allow, ask, or rewrite its input.
    PreToolUse,
    /// After a tool ran. Feedback is shown to the model.
    PostToolUse,
    /// The agent is about to stop. Blocking makes it continue.
    Stop,
    /// A subagent is about to stop. Blocking makes it continue.
    SubagentStop,
    /// Mira needs the user: a tool call is waiting for approval. The
    /// matcher target is the notification type (`permission_prompt`).
    Notification,
    /// History is about to be summarized to free context. The matcher
    /// target is `auto` (or `manual`).
    PreCompact,
    /// The session is ending (the CLI is exiting). Can't block.
    SessionEnd,
}

impl HookEvent {
    /// The name hook configs and hook input use (`PreToolUse`, …).
    pub fn name(self) -> &'static str {
        match self {
            HookEvent::SessionStart => "SessionStart",
            HookEvent::UserPromptSubmit => "UserPromptSubmit",
            HookEvent::PreToolUse => "PreToolUse",
            HookEvent::PostToolUse => "PostToolUse",
            HookEvent::Stop => "Stop",
            HookEvent::SubagentStop => "SubagentStop",
            HookEvent::Notification => "Notification",
            HookEvent::PreCompact => "PreCompact",
            HookEvent::SessionEnd => "SessionEnd",
        }
    }
}

/// A hook's say on a tool call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HookPermission {
    /// Skip the approval prompt (policy deny rules still apply).
    Allow,
    /// Refuse, with the reason shown to the model.
    Deny(String),
    /// Ask the user even if policy would allow it.
    Ask,
}

/// What the hooks for one event decided, combined.
#[derive(Clone, Debug, Default)]
pub struct HookOutcome {
    /// Blocked, with the reason (shown to the model; for
    /// `UserPromptSubmit` to the user).
    pub block: Option<String>,
    pub permission: Option<HookPermission>,
    /// Replacement tool arguments (`PreToolUse`).
    pub updated_input: Option<Value>,
    /// Extra context for the model.
    pub context: Vec<String>,
    /// Messages for the user (warnings, hook errors).
    pub messages: Vec<String>,
}

#[async_trait]
pub trait HookRunner: Send + Sync {
    /// Run the hooks for `event`. `input` carries the event's fields
    /// (`tool_name` and `tool_input` for tool events, `prompt`, …); the
    /// runner adds the common ones. `matcher_target` is what matchers
    /// match: the tool name, or the SessionStart source.
    async fn run(&self, event: HookEvent, matcher_target: &str, input: Value) -> HookOutcome;

    /// Whether any hook is configured for `event`, so the harness can skip
    /// building input for nothing.
    fn has(&self, event: HookEvent) -> bool;
}
