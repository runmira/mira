use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Wire-shape description of a tool, sent to the model on every request.
///
/// Kept in `mira-ai` (not `mira-tools`) because the shape is a provider
/// concern: it must serialize exactly the way the OpenAI schema expects.
/// Tool implementations produce a `ToolSpec` for the harness to forward.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// JSON Schema object describing the tool's argument shape.
    pub parameters: Value,
}
