//! Diff previews for `edit_file` / `write_file` approvals.
//!
//! When the model asks to change a file, both the TUI approval modal and
//! the web UI approval modal render a coloured diff of the proposed
//! change instead of a raw JSON args blob. This module reads the target
//! file, applies the edit in memory, and produces a grouped diff (3
//! lines of context) that any frontend can style.
//!
//! Types derive `Serialize`/`Deserialize` so the server can put a
//! [`DiffPreview`] straight on the WebSocket.

use std::path::Path;

use mira_core::ToolCall;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use similar::{ChangeTag, TextDiff};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiffPreview {
    pub path: String,
    pub kind: DiffKind,
    pub lines: Vec<DiffLine>,
    /// Hunk structure with real old/new line numbers — the rich view
    /// renderers prefer. Additive: previews persisted before this
    /// field existed deserialize with an empty vec (serde default) and
    /// fall back to [`Self::lines`], so old session records keep
    /// loading and the wire format only grows.
    #[serde(default)]
    pub hunks: Vec<DiffHunk>,
    /// True when the diff hit [`MAX_PREVIEW_LINES`] — renderers show a
    /// truncation row instead of pretending the diff is complete.
    #[serde(default)]
    pub truncated: bool,
}

/// One contiguous hunk: a run of context + changed rows with no gap.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiffHunk {
    /// 1-based line number in the OLD file where this hunk starts
    /// (first `del` or `ctx` row). `0` for pure creations.
    pub old_start: u32,
    /// 1-based line number in the NEW file where this hunk starts.
    pub new_start: u32,
    pub rows: Vec<DiffRow>,
}

/// One row of a unified diff, carrying its position in each file.
/// Old/new numbers are what makes the stream read like a real patch:
/// a removed line and the lines that replace it share the region
/// between their respective numbers.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "tag", rename_all = "lowercase")]
pub enum DiffRow {
    /// Unchanged line — present in both files.
    Ctx { old: u32, new: u32, text: String },
    /// Added line — exists only in the new file.
    Add { new: u32, text: String },
    /// Removed line — exists only in the old file.
    Del { old: u32, text: String },
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiffKind {
    /// `edit_file` — modifying existing content.
    Edit,
    /// `write_file` on an existing path — overwrite.
    Overwrite,
    /// `write_file` on a new path — creation.
    Create,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "tag", content = "text", rename_all = "lowercase")]
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

    let diff = render_diff(&current, &proposed);
    let hunks = diff.hunks;
    let truncated = diff.truncated;
    Some(DiffPreview {
        path,
        kind,
        lines: diff.lines,
        hunks,
        truncated,
    })
}

struct RenderedDiff {
    /// Flat compat view (unchanged shape, includes the truncation
    /// notice embedded when truncated).
    lines: Vec<DiffLine>,
    /// Numbered hunk structure for the rich renderer.
    hunks: Vec<DiffHunk>,
    truncated: bool,
}

fn render_diff(before: &str, after: &str) -> RenderedDiff {
    let diff = TextDiff::from_lines(before, after);
    let mut lines = Vec::new();
    let mut hunks: Vec<DiffHunk> = Vec::new();
    let mut truncated = false;

    for (idx, group) in diff.grouped_ops(3).into_iter().enumerate() {
        if idx > 0 {
            lines.push(DiffLine::HunkGap);
        }
        let mut rows: Vec<DiffRow> = Vec::new();
        for op in group {
            for change in diff.iter_changes(&op) {
                let text = change.value().trim_end_matches('\n').to_owned();
                // `similar` indices are 0-based; diffs are 1-based.
                let old = change.old_index().map(|i| (i + 1) as u32);
                let new = change.new_index().map(|i| (i + 1) as u32);
                lines.push(match change.tag() {
                    ChangeTag::Delete => DiffLine::Del(text.clone()),
                    ChangeTag::Insert => DiffLine::Add(text.clone()),
                    ChangeTag::Equal => DiffLine::Ctx(text.clone()),
                });
                match change.tag() {
                    ChangeTag::Delete => {
                        rows.push(DiffRow::Del {
                            old: old.unwrap_or(0),
                            text,
                        });
                    }
                    ChangeTag::Insert => {
                        rows.push(DiffRow::Add {
                            new: new.unwrap_or(0),
                            text,
                        });
                    }
                    ChangeTag::Equal => {
                        rows.push(DiffRow::Ctx {
                            old: old.unwrap_or(0),
                            new: new.unwrap_or(0),
                            text,
                        });
                    }
                }
                if lines.len() >= MAX_PREVIEW_LINES {
                    truncated = true;
                    lines.push(DiffLine::HunkGap);
                    lines.push(DiffLine::Ctx(format!(
                        "… diff truncated at {MAX_PREVIEW_LINES} lines"
                    )));
                    if !rows.is_empty() {
                        hunks.push(finish_hunk(rows));
                    }
                    return RenderedDiff {
                        lines,
                        hunks,
                        truncated,
                    };
                }
            }
        }
        if !rows.is_empty() {
            hunks.push(finish_hunk(rows));
        }
    }
    RenderedDiff {
        lines,
        hunks,
        truncated,
    }
}

/// Collapse the accumulated rows into a hunk, deriving its start
/// numbers from the first row that carries each file's position.
fn finish_hunk(rows: Vec<DiffRow>) -> DiffHunk {
    let old_start = rows.iter().find_map(|r| match r {
        DiffRow::Ctx { old, .. } | DiffRow::Del { old, .. } => Some(*old),
        DiffRow::Add { .. } => None,
    });
    let new_start = rows.iter().find_map(|r| match r {
        DiffRow::Ctx { new, .. } | DiffRow::Add { new, .. } => Some(*new),
        DiffRow::Del { .. } => None,
    });
    DiffHunk {
        old_start: old_start.unwrap_or(0),
        new_start: new_start.unwrap_or(0),
        rows,
    }
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
    async fn modification_carries_old_and_new_numbers() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("foo.txt");
        tokio::fs::write(&path, "a\nb\nc\nd\ne\nf\ng\nh\n")
            .await
            .unwrap();

        let c = call(
            "edit_file",
            json!({ "path": "foo.txt", "old_string": "d", "new_string": "D!" }),
        );
        let preview = compute_preview(tmp.path(), &c).await.unwrap();
        assert_eq!(preview.hunks.len(), 1);
        let h = &preview.hunks[0];
        // 3 lines of context each side, all with both numbers.
        let ctx: Vec<(u32, u32)> = h
            .rows
            .iter()
            .filter_map(|r| match r {
                DiffRow::Ctx { old, new, .. } => Some((*old, *new)),
                _ => None,
            })
            .collect();
        // grouped_ops(3): three context lines on each side of the change.
        assert!(ctx.contains(&(1, 1)));
        assert!(ctx.contains(&(7, 7)));
        let dels: Vec<u32> = h
            .rows
            .iter()
            .filter_map(|r| match r {
                DiffRow::Del { old, .. } => Some(*old),
                _ => None,
            })
            .collect();
        assert_eq!(dels, vec![4]);
        let adds: Vec<u32> = h
            .rows
            .iter()
            .filter_map(|r| match r {
                DiffRow::Add { new, .. } => Some(*new),
                _ => None,
            })
            .collect();
        assert_eq!(adds, vec![4]);
        assert!(!preview.truncated);
    }

    #[tokio::test]
    async fn distant_edits_split_into_hunks_with_shifted_numbers() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("foo.txt");
        // `target` on lines 2 and 25; replace_all turns one call into
        // two changes far apart → two hunks.
        let mut body = String::new();
        for i in 1..=30 {
            if i == 2 || i == 25 {
                body.push_str("target\n");
            } else {
                body.push_str(&format!("pad {i}\n"));
            }
        }
        tokio::fs::write(&path, body).await.unwrap();

        let c = call(
            "edit_file",
            json!({
                "path": "foo.txt",
                "old_string": "target",
                "new_string": "TARGET\ninserted",
                "replace_all": true
            }),
        );
        let preview = compute_preview(tmp.path(), &c).await.unwrap();
        assert_eq!(preview.hunks.len(), 2);
        // After the first insertion, context rows in the second hunk
        // must show new = old + 1 — the shift a unified diff reader
        // expects to see.
        let shifted: Vec<(u32, u32)> = preview.hunks[1]
            .rows
            .iter()
            .filter_map(|r| match r {
                DiffRow::Ctx { old, new, .. } => Some((*old, *new)),
                _ => None,
            })
            .collect();
        assert!(!shifted.is_empty());
        // Every context row shifts by the number of insertions above
        // it: +1 before the second change site, +2 after it.
        assert!(shifted.iter().all(|(o, n)| *n > *o), "{shifted:?}");
        assert_eq!(shifted[0], (22, 23));
        assert_eq!(*shifted.last().unwrap(), (28, 30));
    }

    #[tokio::test]
    async fn creation_numbers_start_at_one() {
        let tmp = tempfile::tempdir().unwrap();
        let c = call(
            "write_file",
            json!({ "path": "new.txt", "content": "one\ntwo\n" }),
        );
        let preview = compute_preview(tmp.path(), &c).await.unwrap();
        let h = &preview.hunks[0];
        assert_eq!(h.old_start, 0);
        assert_eq!(h.new_start, 1);
        assert!(h.rows.iter().all(|r| matches!(r, DiffRow::Add { .. })));
    }

    #[tokio::test]
    async fn non_edit_tool_returns_none() {
        let c = call("bash", json!({ "command": "ls" }));
        let preview = compute_preview(&PathBuf::from("/tmp"), &c).await;
        assert!(preview.is_none());
    }
}
