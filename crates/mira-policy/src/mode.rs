use mira_tools::Action;
use serde::{Deserialize, Serialize};

use crate::Decision;

/// Permission preset.
///
/// Meaning:
///
/// - `Plan`: never edits, never runs commands. Reads and searches only.
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
            (Mode::Plan, Read | Pure) => Allow,
            (Mode::Plan, _) => Deny,

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
}
