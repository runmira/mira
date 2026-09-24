//! `computer` — see and drive the user's desktop.
//!
//! A thin wrapper over [`mira_computer::Computer`]: the arguments follow
//! Anthropic's computer-use action vocabulary, the policy target is
//! [`ComputerAction::policy_target`] (`left_click:512,300`,
//! `type:hello`, …), and screenshots come back as images on the
//! [`ToolResult`].

use std::sync::Arc;

use async_trait::async_trait;
use mira_ai::ToolSpec;
use mira_computer::{Computer, ComputerAction};
use mira_core::{ImageData, ToolCall, ToolResult};
use serde_json::{json, Value};

use crate::context::ToolContext;
use crate::tool::{spec, Action, Tool, ToolError};

pub struct ComputerTool {
    computer: Arc<Computer>,
    /// Model-space display size at registration, for the description.
    display: Option<(u32, u32)>,
}

impl ComputerTool {
    pub fn new(computer: Arc<Computer>, display: Option<(u32, u32)>) -> Self {
        Self { computer, display }
    }
}

fn parse(call: &ToolCall) -> Result<ComputerAction, String> {
    let args: Value = serde_json::from_str(&call.function.arguments).map_err(|e| e.to_string())?;
    ComputerAction::from_args(&args).map_err(|e| e.to_string())
}

#[async_trait]
impl Tool for ComputerTool {
    fn spec(&self) -> ToolSpec {
        let display = match self.display {
            Some((w, h)) => format!("The screen is {w}x{h} in screenshot pixels. "),
            None => String::new(),
        };
        spec(
            "computer",
            &format!(
                "Control the user's desktop: take screenshots, move and click the mouse, type, \
                 press keys, scroll. {display}Coordinates are [x, y] pixels in the most recent \
                 screenshot. Start with `screenshot` to see the screen; after each action you \
                 get a fresh screenshot back. Prefer keyboard shortcuts when they are reliable. \
                 Every action is visible to the user and most need their approval — explain \
                 what you are about to do. Never enter passwords or payment details, and stop \
                 to ask the user before anything destructive or irreversible."
            ),
            json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": [
                            "screenshot", "cursor_position", "mouse_move",
                            "left_click", "right_click", "middle_click", "double_click",
                            "triple_click", "left_click_drag", "left_mouse_down", "left_mouse_up",
                            "type", "key", "hold_key", "scroll", "wait", "zoom"
                        ],
                        "description": "What to do."
                    },
                    "coordinate": {
                        "type": "array", "items": {"type": "integer"}, "minItems": 2, "maxItems": 2,
                        "description": "[x, y] for mouse_move, clicks, left_click_drag (end point) and scroll."
                    },
                    "start_coordinate": {
                        "type": "array", "items": {"type": "integer"}, "minItems": 2, "maxItems": 2,
                        "description": "[x, y] where left_click_drag starts."
                    },
                    "text": {
                        "type": "string",
                        "description": "Text for `type`; a key chord for `key`/`hold_key` (xdotool style: \"Return\", \"ctrl+s\", \"cmd+shift+t\"); modifier keys to hold during a click or scroll (\"shift\")."
                    },
                    "scroll_direction": {"type": "string", "enum": ["up", "down", "left", "right"]},
                    "scroll_amount": {"type": "integer", "minimum": 1, "description": "Wheel clicks. Default 3."},
                    "duration": {"type": "number", "description": "Seconds, for `wait` and `hold_key` (max 30)."},
                    "region": {
                        "type": "array", "items": {"type": "integer"}, "minItems": 4, "maxItems": 4,
                        "description": "[x0, y0, x1, y1] to view at full resolution with `zoom`."
                    }
                },
                "required": ["action"]
            }),
        )
    }

    fn action(&self) -> Action {
        Action::Computer
    }

    fn policy_target(&self, call: &ToolCall) -> String {
        match parse(call) {
            Ok(a) => a.policy_target(),
            // Unparseable calls fail in `invoke` anyway; gate them as
            // the most restrictive thing they could be.
            Err(_) => "invalid".to_owned(),
        }
    }

    async fn invoke(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let action = parse(call).map_err(ToolError::InvalidArgs)?;
        match self.computer.execute(&action).await {
            Ok(out) => {
                let mut result = ToolResult::ok(call.id.clone(), out.text);
                if let Some(shot) = out.screenshot {
                    result.data = Some(json!({
                        "screenshot": {"width": shot.width, "height": shot.height}
                    }));
                    result = result.with_images(vec![ImageData::png(shot.png_base64)]);
                }
                Ok(result)
            }
            Err(e) => Ok(ToolResult::err(call.id.clone(), e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mira_computer::backend::mock::MockBackend;
    use mira_computer::ComputerOptions;
    use mira_core::message::{ToolCallFunction, ToolCallKind};

    fn call(args: Value) -> ToolCall {
        ToolCall {
            id: "c1".into(),
            kind: ToolCallKind::Function,
            function: ToolCallFunction {
                name: "computer".into(),
                arguments: args.to_string(),
            },
        }
    }

    fn tool() -> ComputerTool {
        let backend = Arc::new(MockBackend::new((800, 600), (800, 600)));
        let computer = Computer::new(
            backend,
            ComputerOptions {
                settle: std::time::Duration::ZERO,
                ..Default::default()
            },
        );
        ComputerTool::new(Arc::new(computer), Some((800, 600)))
    }

    #[tokio::test]
    async fn screenshot_returns_an_image() {
        let t = tool();
        let ctx = ToolContext::new(
            std::env::temp_dir(),
            Arc::new(mira_sandbox::Sandbox::default_scrubbed()),
        );
        let r = t
            .invoke(&call(json!({"action": "screenshot"})), &ctx)
            .await
            .unwrap();
        assert!(!r.is_error);
        assert_eq!(r.images.len(), 1);
        assert_eq!(r.images[0].media_type, "image/png");
    }

    #[test]
    fn policy_targets() {
        let t = tool();
        assert_eq!(
            t.policy_target(&call(json!({"action": "type", "text": "hi"}))),
            "type:hi"
        );
        assert_eq!(t.policy_target(&call(json!({"action": "nope"}))), "invalid");
        assert!(!t.parallel_safe(&call(json!({"action": "screenshot"}))));
    }
}
