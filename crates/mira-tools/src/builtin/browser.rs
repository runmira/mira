//! `browser` — drive a real Chromium-family browser in Mira's own profile.
//!
//! Wraps [`mira_browser::Browser`]. The model mostly works from text
//! snapshots (element refs like `e12`); screenshots are available for
//! visual pages. The policy target is
//! [`BrowserAction::policy_target`] (`navigate:https://…`, `click:e12`, …).

use std::sync::Arc;

use async_trait::async_trait;
use mira_ai::ToolSpec;
use mira_browser::{Browser, BrowserAction};
use mira_core::{ImageData, ToolCall, ToolResult};
use serde_json::{json, Value};

use crate::context::ToolContext;
use crate::tool::{spec, Action, Tool, ToolError};

pub struct BrowserTool {
    browser: Arc<Browser>,
}

impl BrowserTool {
    pub fn new(browser: Arc<Browser>) -> Self {
        Self { browser }
    }
}

fn parse(call: &ToolCall) -> Result<BrowserAction, String> {
    let args: Value = serde_json::from_str(&call.function.arguments).map_err(|e| e.to_string())?;
    BrowserAction::from_args(&args).map_err(|e| e.to_string())
}

#[async_trait]
impl Tool for BrowserTool {
    fn spec(&self) -> ToolSpec {
        spec(
            "browser",
            "Drive a real web browser (a separate Mira profile, not the user's own browser). \
             Use it for pages that need JavaScript, logins, clicking or forms; for reading a \
             static page, `web_fetch` is cheaper. Start with `navigate`, which returns a \
             snapshot listing interactive elements as `[e12] button \"Sign in\"` plus the page \
             text. Target elements by `ref` (e.g. \"e12\") from the latest snapshot — refs can \
             change after the page updates. Actions that change the page return a fresh \
             snapshot. Use `screenshot` only when layout or visuals matter; `click` with \
             `coordinate` uses that screenshot's pixels. Never enter passwords or payment \
             details yourself — ask the user to do it in the browser window.",
            json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": [
                            "navigate", "back", "forward", "reload", "snapshot", "screenshot",
                            "click", "type", "key", "scroll", "evaluate",
                            "list_tabs", "new_tab", "switch_tab", "close_tab", "wait", "close"
                        ]
                    },
                    "url": {"type": "string", "description": "For navigate / new_tab."},
                    "ref": {"type": "string", "description": "Element ref from the latest snapshot, e.g. \"e12\"."},
                    "selector": {"type": "string", "description": "CSS selector, when no ref fits."},
                    "coordinate": {
                        "type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 2,
                        "description": "[x, y] in the latest screenshot, for click."
                    },
                    "double": {"type": "boolean", "description": "Double-click."},
                    "text": {"type": "string", "description": "Text to type."},
                    "clear": {"type": "boolean", "description": "Clear the field before typing."},
                    "submit": {"type": "boolean", "description": "Press Enter after typing."},
                    "key": {"type": "string", "description": "Key or chord: \"Enter\", \"Escape\", \"ctrl+a\"."},
                    "direction": {"type": "string", "enum": ["up", "down", "left", "right"]},
                    "amount": {"type": "integer", "minimum": 1, "description": "Scroll steps. Default 3."},
                    "expression": {"type": "string", "description": "JavaScript to evaluate in the page; the result is returned as JSON."},
                    "index": {"type": "integer", "minimum": 0, "description": "Tab index from list_tabs."},
                    "duration": {"type": "number", "description": "Seconds to wait (max 30)."}
                },
                "required": ["action"]
            }),
        )
    }

    fn action(&self) -> Action {
        Action::Browser
    }

    fn policy_target(&self, call: &ToolCall) -> String {
        match parse(call) {
            Ok(a) => a.policy_target(),
            Err(_) => "invalid".to_owned(),
        }
    }

    async fn invoke(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let action = parse(call).map_err(ToolError::InvalidArgs)?;
        match self.browser.execute(&action).await {
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
