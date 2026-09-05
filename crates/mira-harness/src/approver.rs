use async_trait::async_trait;
use mira_core::ToolCall;
use mira_policy::Decision;

/// UI seam for `Decision::Ask` outcomes.
///
/// The policy engine says "should we ask?" and the harness delegates the
/// actual asking to an `Approver`. The CLI implementation pops a y/n prompt;
/// a headless runner might auto-approve or auto-deny; a web UI might raise
/// an event and await a click.
#[async_trait]
pub trait Approver: Send + Sync {
    /// Return `true` to allow the call, `false` to skip it. The harness
    /// records the outcome and moves on either way.
    async fn approve(&self, call: &ToolCall, decision: Decision) -> bool;
}

/// Default approver that auto-answers according to a fixed policy —
/// useful for tests and non-interactive runs.
pub struct AutoApprover {
    /// If true, `Ask` becomes allow. If false, `Ask` becomes deny.
    pub approve_asks: bool,
}

#[async_trait]
impl Approver for AutoApprover {
    async fn approve(&self, _call: &ToolCall, decision: Decision) -> bool {
        match decision {
            Decision::Allow => true,
            Decision::Deny => false,
            Decision::Ask => self.approve_asks,
        }
    }
}
