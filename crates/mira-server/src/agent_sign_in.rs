//! Which external-agent models need a sign-in, as Mira has observed.
//!
//! An agent that starts is not necessarily signed in. OpenCode initializes
//! with no provider credentials and only fails when a turn runs, so its
//! probe alone reports "ready". This registry remembers each turn that
//! failed with an authentication error, keyed by engine instance and model,
//! and overlays it onto agent status so Settings says "Sign-in needed"
//! instead of "Ready".
//!
//! Keyed by model, not provider: OpenCode v2.0.22 lists free `opencode/*`
//! models together, yet one of them can answer while another of them
//! demands authentication. A success on one model proves nothing about its
//! neighbours. A later successful turn on that model, or a successful
//! sign-in from Settings, clears the entry.
use mira_acp::status::{model_provider, AgentStatus, SignInNeeded, SignInSource};
use std::collections::BTreeMap;
use std::sync::Mutex;

#[derive(Default)]
pub struct SignInRegistry {
    /// instance → model (`None` when the chat's model is unknown) → what
    /// was observed.
    needs: Mutex<BTreeMap<String, BTreeMap<Option<String>, SignInNeeded>>>,
}

impl SignInRegistry {
    /// A turn on `instance` with `model` failed with a sign-in error.
    /// Returns true when this is news (so callers republish status).
    pub fn turn_failed(&self, instance: &str, model: Option<&str>, error: &str) -> bool {
        let provider = model.and_then(model_provider).map(str::to_owned);
        let need = SignInNeeded {
            provider: provider.clone(),
            model: model.map(str::to_owned),
            source: SignInSource::Turn,
            reason: error
                .lines()
                .next()
                .unwrap_or(error)
                .chars()
                .take(240)
                .collect(),
        };
        let mut needs = self.needs.lock().unwrap_or_else(|e| e.into_inner());
        let slot = needs.entry(instance.to_owned()).or_default();
        slot.insert(model.map(str::to_owned), need.clone()).as_ref() != Some(&need)
    }

    /// A turn on `instance` with `model` completed, so that model (and an
    /// unattributed need, which this turn disproves) can answer.
    pub fn turn_succeeded(&self, instance: &str, model: Option<&str>) -> bool {
        let mut needs = self.needs.lock().unwrap_or_else(|e| e.into_inner());
        let Some(slot) = needs.get_mut(instance) else {
            return false;
        };
        let before = slot.len();
        slot.remove(&model.map(str::to_owned));
        slot.remove(&None);
        let changed = slot.len() != before;
        if slot.is_empty() {
            needs.remove(instance);
        }
        changed
    }

    /// The user signed in from Settings: forget what turns observed.
    pub fn signed_in(&self, instance: &str) {
        self.needs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(instance);
    }

    pub fn for_instance(&self, instance: &str) -> Vec<SignInNeeded> {
        self.needs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(instance)
            .map(|m| m.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Add observed needs to a probed status, without listing one twice.
    pub fn overlay(&self, instance: &str, status: &mut AgentStatus) {
        for need in self.for_instance(instance) {
            if !status
                .sign_in_needed
                .iter()
                .any(|n| n.provider == need.provider && n.model == need.model)
            {
                status.sign_in_needed.push(need);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AUTH: &str = "acp: agent returned an error: -32000 Authentication required: provider authentication required";

    #[test]
    fn a_failed_turn_marks_its_model_until_a_turn_on_it_succeeds() {
        let r = SignInRegistry::default();
        assert!(r.turn_failed("opencode", Some("opencode/cat-jam-free"), AUTH));
        assert!(
            !r.turn_failed("opencode", Some("opencode/cat-jam-free"), AUTH),
            "same need is not news"
        );
        r.turn_failed("opencode", Some("anthropic/claude-x"), AUTH);
        let needs = r.for_instance("opencode");
        assert_eq!(needs.len(), 2);
        assert_eq!(needs[0].provider.as_deref(), Some("anthropic"));
        assert_eq!(needs[0].source, SignInSource::Turn);
        // A neighbouring free model working proves nothing about this one.
        assert!(!r.turn_succeeded("opencode", Some("opencode/big-pickle")));
        assert_eq!(r.for_instance("opencode").len(), 2);
        assert!(r.turn_succeeded("opencode", Some("opencode/cat-jam-free")));
        assert_eq!(r.for_instance("opencode").len(), 1);
        assert!(
            r.for_instance("opencode-work").is_empty(),
            "instances are separate"
        );
        r.signed_in("opencode");
        assert!(r.for_instance("opencode").is_empty());
    }

    #[test]
    fn an_unattributed_need_is_cleared_by_any_successful_turn() {
        let r = SignInRegistry::default();
        r.turn_failed("claude-code", None, "Not logged in · Please run /login");
        assert_eq!(r.for_instance("claude-code")[0].provider, None);
        assert!(r.turn_succeeded("claude-code", Some("opus")));
        assert!(r.for_instance("claude-code").is_empty());
    }
}
