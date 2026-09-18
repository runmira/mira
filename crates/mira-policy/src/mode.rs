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
        use Action::*;
        use Decision::*;
        match (self, action) {
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
