//! Task tracking tools — `task_create`, `task_list`, `task_get`,
//! `task_update`.
//!
//! Shape mirrors Claude Code's Task suite (see
//! https://code.claude.com/docs/en/agent-sdk/todo-tracking) so a model
//! that already knows how to drive TaskCreate/Update elsewhere works
//! here without re-learning. The store itself lives in
//! `mira_tools::tasks` — see there for lifecycle + persistence notes.
//!
//! Snake-case tool names match Mira's existing convention
//! (`read_file`, `find_symbol`, …); the model doesn't care about the
//! casing.

use async_trait::async_trait;
use mira_ai::ToolSpec;
use mira_core::{ToolCall, ToolResult};
use serde::Deserialize;
use serde_json::json;

use crate::context::ToolContext;
use crate::tasks::{TaskStatus, TaskUpdate};
use crate::tool::{spec, Action, Tool, ToolError};

// ---------- task_create ----------

pub struct TaskCreate;

#[derive(Deserialize)]
struct CreateArgs {
    subject: String,
    #[serde(default)]
    description: String,
    #[serde(default, alias = "activeForm")]
    active_form: Option<String>,
}

#[async_trait]
impl Tool for TaskCreate {
    fn spec(&self) -> ToolSpec {
        spec(
            "task_create",
            "Add a new task to the session's todo list. Use for \
             multi-step work so you can hand-off / resume without \
             losing the plan. Returns the assigned integer id you'll \
             pass to `task_update` and `task_get`.",
            json!({
                "type": "object",
                "properties": {
                    "subject": {
                        "type": "string",
                        "description": "Imperative short title (e.g. 'Run tests')."
                    },
                    "description": {
                        "type": "string",
                        "description": "Optional detail — a sentence or two.",
                        "default": ""
                    },
                    "active_form": {
                        "type": "string",
                        "description": "Present-continuous form shown while the task is in progress (e.g. 'Running tests')."
                    }
                },
                "required": ["subject"],
                "additionalProperties": false
            }),
        )
    }

    fn action(&self) -> Action {
        Action::Pure
    }

    async fn invoke(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let store = ctx
            .tasks
            .as_ref()
            .ok_or_else(|| ToolError::Failed("task store not wired for this session".into()))?;
        let args: CreateArgs = call.parse_arguments()?;
        let subject = args.subject.trim();
        if subject.is_empty() {
            return Err(ToolError::InvalidArgs("subject is empty".into()));
        }
        let item = store
            .create(subject.to_owned(), args.description, args.active_form)
            .await;
        // Content: human-readable line so a raw transcript reads okay
        // even without the structured `data` payload.
        // Data: `{ task: FullItem }` — a superset of Claude Code's
        // documented `{ task: { id, subject } }` so a client that just
        // wants id+subject still works, and a client that wants to
        // hydrate a live UI (Mira's TaskListPanel) has everything.
        let content = format!("created task #{}: {}", item.id, item.subject);
        let data = json!({ "task": item });
        Ok(ToolResult {
            call_id: call.id.clone(),
            content,
            is_error: false,
            data: Some(data),
        })
    }
}

// ---------- task_list ----------

pub struct TaskList;

#[async_trait]
impl Tool for TaskList {
    fn spec(&self) -> ToolSpec {
        spec(
            "task_list",
            "Return the current session's task list — all non-deleted \
             tasks in id order. Cheap: call whenever you want to \
             re-check your plan.",
            json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        )
    }

    fn action(&self) -> Action {
        Action::Read
    }

    async fn invoke(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let store = ctx
            .tasks
            .as_ref()
            .ok_or_else(|| ToolError::Failed("task store not wired for this session".into()))?;
        let items = store.list().await;
        let content = render_list(&items);
        let data = json!({ "tasks": items });
        Ok(ToolResult {
            call_id: call.id.clone(),
            content,
            is_error: false,
            data: Some(data),
        })
    }
}

// ---------- task_get ----------

pub struct TaskGet;

#[derive(Deserialize)]
struct GetArgs {
    #[serde(alias = "id", alias = "taskId")]
    task_id: u32,
}

#[async_trait]
impl Tool for TaskGet {
    fn spec(&self) -> ToolSpec {
        spec(
            "task_get",
            "Return one task's full details (subject, description, \
             active_form, status, timestamps).",
            json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "integer", "minimum": 1 }
                },
                "required": ["task_id"],
                "additionalProperties": false
            }),
        )
    }

    fn action(&self) -> Action {
        Action::Read
    }

    async fn invoke(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let store = ctx
            .tasks
            .as_ref()
            .ok_or_else(|| ToolError::Failed("task store not wired for this session".into()))?;
        let args: GetArgs = call.parse_arguments()?;
        match store.get(args.task_id).await {
            Some(t) => {
                let content = format!(
                    "task #{} [{}]: {}",
                    t.id,
                    t.status.as_str(),
                    t.subject
                );
                let data = json!({ "task": t });
                Ok(ToolResult {
                    call_id: call.id.clone(),
                    content,
                    is_error: false,
                    data: Some(data),
                })
            }
            None => Err(ToolError::Failed(format!(
                "no task with id {}",
                args.task_id
            ))),
        }
    }
}

// ---------- task_update ----------

pub struct TaskUpdateTool;

#[derive(Deserialize)]
struct UpdateArgs {
    #[serde(alias = "id", alias = "taskId")]
    task_id: u32,
    #[serde(default)]
    subject: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default, alias = "activeForm")]
    active_form: Option<String>,
    #[serde(default)]
    status: Option<String>,
}

#[async_trait]
impl Tool for TaskUpdateTool {
    fn spec(&self) -> ToolSpec {
        spec(
            "task_update",
            "Update a task's status or fields. Set `status` to \
             `in_progress` when you start work, `completed` when done, \
             or `deleted` to drop a task you no longer need. Any \
             other field left `null` is unchanged.",
            json!({
                "type": "object",
                "properties": {
                    "task_id":     { "type": "integer", "minimum": 1 },
                    "subject":     { "type": "string" },
                    "description": { "type": "string" },
                    "active_form": { "type": "string" },
                    "status":      {
                        "type": "string",
                        "enum": ["pending", "in_progress", "completed", "deleted"]
                    }
                },
                "required": ["task_id"],
                "additionalProperties": false
            }),
        )
    }

    fn action(&self) -> Action {
        Action::Pure
    }

    async fn invoke(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let store = ctx
            .tasks
            .as_ref()
            .ok_or_else(|| ToolError::Failed("task store not wired for this session".into()))?;
        let args: UpdateArgs = call.parse_arguments()?;
        let status = match args.status.as_deref() {
            None => None,
            Some("pending") => Some(TaskStatus::Pending),
            Some("in_progress") => Some(TaskStatus::InProgress),
            Some("completed") => Some(TaskStatus::Completed),
            Some("deleted") => Some(TaskStatus::Deleted),
            Some(other) => {
                return Err(ToolError::InvalidArgs(format!(
                    "unknown status `{other}` — must be pending / in_progress / completed / deleted"
                )));
            }
        };
        let patch = TaskUpdate {
            subject: args.subject,
            description: args.description,
            active_form: args.active_form,
            status,
        };
        match store.update(args.task_id, patch).await {
            Some(t) => {
                let content = format!(
                    "task #{} → {}: {}",
                    t.id,
                    t.status.as_str(),
                    t.subject
                );
                let data = json!({ "task": t });
                Ok(ToolResult {
                    call_id: call.id.clone(),
                    content,
                    is_error: false,
                    data: Some(data),
                })
            }
            None => Err(ToolError::Failed(format!(
                "no task with id {}",
                args.task_id
            ))),
        }
    }
}

/// Format a list of tasks as a compact plain-text block. Each line:
/// `id  [status]  subject`. Empty list → "(no tasks)".
fn render_list(items: &[crate::tasks::TaskItem]) -> String {
    if items.is_empty() {
        return "(no tasks)".to_owned();
    }
    let mut out = String::new();
    for t in items {
        let label = t.active_form.as_deref().unwrap_or(&t.subject);
        out.push_str(&format!(
            "#{:<3} [{:<11}] {}\n",
            t.id,
            t.status.as_str(),
            label,
        ));
    }
    out.trim_end().to_owned()
}
