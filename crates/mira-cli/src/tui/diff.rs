//! Diff previews for `edit_file` / `write_file` approvals.
//!
//! When the model asks to change a file, we'd rather show the user a
//! coloured unified diff than a raw JSON args blob. This module reads
//! the target file, applies the proposed edit in memory, and produces a
//! grouped diff (3 lines of context) that [`crate::tui::render`] paints
//! red/green in the approval modal.

use std::path::Path;

use mira_core::ToolCall;
use serde_json::Value;
use similar::{ChangeTag, TextDiff};

#[derive(Clone, Debug)]
pub struct DiffPreview {
    pub path: String,
    pub kind: DiffKind,
    pub lines: Vec<DiffLine>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DiffKind {
    /// `edit_file` — modifying existing content.
    Edit,
    /// `write_file` on an existing path — overwrite.
    Overwrite,
    /// `write_file` on a new path — creation.
    Create,
}

#[derive(Clone, Debug)]
pub enum DiffLine {
    /// Unchanged context line.
    Ctx(String),
    /// Added line (green, `+` prefix).
    Add(String),
    /// Removed line (red, `-` prefix).
    Del(String),
    /// Hunk separator between grouped edits (`···`).
    HunkGap,
}

const MAX_PREVIEW_LINES: usize = 40;

/// Read the target file, apply the proposed edit in memory, and return
/// the diff. Returns `None` for tools we don't know how to preview or
/// if the call's args don't parse.
pub async fn compute_preview(cwd: &Path, call: &ToolCall) -> Option<DiffPreview> {
    let args: Value = serde_json::from_str(&call.function.arguments).ok()?;
    let path = args.get("path")?.as_str()?.to_owned();
    let full = cwd.join(&path);

    let current = tokio::fs::read_to_string(&full).await.unwrap_or_default();
    let file_exists = full.exists();

    let (proposed, kind) = match call.function.name.as_str() {
        "write_file" => {
            let content = args.get("content")?.as_str()?.to_owned();
            let kind = if file_exists {
                DiffKind::Overwrite
            } else {
                DiffKind::Create
            };
            (content, kind)
        }
        "edit_file" => {
            let old = args.get("old_string")?.as_str()?;
            let new = args.get("new_string")?.as_str()?;
            let replace_all = args
                .get("replace_all")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let out = if replace_all {
                current.replace(old, new)
            } else {
                current.replacen(old, new, 1)
            };
            (out, DiffKind::Edit)
        }
        _ => return None,
    };

    Some(DiffPreview {
        path,
        kind,
        lines: render_diff(&current, &proposed),
    })
}

fn render_diff(before: &str, after: &str) -> Vec<DiffLine> {
    let diff = TextDiff::from_lines(before, after);
    let mut lines = Vec::new();

    for (idx, group) in diff.grouped_ops(3).into_iter().enumerate() {
        if idx > 0 {
            lines.push(DiffLine::HunkGap);
        }
        for op in group {
            for change in diff.iter_changes(&op) {
                let text = change.value().trim_end_matches('\n').to_owned();
                lines.push(match change.tag() {
                    ChangeTag::Delete => DiffLine::Del(text),
                    ChangeTag::Insert => DiffLine::Add(text),
                    ChangeTag::Equal => DiffLine::Ctx(text),
                });
                if lines.len() >= MAX_PREVIEW_LINES {
                    lines.push(DiffLine::HunkGap);
                    lines.push(DiffLine::Ctx(format!(
                        "… diff truncated at {MAX_PREVIEW_LINES} lines"
                    )));
                    return lines;
                }
            }
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use mira_core::message::{ToolCallFunction, ToolCallKind};
    use mira_core::{ToolCall, ToolCallId};
    use serde_json::json;
    use std::path::PathBuf;

    fn call(name: &str, args: Value) -> ToolCall {
        ToolCall {
            id: ToolCallId::from("call_test"),
            kind: ToolCallKind::Function,
            function: ToolCallFunction {
                name: name.to_owned(),
                arguments: args.to_string(),
            },
        }
    }

    #[tokio::test]
    async fn edit_produces_diff() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("foo.txt");
        tokio::fs::write(&path, "hello\nworld\n").await.unwrap();

        let c = call(
            "edit_file",
            json!({ "path": "foo.txt", "old_string": "world", "new_string": "there" }),
        );
        let preview = compute_preview(tmp.path(), &c).await.unwrap();
        assert_eq!(preview.kind, DiffKind::Edit);
        assert!(preview
            .lines
            .iter()
            .any(|l| matches!(l, DiffLine::Del(s) if s == "world")));
        assert!(preview
            .lines
            .iter()
            .any(|l| matches!(l, DiffLine::Add(s) if s == "there")));
    }

    #[tokio::test]
    async fn write_new_file_is_create() {
        let tmp = tempfile::tempdir().unwrap();
        let c = call(
            "write_file",
            json!({ "path": "new.txt", "content": "line one\nline two\n" }),
        );
        let preview = compute_preview(tmp.path(), &c).await.unwrap();
        assert_eq!(preview.kind, DiffKind::Create);
        assert_eq!(
            preview
                .lines
                .iter()
                .filter(|l| matches!(l, DiffLine::Add(_)))
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn non_edit_tool_returns_none() {
        let c = call("bash", json!({ "command": "ls" }));
        let preview = compute_preview(&PathBuf::from("/tmp"), &c).await;
        assert!(preview.is_none());
    }
}
