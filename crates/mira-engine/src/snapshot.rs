//! Health snapshots — what a picker needs to render one row.
//!
//! The list a user sees has to answer, per instance: is it configured,
//! is it installed, is it authenticated, and what models does it offer.
//! Every backend maps into the same small set of states so the UI can
//! render them identically — a native provider missing its API key and
//! a Codex CLI that was never installed are the same visual problem.

use crate::id::{DriverKind, EngineId};
use mira_ai::ModelInfo;
use serde::{Deserialize, Serialize};

/// Which half of the system serves this instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineFlavor {
    /// Mira's own loop over an LLM provider.
    Native,
    /// An external agent process (ACP / app-server).
    External,
}

/// Health of one instance, right now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum EngineState {
    /// Usable as-is.
    Ready,
    /// Known driver, but config is incomplete (missing key, missing
    /// base URL). `reason` is user-facing.
    NotConfigured { reason: String },
    /// Not on `PATH` and no path configured. `looked_for` names the
    /// binary that was searched for.
    NotFound { looked_for: String },
    /// Present but broken: failed to launch or to complete its
    /// handshake. `reason` is user-facing.
    Failed { reason: String },
    /// This build does not know the driver named in config. The
    /// instance is preserved (and still listed) so config written by a
    /// newer build or a fork survives; it just can't run here.
    Unavailable { reason: String },
}

impl EngineState {
    /// True when a turn could start on this instance right now.
    pub fn is_ready(&self) -> bool {
        matches!(self, EngineState::Ready)
    }
}

/// Everything a picker shows for one engine instance.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EngineSnapshot {
    pub instance: EngineId,
    pub driver: DriverKind,
    pub flavor: EngineFlavor,
    pub display_name: String,
    /// The user's opt-in. Health is reported regardless — see
    /// `mira_acp::status` for why conflating the two was a bug.
    pub enabled: bool,
    pub state: EngineState,
    /// The instance's model catalog, when one is known. Empty for
    /// agents that advertise none (the caller falls back to free text)
    /// and for states that can't reach a catalog.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<ModelInfo>,
    /// Model to start on when the user picks this instance without
    /// naming one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
    /// Human auth summary, e.g. "Claude subscription · a@b.com".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<String>,
    /// How to install what's missing, when the backend knows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_hint: Option<String>,
    /// The resolved launch command, credentials redacted. External
    /// agents only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch: Option<String>,
    /// Set when another enabled instance of the same driver resolves to
    /// the same credential boundary: the two sign in as one account
    /// however different the rest of their config is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_note: Option<String>,
    /// Native instances: how prompts are cached. `markers` (Mira marks
    /// prompts for the provider's cache), `automatic` (the provider caches on
    /// its own) or `off`. Absent for external agents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_caching: Option<String>,
    /// External agents: the full agent status (versions, transport, sign-in
    /// methods, install hint), so clients read agent health from this one
    /// list instead of a second probe (#85).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<mira_acp::status::AgentStatus>,
}

impl EngineSnapshot {
    /// True when the instance can take a turn right now.
    pub fn is_ready(&self) -> bool {
        self.enabled && self.state.is_ready()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_serialize_with_a_discriminator() {
        let v = serde_json::to_value(EngineState::NotConfigured {
            reason: "no api_key".into(),
        })
        .unwrap();
        assert_eq!(
            v,
            serde_json::json!({ "state": "not_configured", "reason": "no api_key" })
        );
        let back: EngineState = serde_json::from_value(v).unwrap();
        assert_eq!(
            back,
            EngineState::NotConfigured {
                reason: "no api_key".into()
            }
        );
    }
}
