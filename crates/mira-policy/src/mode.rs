use mira_tools::Action;
use serde::{Deserialize, Serialize};

use crate::Decision;

/// Permission preset.
///
/// Meaning:
///
/// - `Plan`: same gating as `Manual`, but the system prompt directs the
///   model to call the `plan` tool before touching anything. Behaves like
///   Manual once the plan is approved — the "planning" part is a prompt-
///   level convention, not a hard policy block.
/// - `Manual`: prompt before every write, edit, or command.
/// - `Auto`: writes and edits auto-approve; commands prompt (except pure-safe).
/// - `Edit`: writes, edits, AND commands auto-approve unless a rule blocks.
/// - `Yolo`: no gating whatsoever.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Plan,
    #[default]
    Manual,
    Auto,
    Edit,
    Yolo,
}

impl Mode {
    pub fn default_for(self, action: Action) -> Decision {
        self.default_for_target(action, "")
    }

    /// Mode default for a specific target. Only `Computer` and `Browser`
    /// look at the target: their observation verbs (screenshot,
    /// snapshot, …) are split from the verbs that change something
    /// (click, type, navigate, …).
    ///
    /// Desktop control is the one family that never auto-approves a
    /// mutating action, not even in `Yolo`: a stray click can land on
    /// anything on the user's screen. Opting in takes an explicit
    /// `Computer(...)` allow rule.
    pub fn default_for_target(self, action: Action, target: &str) -> Decision {
        use Action::*;
        use Decision::*;
        let observe = is_observation(target);
        match (self, action) {
            (Mode::Plan | Mode::Manual, Computer) => Ask,
            (_, Computer) if observe => Allow,
            (_, Computer) => Ask,

            (_, Browser) if observe => Allow,
            (Mode::Plan | Mode::Manual | Mode::Auto, Browser) => Ask,
            (Mode::Edit | Mode::Yolo, Browser) => Allow,

            // Plan mode used to be "reads only" — that blocked execution of
            // any user-approved plan. Now it defers to the manual defaults
            // and relies on the system prompt to steer the model into calling
            // `plan` first.
            (Mode::Plan, Read | Pure) => Allow,
            (Mode::Plan, _) => Ask,

            (Mode::Manual, Read | Pure) => Allow,
            (Mode::Manual, _) => Ask,

            (Mode::Auto, Read | Pure | Write | Edit) => Allow,
            (Mode::Auto, Bash) => Ask,

            (Mode::Edit, _) => Allow,
            (Mode::Yolo, _) => Allow,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Plan => "plan",
            Mode::Manual => "manual",
            Mode::Auto => "auto",
            Mode::Edit => "edit",
            Mode::Yolo => "yolo",
        }
    }

    /// Cycle to the next mode in a fixed order. Wraps around; used by
    /// the TUI's Shift+Tab shortcut to give users a keyboard-only path
    /// through the permission ladder without opening `/mode`.
    pub fn next(self) -> Mode {
        match self {
            Mode::Plan => Mode::Manual,
            Mode::Manual => Mode::Auto,
            Mode::Auto => Mode::Edit,
            Mode::Edit => Mode::Yolo,
            Mode::Yolo => Mode::Plan,
        }
    }

    /// Human-friendly one-liner used on the input-box chip. Matches
    /// Claude Code's `accept edits on` phrasing where it maps cleanly.
    pub fn chip_label(self) -> &'static str {
        match self {
            Mode::Plan => "plan mode",
            Mode::Manual => "ask on writes",
            Mode::Auto => "auto approve writes",
            Mode::Edit => "accept edits on",
            Mode::Yolo => "yolo · no gating",
        }
    }
}

/// Verbs of the `computer` / `browser` tools that only look at the
/// screen or page. The target's verb is everything before the first `:`.
const OBSERVATION_VERBS: &[&str] = &[
    "screenshot",
    "cursor_position",
    "zoom",
    "wait",
    "snapshot",
    "get_text",
    "list_tabs",
];

fn is_observation(target: &str) -> bool {
    let verb = target.split_once(':').map_or(target, |(v, _)| v);
    OBSERVATION_VERBS.contains(&verb)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computer_actions_prompt_even_in_yolo() {
        for mode in [Mode::Manual, Mode::Auto, Mode::Edit, Mode::Yolo] {
            assert_eq!(
                mode.default_for_target(Action::Computer, "left_click:10,10"),
                Decision::Ask,
                "{mode:?}"
            );
        }
        assert_eq!(
            Mode::Manual.default_for_target(Action::Computer, "screenshot"),
            Decision::Ask
        );
        assert_eq!(
            Mode::Auto.default_for_target(Action::Computer, "screenshot"),
            Decision::Allow
        );
    }

    #[test]
    fn browser_observation_is_free_but_actions_gate_below_edit() {
        assert_eq!(
            Mode::Manual.default_for_target(Action::Browser, "snapshot"),
            Decision::Allow
        );
        assert_eq!(
            Mode::Auto.default_for_target(Action::Browser, "navigate:https://x.dev"),
            Decision::Ask
        );
        assert_eq!(
            Mode::Edit.default_for_target(Action::Browser, "click:e3"),
            Decision::Allow
        );
    }
}
