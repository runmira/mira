//! What the user has picked, and what a turn runs on.
//!
//! A [`ModelSelection`] is the complete answer to "who serves the next
//! turn": one engine instance (the routing key) plus a model id on it,
//! plus any per-model option values (reasoning effort, service tier…).
//!
//! Selections ride on turn starts rather than living only in session
//! config, so a picker can retarget mid-conversation without the caller
//! having to know whether the instance is Mira's own loop or an external
//! agent.

use crate::id::EngineId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One model on one configured engine instance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelSelection {
    /// Routing key. Resolved through the engine registry, never guessed.
    pub instance: EngineId,
    /// Model id on that instance. For external agents this is whatever
    /// the agent's own catalog or `--model` flag calls it.
    pub model: String,
    /// Option values the instance advertised for this model
    /// (`reasoning_effort`, `service_tier`, agent config options…).
    /// Absent keys mean "use the instance default".
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub options: BTreeMap<String, String>,
}

impl ModelSelection {
    pub fn new(instance: impl Into<EngineId>, model: impl Into<String>) -> Self {
        Self {
            instance: instance.into(),
            model: model.into(),
            options: BTreeMap::new(),
        }
    }

    /// Builder-style option setter.
    pub fn with_option(mut self, id: impl Into<String>, value: impl Into<String>) -> Self {
        self.options.insert(id.into(), value.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::EngineId;

    #[test]
    fn selection_serializes_without_noise() {
        let sel = ModelSelection::new(EngineId::new("mira").unwrap(), "claude-sonnet-4-5");
        let v = serde_json::to_value(&sel).unwrap();
        assert_eq!(
            v,
            serde_json::json!({ "instance": "mira", "model": "claude-sonnet-4-5" })
        );
        let back: ModelSelection = serde_json::from_value(v).unwrap();
        assert_eq!(back, sel);
    }

    #[test]
    fn options_round_trip() {
        let sel = ModelSelection::new(EngineId::new("mira").unwrap(), "gpt-5")
            .with_option("reasoning_effort", "high");
        let back: ModelSelection =
            serde_json::from_value(serde_json::to_value(&sel).unwrap()).unwrap();
        assert_eq!(
            back.options.get("reasoning_effort").map(String::as_str),
            Some("high")
        );
    }
}
