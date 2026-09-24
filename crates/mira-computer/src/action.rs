//! The action vocabulary the model speaks.
//!
//! Mirrors Anthropic's computer-use tool (`action` discriminator,
//! `coordinate: [x, y]`, `scroll_direction`, …) so a Claude model's
//! trained habits carry over when it calls Mira's function-style
//! `computer` tool, and a future native passthrough can reuse the parser.

use serde::Deserialize;
use serde_json::Value;

use crate::ComputerError;

/// A point in the *model's* coordinate space — pixels of the most recent
/// screenshot, not physical screen pixels. [`crate::Computer`] rescales.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ScrollDirection {
    Up,
    Down,
    Left,
    Right,
}

/// One parsed `computer` tool call.
#[derive(Clone, Debug, PartialEq)]
pub enum ComputerAction {
    Screenshot,
    CursorPosition,
    MouseMove(Point),
    /// Press-and-release `count` times (1 = click, 2 = double, 3 = triple),
    /// optionally moving first and holding modifier keys (`text`).
    Click {
        button: MouseButton,
        at: Option<Point>,
        count: u8,
        modifiers: Option<String>,
    },
    Drag {
        from: Point,
        to: Point,
    },
    MouseDown,
    MouseUp,
    Type(String),
    Key(String),
    HoldKey {
        keys: String,
        seconds: f64,
    },
    Scroll {
        at: Option<Point>,
        direction: ScrollDirection,
        amount: u32,
        modifiers: Option<String>,
    },
    Wait {
        seconds: f64,
    },
    /// Return a full-resolution crop of `[x0, y0, x1, y1]` (model space).
    Zoom {
        region: [i32; 4],
    },
}

#[derive(Deserialize)]
struct RawArgs {
    action: String,
    #[serde(default)]
    coordinate: Option<Vec<f64>>,
    #[serde(default)]
    start_coordinate: Option<Vec<f64>>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    scroll_direction: Option<String>,
    #[serde(default)]
    scroll_amount: Option<u32>,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    region: Option<Vec<f64>>,
}

/// Longest `wait` / `hold_key` we honor. Anything longer is almost
/// certainly a unit mix-up (ms vs s) and would stall the turn.
const MAX_WAIT_SECS: f64 = 30.0;

impl ComputerAction {
    /// Parse the tool-call arguments.
    pub fn from_args(args: &Value) -> Result<Self, ComputerError> {
        let raw: RawArgs = serde_json::from_value(args.clone())
            .map_err(|e| ComputerError::InvalidArgs(e.to_string()))?;
        let coord = |v: &Option<Vec<f64>>, field: &str| -> Result<Option<Point>, ComputerError> {
            match v {
                None => Ok(None),
                Some(xy) if xy.len() == 2 => Ok(Some(Point {
                    x: xy[0].round() as i32,
                    y: xy[1].round() as i32,
                })),
                Some(_) => Err(ComputerError::InvalidArgs(format!(
                    "`{field}` must be [x, y]"
                ))),
            }
        };
        let need = |p: Option<Point>, field: &str| {
            p.ok_or_else(|| ComputerError::InvalidArgs(format!("`{field}` is required")))
        };
        let need_text = |field: &str| {
            raw.text
                .clone()
                .filter(|t| !t.is_empty())
                .ok_or_else(|| ComputerError::InvalidArgs(format!("`{field}` is required")))
        };
        let at = coord(&raw.coordinate, "coordinate")?;
        let click = |button, count| ComputerAction::Click {
            button,
            at,
            count,
            modifiers: raw.text.clone().filter(|t| !t.is_empty()),
        };
        let seconds = raw.duration.unwrap_or(1.0).clamp(0.0, MAX_WAIT_SECS);

        Ok(match raw.action.as_str() {
            "screenshot" => Self::Screenshot,
            "cursor_position" => Self::CursorPosition,
            "mouse_move" => Self::MouseMove(need(at, "coordinate")?),
            "left_click" => click(MouseButton::Left, 1),
            "right_click" => click(MouseButton::Right, 1),
            "middle_click" => click(MouseButton::Middle, 1),
            "double_click" => click(MouseButton::Left, 2),
            "triple_click" => click(MouseButton::Left, 3),
            "left_click_drag" => Self::Drag {
                from: need(
                    coord(&raw.start_coordinate, "start_coordinate")?,
                    "start_coordinate",
                )?,
                to: need(at, "coordinate")?,
            },
            "left_mouse_down" => Self::MouseDown,
            "left_mouse_up" => Self::MouseUp,
            "type" => Self::Type(need_text("text")?),
            "key" => Self::Key(need_text("text")?),
            "hold_key" => Self::HoldKey {
                keys: need_text("text")?,
                seconds,
            },
            "scroll" => Self::Scroll {
                at,
                direction: match raw.scroll_direction.as_deref() {
                    Some("up") => ScrollDirection::Up,
                    Some("down") | None => ScrollDirection::Down,
                    Some("left") => ScrollDirection::Left,
                    Some("right") => ScrollDirection::Right,
                    Some(other) => {
                        return Err(ComputerError::InvalidArgs(format!(
                            "unknown scroll_direction `{other}`"
                        )))
                    }
                },
                amount: raw.scroll_amount.unwrap_or(3).clamp(1, 50),
                modifiers: raw.text.clone().filter(|t| !t.is_empty()),
            },
            "wait" => Self::Wait { seconds },
            "zoom" => {
                let r = raw.region.filter(|r| r.len() == 4).ok_or_else(|| {
                    ComputerError::InvalidArgs("`region` must be [x0, y0, x1, y1]".into())
                })?;
                Self::Zoom {
                    region: [
                        r[0].round() as i32,
                        r[1].round() as i32,
                        r[2].round() as i32,
                        r[3].round() as i32,
                    ],
                }
            }
            other => {
                return Err(ComputerError::InvalidArgs(format!(
                    "unknown action `{other}`"
                )))
            }
        })
    }

    /// The wire name of this action (`left_click`, `type`, …).
    pub fn verb(&self) -> &'static str {
        match self {
            Self::Screenshot => "screenshot",
            Self::CursorPosition => "cursor_position",
            Self::MouseMove(_) => "mouse_move",
            Self::Click { button, count, .. } => match (button, count) {
                (MouseButton::Right, _) => "right_click",
                (MouseButton::Middle, _) => "middle_click",
                (MouseButton::Left, 2) => "double_click",
                (MouseButton::Left, 3) => "triple_click",
                (MouseButton::Left, _) => "left_click",
            },
            Self::Drag { .. } => "left_click_drag",
            Self::MouseDown => "left_mouse_down",
            Self::MouseUp => "left_mouse_up",
            Self::Type(_) => "type",
            Self::Key(_) => "key",
            Self::HoldKey { .. } => "hold_key",
            Self::Scroll { .. } => "scroll",
            Self::Wait { .. } => "wait",
            Self::Zoom { .. } => "zoom",
        }
    }

    /// The string the policy engine gates on: `verb` or `verb:detail`.
    /// Details are the coordinates, text, or key chord — what a rule like
    /// `Computer(type:*password*)` or `Computer(key:ctrl+*)` matches.
    pub fn policy_target(&self) -> String {
        let verb = self.verb();
        let pt = |p: &Point| format!("{},{}", p.x, p.y);
        let detail = match self {
            Self::MouseMove(p) => pt(p),
            Self::Click { at: Some(p), .. } => pt(p),
            Self::Drag { from, to } => format!("{}->{}", pt(from), pt(to)),
            Self::Type(t) => t.clone(),
            Self::Key(k) | Self::HoldKey { keys: k, .. } => k.clone(),
            Self::Scroll { direction, .. } => format!("{direction:?}").to_lowercase(),
            _ => String::new(),
        };
        if detail.is_empty() {
            verb.to_owned()
        } else {
            format!("{verb}:{detail}")
        }
    }

    /// Whether the action only observes the screen (no input events).
    pub fn is_observation(&self) -> bool {
        matches!(
            self,
            Self::Screenshot | Self::CursorPosition | Self::Wait { .. } | Self::Zoom { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_anthropic_shaped_calls() {
        let a = ComputerAction::from_args(&json!({"action": "left_click", "coordinate": [10, 20]}))
            .unwrap();
        assert_eq!(
            a,
            ComputerAction::Click {
                button: MouseButton::Left,
                at: Some(Point { x: 10, y: 20 }),
                count: 1,
                modifiers: None,
            }
        );
        assert_eq!(a.policy_target(), "left_click:10,20");

        let d = ComputerAction::from_args(&json!({
            "action": "left_click_drag", "start_coordinate": [1, 2], "coordinate": [3, 4]
        }))
        .unwrap();
        assert_eq!(d.policy_target(), "left_click_drag:1,2->3,4");

        let k = ComputerAction::from_args(&json!({"action": "key", "text": "ctrl+s"})).unwrap();
        assert_eq!(k.policy_target(), "key:ctrl+s");

        let s = ComputerAction::from_args(&json!({
            "action": "scroll", "coordinate": [5, 5], "scroll_direction": "up", "scroll_amount": 2
        }))
        .unwrap();
        assert_eq!(s.policy_target(), "scroll:up");
        assert_eq!(
            ComputerAction::from_args(&json!({"action": "screenshot"}))
                .unwrap()
                .policy_target(),
            "screenshot"
        );
    }

    #[test]
    fn rejects_bad_shapes() {
        for bad in [
            json!({"action": "mouse_move"}),
            json!({"action": "type"}),
            json!({"action": "left_click", "coordinate": [1]}),
            json!({"action": "teleport"}),
            json!({"action": "scroll", "scroll_direction": "sideways"}),
        ] {
            assert!(ComputerAction::from_args(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn wait_is_clamped() {
        let w = ComputerAction::from_args(&json!({"action": "wait", "duration": 5000})).unwrap();
        assert_eq!(
            w,
            ComputerAction::Wait {
                seconds: MAX_WAIT_SECS
            }
        );
    }
}
