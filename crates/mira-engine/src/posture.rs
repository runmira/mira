//! The permission-posture vocabulary, in one place.
//!
//! "What may the agent do without asking" is the same question for
//! Mira's own harness and for every external agent, but the answers
//! used to be named differently at each layer — harness `Mode`s, ACP
//! mode ids, and a frontend regex table that matched agent mode names.
//! This module is the single fixed vocabulary; everything maps *to* a
//! posture and clients never pattern-match raw mode ids again.

use mira_policy::Mode;
use serde::{Deserialize, Serialize};

/// The five postures, in escalating order of authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Posture {
    /// The agent plans and explains, but changes nothing.
    Plan,
    /// The agent asks before every consequential action.
    Ask,
    /// File edits go through; commands still ask.
    Edits,
    /// The agent acts on its own judgement within the workspace.
    Auto,
    /// No prompts. Everything the agent attempts is allowed.
    Yolo,
}

impl Posture {
    /// Every posture, escalation order. The order is part of the
    /// contract: pickers render top to bottom as a trust ladder.
    pub const ALL: [Posture; 5] = [
        Posture::Plan,
        Posture::Ask,
        Posture::Edits,
        Posture::Auto,
        Posture::Yolo,
    ];

    /// Short label, e.g. "Plan only".
    pub fn label(self) -> &'static str {
        match self {
            Posture::Plan => "Plan only",
            Posture::Ask => "Ask each time",
            Posture::Edits => "Auto edits",
            Posture::Auto => "Auto everything",
            Posture::Yolo => "YOLO",
        }
    }

    /// One line on what this means in practice.
    pub fn blurb(self) -> &'static str {
        match self {
            Posture::Plan => "The agent plans and explains, but changes nothing.",
            Posture::Ask => "The agent asks before every consequential action.",
            Posture::Edits => "File edits go through; commands still ask.",
            Posture::Auto => "The agent acts on its own judgement within the workspace.",
            Posture::Yolo => "No prompts. Everything the agent attempts is allowed.",
        }
    }

    /// Which posture a harness [`Mode`] speaks. (`manual` is "ask each
    /// time", `auto` is "auto edits", `edit` is "auto everything" — the
    /// names disagree but the intent lines up, and this mapping is what
    /// lets one picker drive both systems.)
    pub fn from_harness_mode(mode: Mode) -> Posture {
        match mode {
            Mode::Plan => Posture::Plan,
            Mode::Manual => Posture::Ask,
            Mode::Auto => Posture::Edits,
            Mode::Edit => Posture::Auto,
            Mode::Yolo => Posture::Yolo,
        }
    }

    /// The harness [`Mode`] that means this posture.
    pub fn to_harness_mode(self) -> Mode {
        match self {
            Posture::Plan => Mode::Plan,
            Posture::Ask => Mode::Manual,
            Posture::Edits => Mode::Auto,
            Posture::Auto => Mode::Edit,
            Posture::Yolo => Mode::Yolo,
        }
    }

    /// Does an agent's own mode (id or human name) mean this posture?
    ///
    /// Matching is case-insensitive and intentionally forgiving — agent
    /// vendors name modes freely (`acceptEdits`, `agent-full-access`,
    /// `dontask`). First match wins, so callers must offer postures in
    /// [`Posture::ALL`] order.
    pub fn matches_mode(self, mode_id: &str, mode_name: &str) -> bool {
        let id = mode_id.trim();
        let name = mode_name.trim();
        match self {
            Posture::Plan => {
                contains_ci(id, ["plan", "read-only", "readonly"])
                    || contains_ci(name, ["plan", "read-only", "readonly"])
            }
            Posture::Ask => {
                equals_ci(id, ["default", "ask", "agent", "approval-required"])
                    || equals_ci(name, ["default", "ask", "agent", "approval-required"])
            }
            Posture::Edits => contains_ci(id, ["accept"]) || contains_ci(name, ["accept"]),
            Posture::Auto => equals_ci(id, ["auto"]) || equals_ci(name, ["auto"]),
            Posture::Yolo => {
                contains_ci(
                    id,
                    [
                        "dontask",
                        "don't ask",
                        "never-ask",
                        "bypass",
                        "full-access",
                        "full_access",
                        "danger",
                    ],
                ) || contains_ci(
                    name,
                    [
                        "dontask",
                        "don't ask",
                        "never-ask",
                        "bypass",
                        "full-access",
                        "full_access",
                        "danger",
                    ],
                )
            }
        }
    }
}

/// Case-insensitive "does the haystack contain any of these needles".
fn contains_ci<const N: usize>(haystack: &str, needles: [&str; N]) -> bool {
    let h = haystack.to_ascii_lowercase();
    needles.iter().any(|n| h.contains(n))
}

/// Case-insensitive "does the haystack equal any of these".
fn equals_ci<const N: usize>(haystack: &str, needles: [&str; N]) -> bool {
    let h = haystack.trim().to_ascii_lowercase();
    needles.iter().any(|n| *n == h)
}

/// One posture bound to the agent mode that means it — what the picker
/// renders, computed by the server so no client ever regex-matches mode
/// names itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostureMapping {
    pub key: Posture,
    /// The agent's mode id this posture maps to.
    pub mode_id: String,
    /// The agent's own name for it, when it differs from the posture label.
    pub mode_name: String,
    /// The agent's own description, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode_description: Option<String>,
    /// True when this is already the active mode.
    pub current: bool,
}

/// Map the canonical postures onto an agent's advertised modes.
///
/// Only postures with a matching mode are returned — offering "YOLO" to
/// an agent that has no such mode would be offering a lie. Modes nothing
/// matches are dropped rather than appended: the point is one fixed
/// vocabulary, not the agent's raw ids under new paint.
pub fn map_postures(
    available: &[(String, String, Option<String>)],
    current_id: Option<&str>,
) -> Vec<PostureMapping> {
    let mut used = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for posture in Posture::ALL {
        let hit = available.iter().find(|(id, name, _)| {
            !used.contains(id.as_str()) && posture.matches_mode(id, name)
        });
        let Some((id, name, description)) = hit else {
            continue;
        };
        used.insert(id.as_str());
        out.push(PostureMapping {
            key: posture,
            mode_id: id.clone(),
            mode_name: name.clone(),
            mode_description: description.clone(),
            current: current_id.is_some_and(|c| c == id),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modes(pairs: &[(&str, &str)]) -> Vec<(String, String, Option<String>)> {
        pairs
            .iter()
            .map(|(id, name)| (id.to_string(), name.to_string(), None))
            .collect()
    }

    #[test]
    fn harness_modes_and_postures_round_trip() {
        for mode in [Mode::Plan, Mode::Manual, Mode::Auto, Mode::Edit, Mode::Yolo] {
            let p = Posture::from_harness_mode(mode);
            assert_eq!(p.to_harness_mode(), mode);
        }
        assert_eq!(Posture::from_harness_mode(Mode::Manual), Posture::Ask);
        assert_eq!(Posture::from_harness_mode(Mode::Auto), Posture::Edits);
        assert_eq!(Posture::from_harness_mode(Mode::Edit), Posture::Auto);
    }

    #[test]
    fn agent_mode_names_map_onto_postures() {
        let available = modes(&[
            ("plan", "Plan"),
            ("default", "Default"),
            ("acceptEdits", "Accept edits"),
            ("dontask", "Don't ask"),
        ]);
        let mapped = map_postures(&available, Some("acceptEdits"));
        let keys: Vec<_> = mapped.iter().map(|m| m.key).collect();
        assert_eq!(keys, [Posture::Plan, Posture::Ask, Posture::Edits, Posture::Yolo]);
        assert!(mapped.iter().find(|m| m.key == Posture::Edits).unwrap().current);
    }

    #[test]
    fn unmapped_modes_are_dropped_not_painted_over() {
        // `bogus` matches nothing, so the result carries only Ask.
        let mapped = map_postures(&modes(&[("bogus", "Bogus"), ("default", "Default")]), None);
        assert_eq!(mapped.len(), 1);
        assert_eq!(mapped[0].key, Posture::Ask);
    }

    #[test]
    fn a_mode_cannot_match_two_postures() {
        // Both `plan` and `read-only` would match Plan::matches; the first
        // mode must be consumed by the first posture and not offered again.
        let available = modes(&[("read-only", "Read only"), ("plan", "Plan")]);
        let mapped = map_postures(&available, None);
        assert_eq!(mapped.len(), 1);
        assert_eq!(mapped[0].mode_id, "read-only");
    }

    #[test]
    fn codex_style_names_resolve() {
        let available = modes(&[
            ("read-only", "Read only"),
            ("agent", "Agent"),
            ("agent-full-access", "Full access"),
        ]);
        let mapped = map_postures(&available, None);
        let keys: Vec<_> = mapped.iter().map(|m| m.key).collect();
        assert_eq!(keys, [Posture::Plan, Posture::Ask, Posture::Yolo]);
    }
}
