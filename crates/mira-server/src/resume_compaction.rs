//! One preflight question per resumed Claude process, with a per-chat opt-out.
use crate::{
    acp_session::{with_prompt_card, SlotAgent},
    slot::SessionSlot,
};
use mira_acp::native::AgentHandle;
use mira_tools::prompt::{
    AskUserOption, AskUserProposal, AskUserQuestion, PromptRequest, PromptResponse,
};
use serde_json::{json, Value};
use std::sync::{atomic::Ordering, Arc};

const COMPACT: &str = "Compact first";
const CONTINUE: &str = "Continue";
const NEVER: &str = "Continue and never ask in this chat";

fn near_limit(usage: &Value) -> bool {
    let (Some(total), Some(threshold)) = (
        usage["totalTokens"].as_u64(),
        usage["autoCompactThreshold"].as_u64(),
    ) else {
        return false;
    };
    threshold > 0 && total >= threshold.saturating_mul(9) / 10
}

#[derive(Debug, PartialEq)]
enum Choice {
    Compact,
    Continue,
    Never,
}
fn choice(response: Option<PromptResponse>) -> Choice {
    match response {
        Some(PromptResponse::AskUser(response)) if !response.cancelled => {
            match response
                .answers
                .first()
                .and_then(|a| a.picked.first())
                .map(String::as_str)
            {
                Some(COMPACT) => Choice::Compact,
                Some(NEVER) => Choice::Never,
                _ => Choice::Continue,
            }
        }
        _ => Choice::Continue,
    }
}

pub(crate) async fn before_prompt(
    slot: &Arc<SessionSlot>,
    handle: &SlotAgent,
    agent: &AgentHandle,
) -> Result<(), String> {
    let AgentHandle::Native(native) = agent else {
        return Ok(());
    };
    if handle.driver_kind != "claude-code"
        || !handle
            .resume_compaction_pending
            .swap(false, Ordering::SeqCst)
    {
        return Ok(());
    }
    let session = slot.session.read().await.clone();
    if session.config().await.skip_claude_resume_compaction_prompt {
        return Ok(());
    }
    // Older clients may not support the control request. Missing counts never
    // become estimates that unexpectedly block a resume.
    let Ok(usage) = native.context_usage().await else {
        return Ok(());
    };
    if !near_limit(&usage) {
        return Ok(());
    }
    let question = AskUserQuestion {
        question: "This resumed Claude conversation is close to its context limit. Compact it before sending your message?".into(),
        header: Some("Context".into()), multi_select:false,
        options: vec![
            AskUserOption {label:COMPACT.into(),description:Some("Summarize older context, then send your pending message.".into()),recommended:true},
            AskUserOption {label:CONTINUE.into(),description:Some("Send your message with the existing context.".into()),recommended:false},
            AskUserOption {label:NEVER.into(),description:Some("Keep resuming this chat without this question.".into()),recommended:false},
        ],
    };
    let proposal = AskUserProposal {
        questions: vec![question],
    };
    let args = json!(proposal);
    let channel = slot.prompt_channel.clone();
    let pending = slot.prompt_pending.clone();
    let selected = with_prompt_card(slot, "ask_user", args, |id| async move {
        // Clean pending registrations even if Stop drops the preflight future.
        struct Pending {
            id: String,
            pending: crate::interactive::PendingPromptMap,
        }
        impl Drop for Pending {
            fn drop(&mut self) {
                let pending = self.pending.clone();
                let id = self.id.clone();
                tokio::spawn(async move {
                    pending.lock().await.remove(&id);
                });
            }
        }
        let _pending = Pending {
            id: id.clone(),
            pending,
        };
        let selected = choice(channel.ask(id, PromptRequest::AskUser(proposal)).await);
        let label = match selected {
            Choice::Compact => COMPACT,
            Choice::Continue => CONTINUE,
            Choice::Never => NEVER,
        };
        (selected, label.to_string())
    })
    .await;
    match selected {
        Choice::Never => session.skip_claude_resume_compaction_prompt().await,
        Choice::Compact => native
            .compact_before_prompt()
            .await
            .map_err(|e| format!("Could not compact Claude; your message was not sent. {e}"))?,
        Choice::Continue => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mira_tools::prompt::{AskUserAnswer, AskUserResponse};
    #[test]
    fn only_reported_near_limit_sessions_prompt() {
        assert!(!near_limit(&json!({})));
        assert!(!near_limit(
            &json!({"totalTokens":1,"autoCompactThreshold":0})
        ));
        assert!(!near_limit(
            &json!({"totalTokens":161999,"autoCompactThreshold":180000})
        ));
        assert!(near_limit(
            &json!({"totalTokens":162000,"autoCompactThreshold":180000})
        ));
    }
    #[test]
    fn only_an_explicit_choice_compacts_or_remembers_opt_out() {
        for (label, expected) in [
            (COMPACT, Choice::Compact),
            (CONTINUE, Choice::Continue),
            (NEVER, Choice::Never),
            ("other", Choice::Continue),
        ] {
            let response = PromptResponse::AskUser(AskUserResponse {
                answers: vec![AskUserAnswer {
                    picked: vec![label.into()],
                    custom: None,
                }],
                cancelled: false,
            });
            assert_eq!(choice(Some(response)), expected);
        }
        assert_eq!(choice(None), Choice::Continue);
    }
}
