//! `Skill` — invoke a named instruction bundle.
//!
//! Skills are markdown files that tell the current agent *how* to do a
//! specific task in this project (verify a change, write a commit,
//! review a diff, run a security audit, …). Unlike the `agent` tool,
//! which spawns a cold-context child conversation, a skill invocation
//! stays in the same conversation and only changes what the model reads
//! before its next reply.
//!
//! The tool's `spec()` enumerates every loaded skill's name in the
//! `name` parameter's `enum` and lists each name + description in the
//! tool description — same trick the agent tool uses so the model sees
//! the roster in the schema, not just as freeform prose.
//!
//! Delivery is belt-and-suspenders: the tool result contains the skill
//! body wrapped in a `<system-reminder>` block so it lands both as a
//! visible tool result AND as an authoritative instruction the model
//! treats seriously on the next round.

use std::sync::Arc;

use async_trait::async_trait;
use mira_ai::ToolSpec;
use mira_core::{ToolCall, ToolResult};
use mira_skills::SkillRegistry;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::context::ToolContext;
use crate::tool::{spec, Action, Tool, ToolError};

/// Handle to the (possibly-swappable) skill registry the tool consults.
/// `RwLock` because `mira serve` swaps the registry on cwd change so
/// project-scoped skills track the current folder.
pub type SkillHandle = Arc<RwLock<Arc<SkillRegistry>>>;

pub struct SkillTool {
    skills: SkillHandle,
}

impl SkillTool {
    pub fn new(skills: SkillHandle) -> Self {
        Self { skills }
    }

    /// Convenience for headless callers (CLI, tests) that don't need
    /// hot-swap semantics — just wraps the registry in the same
    /// `RwLock<Arc<_>>` shape.
    pub fn from_registry(reg: SkillRegistry) -> Self {
        Self::new(Arc::new(RwLock::new(Arc::new(reg))))
    }
}

#[derive(Deserialize)]
struct Args {
    /// Machine name of the skill to invoke.
    name: String,
    /// Optional free-text argument the caller passed to `/skill <name>
    /// <args>` or `Skill { name, args }`. Threaded into the reminder so
    /// the skill body sees the user's intent inline.
    #[serde(default)]
    args: Option<String>,
}

#[async_trait]
impl Tool for SkillTool {
    fn spec(&self) -> ToolSpec {
        // Snapshot the registry once per spec build. `spec()` is
        // called at request construction time; the registry can change
        // between requests but the snapshot is consistent within one.
        let reg = self.skills.blocking_read_snapshot();

        let names = reg.names();
        let (description, enum_values) = if names.is_empty() {
            (
                "Invoke a named skill (instruction bundle). No skills are \
                 currently registered — this tool is a no-op until you add \
                 a skill file to `~/.mira/skills/` or `<cwd>/.mira/skills/`."
                    .to_owned(),
                Vec::new(),
            )
        } else {
            let roster = reg
                .skills
                .values()
                .map(|s| format!("• `{}` — {}", s.name, s.description))
                .collect::<Vec<_>>()
                .join("\n");
            (
                format!(
                    "Invoke a named skill — a procedural instruction bundle for \
                     accomplishing a specific task in this project. When the \
                     user's request matches a skill's description, prefer \
                     calling this over improvising: the skill has been tuned \
                     for this codebase and will produce a more predictable \
                     result. The skill body is returned as this tool's result \
                     and MUST be followed exactly on the next turn.\n\n\
                     Available skills:\n{roster}",
                ),
                names.iter().map(|n| Value::String(n.clone())).collect(),
            )
        };

        let mut properties = json!({
            "name": {
                "type": "string",
                "description": "Machine name of the skill to invoke. Must be one of the values listed in `enum`.",
            },
            "args": {
                "type": "string",
                "description": "Optional free-text argument threaded into the skill's activation reminder. Use to pass a target file, a PR number, an approval token — anything the skill's body says it accepts.",
            }
        });
        if !enum_values.is_empty() {
            properties["name"]["enum"] = Value::Array(enum_values);
        }

        spec(
            "skill",
            &description,
            json!({
                "type": "object",
                "properties": properties,
                "required": ["name"],
                "additionalProperties": false
            }),
        )
    }

    fn action(&self) -> Action {
        // Skills don't touch the filesystem or shell — they just
        // return instructions. What the model does *after* reading
        // them is gated by whatever tools it calls next.
        Action::Pure
    }

    fn parallel_safe(&self, _call: &ToolCall) -> bool {
        true
    }

    async fn invoke(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let args: Args = call.parse_arguments()?;
        let reg = self.skills.read().await.clone();
        let Some(skill) = reg.get(&args.name) else {
            let available = reg.names().join(", ");
            let hint = if available.is_empty() {
                "no skills registered".to_owned()
            } else {
                format!("available: {available}")
            };
            return Ok(ToolResult::err(
                call.id.clone(),
                format!("unknown skill `{}` ({hint})", args.name),
            ));
        };

        let args_line = args
            .args
            .as_deref()
            .map(|a| a.trim())
            .filter(|a| !a.is_empty())
            .map(|a| format!("\n\nInvocation arg: {a}"))
            .unwrap_or_default();

        // Attached-files roster (directory-shape skills only). The
        // model may need to `read_file` these if the body says
        // "run `check.sh`" or "use the template at `template.md`".
        // Bundled builtins have no source_dir → empty list → no
        // attachments block.
        let attachments_block = match skill.source_dir.as_ref() {
            Some(dir) => {
                let files = skill.attached_files();
                if files.is_empty() {
                    String::new()
                } else {
                    let listing = files
                        .iter()
                        .map(|p| format!("- {}", dir.join(p).display()))
                        .collect::<Vec<_>>()
                        .join("\n");
                    format!(
                        "\n\nAttached resources (siblings of SKILL.md — \
                         `read_file` any the body references):\n{listing}"
                    )
                }
            }
            None => String::new(),
        };

        // Belt-and-suspenders delivery: the tool result content is the
        // skill body wrapped in a `<system-reminder>` block so the
        // model reads it as an authoritative instruction on the next
        // turn — not as generic tool output. The tool result is still
        // visible in the transcript for the user's audit trail.
        let body = format!(
            "<system-reminder>\n\
             You invoked the `{name}` skill. Follow these instructions \
             exactly on your next turn; they override any conflicting \
             defaults from your system prompt.{args_line}{attachments_block}\n\n\
             --- skill body ---\n\n{skill_body}\n\
             </system-reminder>",
            name = skill.name,
            skill_body = skill.body.trim(),
        );

        Ok(ToolResult::ok(call.id.clone(), body))
    }
}

/* ---------- RwLock read-snapshot helper ---------- */

/// `tokio::sync::RwLock` doesn't offer a blocking read, and `Tool::spec`
/// is sync. We use the async `try_read` (fast path) with a fallback to
/// `blocking_read` — safe because writes are rare (only on cwd swap)
/// and the fallback path yields the tokio runtime through the block-
/// in-place shim used elsewhere in the workspace.
trait BlockingReadSnapshot<T> {
    fn blocking_read_snapshot(&self) -> T;
}

impl BlockingReadSnapshot<Arc<SkillRegistry>> for SkillHandle {
    fn blocking_read_snapshot(&self) -> Arc<SkillRegistry> {
        // Fast path — try a non-blocking read; almost always succeeds.
        if let Ok(g) = self.try_read() {
            return g.clone();
        }
        // Slow path — a writer is holding the lock right now. Spin
        // briefly rather than block, because `spec()` is called from
        // sync code paths that shouldn't wait on I/O.
        for _ in 0..64 {
            if let Ok(g) = self.try_read() {
                return g.clone();
            }
            std::thread::yield_now();
        }
        // Last-ditch: hand back an empty registry rather than panicking.
        // The next round will retry.
        Arc::new(SkillRegistry::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mira_core::message::{ToolCallFunction, ToolCallKind};
    use mira_core::{ToolCall, ToolCallId};
    use mira_sandbox::Sandbox;
    use mira_skills::Skill;
    use std::path::PathBuf;

    fn call(name: &str, args: serde_json::Value) -> ToolCall {
        ToolCall {
            id: ToolCallId::from("test-1"),
            kind: ToolCallKind::Function,
            function: ToolCallFunction {
                name: name.into(),
                arguments: args.to_string(),
            },
        }
    }

    fn ctx() -> ToolContext {
        ToolContext::new(PathBuf::from("/tmp"), Arc::new(Sandbox::default_scrubbed()))
    }

    fn make_tool(skills: Vec<Skill>) -> SkillTool {
        let mut reg = SkillRegistry::new();
        for s in skills {
            reg.skills.insert(s.name.clone(), s);
        }
        SkillTool::from_registry(reg)
    }

    fn s(name: &str, description: &str, body: &str) -> Skill {
        Skill {
            name: name.into(),
            description: description.into(),
            category: None,
            icon: None,
            color: None,
            body: body.into(),
            source: None,
            source_dir: None,
        }
    }

    #[tokio::test]
    async fn known_skill_returns_wrapped_body() {
        let tool = make_tool(vec![s("verify", "run the app", "Step one.")]);
        let r = tool
            .invoke(&call("skill", json!({"name": "verify"})), &ctx())
            .await
            .unwrap();
        assert!(r.content.contains("<system-reminder>"));
        assert!(r.content.contains("You invoked the `verify` skill"));
        assert!(r.content.contains("Step one."));
        assert!(r.content.contains("</system-reminder>"));
    }

    #[tokio::test]
    async fn args_line_threads_through() {
        let tool = make_tool(vec![s("verify", "run", "Body.")]);
        let r = tool
            .invoke(
                &call("skill", json!({"name": "verify", "args": "PR 42"})),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(r.content.contains("Invocation arg: PR 42"));
    }

    #[tokio::test]
    async fn unknown_skill_returns_error_result() {
        let tool = make_tool(vec![s("verify", "run", "Body.")]);
        let r = tool
            .invoke(&call("skill", json!({"name": "nope"})), &ctx())
            .await
            .unwrap();
        assert!(r.is_error);
        assert!(r.content.contains("unknown skill"));
        assert!(r.content.contains("available: verify"));
    }

    #[test]
    fn spec_lists_roster_in_description() {
        let tool = make_tool(vec![
            s("verify", "verify a change", "Body A."),
            s("commit", "compose a commit", "Body B."),
        ]);
        let spec = tool.spec();
        assert!(spec.description.contains("verify"));
        assert!(spec.description.contains("commit"));
        // Enum on `name` lists both.
        let params = &spec.parameters;
        let name_prop = &params["properties"]["name"];
        let enum_values = name_prop["enum"].as_array().unwrap();
        let names: Vec<&str> = enum_values.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(names.contains(&"verify"));
        assert!(names.contains(&"commit"));
    }

    #[test]
    fn spec_when_empty_registry() {
        let tool = SkillTool::from_registry(SkillRegistry::new());
        let spec = tool.spec();
        assert!(spec.description.contains("No skills"));
        // No enum when empty — model shouldn't be told to pick from nothing.
        assert!(spec.parameters["properties"]["name"].get("enum").is_none());
    }
}
