//! Structured multi-file patch tool.
//!
//! The `edit_file` tool is fine for a single find-and-replace, but a
//! model that wants to make three changes across two files still has to
//! issue three round-trips.`apply_patch` DSL is the escape
//! hatch: one tool call, one atomic-ish batch of edits, plus adds /
//! deletes / renames.
//!
//! ## DSL
//!
//! ```text
//! *** Begin Patch
//! *** Add File: path/new.rs
//! +fn foo() {}
//! +
//! +fn bar() {}
//! *** Update File: path/existing.rs
//! @@ optional context anchor
//!  unchanged context line
//! -removed line
//! +added line
//!  unchanged context line
//! *** Delete File: path/gone.rs
//! *** Move File: old/path.rs -> new/path.rs
//! *** End Patch
//! ```
//!
//! ### Body-line prefixes (Update / Move-with-edits)
//!
//! * ` `  — context (kept)
//! * `-`  — removed
//! * `+`  — added
//! * `@@` — anchor hint, restricts the hunk search to lines after the
//!   first anchor match. Optional.
//!
//! `Add File` bodies are all `+`-prefixed. `Delete File` has no body.
//! `Move File` accepts an optional hunk body that runs against the
//! *destination* after the rename.
//!
//! ## Applier semantics
//!
//! * Every mutation goes through [`FileGuard`] the same way `edit_file`
//!   does — pre-image snapshot for undo, conflict watermark honored on
//!   any file previously seen by `read_file`.
//! * Hunks apply in the order they appear. Each hunk's search block
//!   (context + removes) must occur exactly once **in the remaining
//!   region** of the file — otherwise the tool refuses rather than
//!   guess.
//! * Failure semantics are best-effort: we apply as much as we can,
//!   but on the first hard error we stop and surface the failure so
//!   the model can retry. Files already written stay written; the
//!   guard's snapshots let the user (or `mira undo`) revert.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use mira_ai::ToolSpec;
use mira_core::{ToolCall, ToolResult};
use serde::Deserialize;
use serde_json::json;
use tokio::fs;

use crate::context::ToolContext;
use crate::tool::{spec, Action, Tool, ToolError};

pub struct ApplyPatch;

#[derive(Deserialize)]
struct Args {
    /// The full patch envelope, including the `*** Begin Patch` /
    /// `*** End Patch` sentinels.
    patch: String,
}

#[async_trait]
impl Tool for ApplyPatch {
    fn spec(&self) -> ToolSpec {
        spec(
            "apply_patch",
            "Apply a structured multi-file patch. Prefer this over \
             `edit_file` when making related changes across more than one \
             hunk or file — it's atomic-per-file and deterministic. \
             \
             DSL: envelope with `*** Begin Patch` and `*** End Patch`. \
             Sections: `*** Add File: <path>` (body is +lines), \
             `*** Update File: <path>` (body is diff-shaped: ` `/`-`/`+` \
             line prefixes, optional `@@ anchor` before a hunk), \
             `*** Delete File: <path>`, `*** Move File: <from> -> <to>` \
             (optional hunk body applies after the rename). \
             \
             Read the file first so context lines match byte-for-byte. \
             Each hunk's context+removes must appear exactly once in the \
             remaining region; add more context to disambiguate if not.",
            json!({
                "type": "object",
                "properties": {
                    "patch": { "type": "string" }
                },
                "required": ["patch"],
                "additionalProperties": false
            }),
        )
    }

    fn action(&self) -> Action {
        // The whole batch is treated as a single Edit for policy. Individual
        // Add/Delete/Move don't have a natural bucket and Edit is the
        // closest existing rule family — a user allowing `Edit(src/**)`
        // implicitly allows apply_patch there too.
        Action::Edit
    }

    fn policy_target(&self, call: &ToolCall) -> String {
        // Surface the first affected path so a rule keyed on a directory
        // still fires. Best-effort: if parsing fails, empty target — the
        // engine will fall through to the default gate for `Edit`.
        //
        // Kept for backwards-compat with the single-target contract; the
        // *actual* per-op containment happens via `policy_targets` below,
        // which the dispatcher evaluates against every path this call
        // would touch.
        let args: Args = match call.parse_arguments() {
            Ok(a) => a,
            Err(_) => return String::new(),
        };
        match parse_patch(&args.patch) {
            Ok(ops) => ops
                .first()
                .map(|op| op.primary_path().to_string())
                .unwrap_or_default(),
            Err(_) => String::new(),
        }
    }

    fn policy_targets(&self, call: &ToolCall) -> Vec<String> {
        // Return every path the patch would touch — Add/Update/Delete
        // targets, plus BOTH sides of any Move. The dispatcher evaluates
        // each entry independently, so a patch that would edit `src/a.rs`
        // *and* `.env` under `Edit(src/**)` no longer sneaks through
        // just because `src/a.rs` happens to come first.
        //
        // Falls back to the single-target default when parsing fails so
        // an invalid patch still gets a gate (and rejects cleanly inside
        // `invoke` with a parse error).
        let args: Args = match call.parse_arguments() {
            Ok(a) => a,
            Err(_) => return vec![self.policy_target(call)],
        };
        match parse_patch(&args.patch) {
            Ok(ops) => {
                let mut out: Vec<String> = Vec::new();
                for op in ops {
                    for p in op.all_paths() {
                        out.push(p.to_string());
                    }
                }
                if out.is_empty() {
                    vec![String::new()]
                } else {
                    out
                }
            }
            Err(_) => vec![self.policy_target(call)],
        }
    }

    async fn invoke(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let args: Args = call.parse_arguments()?;
        let ops = parse_patch(&args.patch).map_err(ToolError::InvalidArgs)?;

        if ops.is_empty() {
            return Err(ToolError::InvalidArgs(
                "empty patch — no sections between `*** Begin Patch` and \
                 `*** End Patch`"
                    .into(),
            ));
        }

        // Pre-resolve every path so we fail before touching disk if any
        // escapes cwd. Also lets us report all offenders in one message.
        let mut resolved: Vec<ResolvedOp> = Vec::with_capacity(ops.len());
        for op in ops {
            resolved.push(op.resolve(ctx)?);
        }

        let mut summary: Vec<String> = Vec::new();
        for op in resolved {
            summary.push(op.apply(ctx).await?);
        }

        Ok(ToolResult::ok(
            call.id.clone(),
            format!(
                "applied {} change(s):\n{}",
                summary.len(),
                summary.join("\n")
            ),
        ))
    }
}

// ---------- IR ----------

#[derive(Debug, PartialEq, Eq)]
enum PatchOp {
    Add {
        path: String,
        body: String,
    },
    Update {
        path: String,
        hunks: Vec<Hunk>,
    },
    Delete {
        path: String,
    },
    Move {
        from: String,
        to: String,
        hunks: Vec<Hunk>,
    },
}

impl PatchOp {
    /// Every path this op touches — source AND destination for a Move,
    /// the single path for Add/Update/Delete. Used by
    /// `policy_targets` so every affected file gets its own policy
    /// check (matches the audit's Gap #1a fix: no more approving a
    /// batch on the strength of just its first path).
    fn all_paths(&self) -> Vec<&str> {
        match self {
            PatchOp::Add { path, .. } | PatchOp::Update { path, .. } | PatchOp::Delete { path } => {
                vec![path.as_str()]
            }
            PatchOp::Move { from, to, .. } => vec![from.as_str(), to.as_str()],
        }
    }

    fn primary_path(&self) -> &str {
        match self {
            PatchOp::Add { path, .. } | PatchOp::Update { path, .. } | PatchOp::Delete { path } => {
                path.as_str()
            }
            PatchOp::Move { from, .. } => from.as_str(),
        }
    }

    fn resolve(self, ctx: &ToolContext) -> Result<ResolvedOp, ToolError> {
        let resolve_one = |p: &str| {
            ctx.resolve(p)
                .ok_or_else(|| ToolError::Failed(format!("path escapes cwd: {p}")))
        };
        Ok(match self {
            PatchOp::Add { path, body } => ResolvedOp::Add {
                path: resolve_one(&path)?,
                body,
            },
            PatchOp::Update { path, hunks } => ResolvedOp::Update {
                path: resolve_one(&path)?,
                hunks,
            },
            PatchOp::Delete { path } => ResolvedOp::Delete {
                path: resolve_one(&path)?,
            },
            PatchOp::Move { from, to, hunks } => ResolvedOp::Move {
                from: resolve_one(&from)?,
                to: resolve_one(&to)?,
                hunks,
            },
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Hunk {
    /// Optional anchor line the search starts from. `None` = search the
    /// whole remaining region.
    anchor: Option<String>,
    lines: Vec<HunkLine>,
}

#[derive(Debug, PartialEq, Eq)]
enum HunkLine {
    Context(String),
    Add(String),
    Remove(String),
}

enum ResolvedOp {
    Add {
        path: PathBuf,
        body: String,
    },
    Update {
        path: PathBuf,
        hunks: Vec<Hunk>,
    },
    Delete {
        path: PathBuf,
    },
    Move {
        from: PathBuf,
        to: PathBuf,
        hunks: Vec<Hunk>,
    },
}

impl ResolvedOp {
    async fn apply(self, ctx: &ToolContext) -> Result<String, ToolError> {
        match self {
            ResolvedOp::Add { path, body } => apply_add(&path, &body, ctx).await,
            ResolvedOp::Update { path, hunks } => apply_update(&path, &hunks, ctx).await,
            ResolvedOp::Delete { path } => apply_delete(&path, ctx).await,
            ResolvedOp::Move { from, to, hunks } => apply_move(&from, &to, &hunks, ctx).await,
        }
    }
}

// ---------- Parser ----------

const BEGIN: &str = "*** Begin Patch";
const END: &str = "*** End Patch";
const ADD: &str = "*** Add File: ";
const UPDATE: &str = "*** Update File: ";
const DELETE: &str = "*** Delete File: ";
const MOVE: &str = "*** Move File: ";

/// Parse the DSL into ordered operations. Errors are user-facing —
/// return a description precise enough that the model can fix the
/// patch without a second round-trip.
fn parse_patch(input: &str) -> Result<Vec<PatchOp>, String> {
    // Locate the envelope. Tolerate leading/trailing blank lines and
    // fenced code blocks (```patch … ```) around the sentinels.
    let mut lines = input.lines();
    let mut in_envelope = false;
    let mut body: Vec<&str> = Vec::new();
    for line in lines.by_ref() {
        let trimmed = line.trim();
        if !in_envelope {
            if trimmed == BEGIN {
                in_envelope = true;
            }
            continue;
        }
        if trimmed == END {
            in_envelope = false;
            break;
        }
        body.push(line);
    }
    if in_envelope {
        return Err("patch missing `*** End Patch` sentinel".into());
    }
    if body.is_empty() && !input.trim().is_empty() && !input.contains(BEGIN) {
        return Err(format!("patch missing `{BEGIN}` sentinel"));
    }

    let mut ops: Vec<PatchOp> = Vec::new();
    let mut idx = 0;
    while idx < body.len() {
        let line = body[idx];
        let trimmed = line.trim_end_matches('\r');
        // Skip blank inter-section lines.
        if trimmed.is_empty() {
            idx += 1;
            continue;
        }
        if let Some(path) = trimmed.strip_prefix(ADD) {
            let path = path.trim().to_owned();
            if path.is_empty() {
                return Err("`*** Add File:` with empty path".into());
            }
            idx += 1;
            let (body_text, next) = take_add_body(&body, idx)?;
            ops.push(PatchOp::Add {
                path,
                body: body_text,
            });
            idx = next;
        } else if let Some(path) = trimmed.strip_prefix(UPDATE) {
            let path = path.trim().to_owned();
            if path.is_empty() {
                return Err("`*** Update File:` with empty path".into());
            }
            idx += 1;
            let (hunks, next) = take_hunks(&body, idx)?;
            if hunks.is_empty() {
                return Err(format!(
                    "`*** Update File: {path}` with no hunks — add at least one \
                     line prefixed with ` `, `+`, `-`, or `@@`"
                ));
            }
            ops.push(PatchOp::Update { path, hunks });
            idx = next;
        } else if let Some(path) = trimmed.strip_prefix(DELETE) {
            let path = path.trim().to_owned();
            if path.is_empty() {
                return Err("`*** Delete File:` with empty path".into());
            }
            ops.push(PatchOp::Delete { path });
            idx += 1;
        } else if let Some(spec) = trimmed.strip_prefix(MOVE) {
            let (from, to) = parse_move_spec(spec)?;
            idx += 1;
            let (hunks, next) = take_hunks(&body, idx)?;
            ops.push(PatchOp::Move { from, to, hunks });
            idx = next;
        } else {
            return Err(format!(
                "unrecognised section header (expected `*** Add/Update/Delete/Move File: …`): {line:?}"
            ));
        }
    }

    Ok(ops)
}

fn parse_move_spec(spec: &str) -> Result<(String, String), String> {
    let parts: Vec<&str> = spec.splitn(2, " -> ").collect();
    if parts.len() != 2 {
        return Err(format!(
            "`*** Move File:` needs `<from> -> <to>`, got: {spec:?}"
        ));
    }
    let from = parts[0].trim();
    let to = parts[1].trim();
    if from.is_empty() || to.is_empty() {
        return Err("`*** Move File:` needs non-empty from/to paths".into());
    }
    Ok((from.to_owned(), to.to_owned()))
}

/// Read `+` lines until the next section header (or EOF). Rejects any
/// other prefix — an Add body is pure insertion.
fn take_add_body(body: &[&str], mut idx: usize) -> Result<(String, usize), String> {
    let mut out = String::new();
    let mut wrote_any = false;
    while idx < body.len() {
        let line = body[idx];
        if is_section_header(line) {
            break;
        }
        // Blank lines inside an Add body are tolerated — treat as empty
        // additions. Otherwise require a `+` prefix.
        if line.is_empty() {
            out.push('\n');
            idx += 1;
            continue;
        }
        let Some(rest) = line.strip_prefix('+') else {
            return Err(format!(
                "`*** Add File` body line must start with `+`, got: {line:?}"
            ));
        };
        out.push_str(rest);
        out.push('\n');
        wrote_any = true;
        idx += 1;
    }
    if !wrote_any && out.is_empty() {
        // Allow an empty file — DSL supports creating a zero-byte file
        // by having no body. But require the caller to be explicit; a
        // section with zero body lines and no `+` lines is likely a
        // mistake, so we still return Ok here rather than error.
    }
    // Drop a trailing empty line if the DSL added one from a blank
    // separator; the last `+` provided its own newline already.
    if out.ends_with("\n\n") {
        out.pop();
    }
    Ok((out, idx))
}

/// Read hunk lines (context / add / remove / anchor) until the next
/// section header or EOF. Blank lines are treated as empty context.
fn take_hunks(body: &[&str], mut idx: usize) -> Result<(Vec<Hunk>, usize), String> {
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut current: Option<Hunk> = None;

    while idx < body.len() {
        let line = body[idx];
        if is_section_header(line) {
            break;
        }

        if let Some(anchor) = line.strip_prefix("@@") {
            // Start a new hunk with this anchor. Anchor text is
            // whatever follows `@@` (trimmed of one leading space).
            let anchor = anchor.strip_prefix(' ').unwrap_or(anchor).to_owned();
            if let Some(h) = current.take() {
                hunks.push(h);
            }
            current = Some(Hunk {
                anchor: (!anchor.is_empty()).then_some(anchor),
                lines: Vec::new(),
            });
            idx += 1;
            continue;
        }

        let hunk_line = if line.is_empty() {
            HunkLine::Context(String::new())
        } else if let Some(rest) = line.strip_prefix('+') {
            HunkLine::Add(rest.to_owned())
        } else if let Some(rest) = line.strip_prefix('-') {
            HunkLine::Remove(rest.to_owned())
        } else if let Some(rest) = line.strip_prefix(' ') {
            HunkLine::Context(rest.to_owned())
        } else {
            return Err(format!(
                "hunk body line must start with ` `, `+`, `-`, or `@@` — got: {line:?}"
            ));
        };

        current
            .get_or_insert_with(|| Hunk {
                anchor: None,
                lines: Vec::new(),
            })
            .lines
            .push(hunk_line);
        idx += 1;
    }

    if let Some(h) = current.take() {
        hunks.push(h);
    }

    // Reject hunks with no additions and no removals — they'd be no-ops
    // that still consume search context, which is almost certainly a
    // model mistake worth surfacing.
    for h in &hunks {
        let has_change = h
            .lines
            .iter()
            .any(|l| matches!(l, HunkLine::Add(_) | HunkLine::Remove(_)));
        if !has_change {
            return Err(
                "hunk has no `+`/`-` lines — it would be a no-op. Remove it or add a change."
                    .into(),
            );
        }
    }

    Ok((hunks, idx))
}

fn is_section_header(line: &str) -> bool {
    let t = line.trim_end_matches('\r');
    t.starts_with(ADD) || t.starts_with(UPDATE) || t.starts_with(DELETE) || t.starts_with(MOVE)
}

// ---------- Applier ----------

async fn apply_add(path: &Path, body: &str, ctx: &ToolContext) -> Result<String, ToolError> {
    if fs::metadata(path).await.is_ok() {
        return Err(ToolError::Failed(format!(
            "add failed: {} already exists — use Update or Delete first",
            path.display()
        )));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }
    // No conflict check — brand-new path can't have a stale watermark.
    if let Some(g) = &ctx.guard {
        g.snapshot_before(path)
            .await
            .map_err(|e| ToolError::Failed(e.to_string()))?;
    }
    fs::write(path, body.as_bytes()).await?;
    Ok(format!("added {} ({} bytes)", path.display(), body.len()))
}

async fn apply_delete(path: &Path, ctx: &ToolContext) -> Result<String, ToolError> {
    if fs::metadata(path).await.is_err() {
        return Err(ToolError::Failed(format!(
            "delete failed: {} does not exist",
            path.display()
        )));
    }
    if let Some(g) = &ctx.guard {
        g.check_conflict(path)
            .await
            .map_err(|e| ToolError::Failed(e.to_string()))?;
        g.snapshot_before(path)
            .await
            .map_err(|e| ToolError::Failed(e.to_string()))?;
    }
    fs::remove_file(path).await?;
    Ok(format!("deleted {}", path.display()))
}

async fn apply_update(path: &Path, hunks: &[Hunk], ctx: &ToolContext) -> Result<String, ToolError> {
    if let Some(g) = &ctx.guard {
        g.check_conflict(path)
            .await
            .map_err(|e| ToolError::Failed(e.to_string()))?;
    }
    let contents = fs::read_to_string(path).await.map_err(|e| {
        ToolError::Failed(format!(
            "update failed: cannot read {}: {e}",
            path.display()
        ))
    })?;
    let updated = apply_hunks_to_string(&contents, hunks, path)?;
    if updated == contents {
        return Err(ToolError::Failed(format!(
            "update produced no change to {} — hunk context matched but +/- \
             lines were identical",
            path.display()
        )));
    }
    if let Some(g) = &ctx.guard {
        g.snapshot_before(path)
            .await
            .map_err(|e| ToolError::Failed(e.to_string()))?;
    }
    fs::write(path, updated.as_bytes()).await?;
    Ok(format!(
        "updated {} ({} hunk{})",
        path.display(),
        hunks.len(),
        if hunks.len() == 1 { "" } else { "s" }
    ))
}

async fn apply_move(
    from: &Path,
    to: &Path,
    hunks: &[Hunk],
    ctx: &ToolContext,
) -> Result<String, ToolError> {
    if fs::metadata(from).await.is_err() {
        return Err(ToolError::Failed(format!(
            "move failed: source {} does not exist",
            from.display()
        )));
    }
    if fs::metadata(to).await.is_ok() {
        return Err(ToolError::Failed(format!(
            "move failed: destination {} already exists",
            to.display()
        )));
    }
    if let Some(g) = &ctx.guard {
        g.check_conflict(from)
            .await
            .map_err(|e| ToolError::Failed(e.to_string()))?;
        // Record source as a delete-style snapshot so undo restores it,
        // and the destination as a create so undo removes it. This is
        // the same two-entry pattern edit_file would produce for the
        // rename-with-modify shape.
        g.snapshot_before(from)
            .await
            .map_err(|e| ToolError::Failed(e.to_string()))?;
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).await?;
    }
    fs::rename(from, to).await?;
    if let Some(g) = &ctx.guard {
        g.snapshot_before(to)
            .await
            .map_err(|e| ToolError::Failed(e.to_string()))?;
    }

    let summary = if hunks.is_empty() {
        format!("moved {} -> {}", from.display(), to.display())
    } else {
        let contents = fs::read_to_string(to).await?;
        let updated = apply_hunks_to_string(&contents, hunks, to)?;
        fs::write(to, updated.as_bytes()).await?;
        format!(
            "moved {} -> {} ({} hunk{})",
            from.display(),
            to.display(),
            hunks.len(),
            if hunks.len() == 1 { "" } else { "s" }
        )
    };

    Ok(summary)
}

/// Core edit engine — given the file's current text and an ordered set
/// of hunks, produce the updated text. Never touches disk.
fn apply_hunks_to_string(original: &str, hunks: &[Hunk], path: &Path) -> Result<String, ToolError> {
    // Work in Vec<String> so hunks can insert/remove full lines cleanly
    // without paying for repeated string surgery.
    let mut lines: Vec<String> = original
        .split_inclusive('\n')
        .map(|s| s.to_owned())
        .collect();
    // Track a per-hunk cursor so later hunks can't match earlier text —
    // this is what disambiguates repeated regions when the model
    // supplies hunks in file order without anchors.
    let mut cursor: usize = 0;

    for (i, hunk) in hunks.iter().enumerate() {
        let start = if let Some(anchor) = &hunk.anchor {
            // Match a *line* whose content (with trailing newline
            // stripped) contains the anchor. That's forgiving enough to
            // survive minor whitespace drift while still being
            // unambiguous in practice.
            let hit = lines[cursor..]
                .iter()
                .position(|l| strip_nl(l).contains(anchor.as_str()));
            match hit {
                Some(pos) => cursor + pos,
                None => {
                    return Err(ToolError::Failed(format!(
                        "hunk {}: anchor {:?} not found after line {} in {}",
                        i + 1,
                        anchor,
                        cursor + 1,
                        path.display()
                    )));
                }
            }
        } else {
            cursor
        };

        // "search block" — the sequence we expect to find (context +
        // removes, in the DSL's order).
        let search: Vec<&str> = hunk
            .lines
            .iter()
            .filter_map(|l| match l {
                HunkLine::Context(s) | HunkLine::Remove(s) => Some(s.as_str()),
                HunkLine::Add(_) => None,
            })
            .collect();

        if search.is_empty() {
            return Err(ToolError::Failed(format!(
                "hunk {}: pure-insertion hunks need at least one context (` `) line as an anchor in {}",
                i + 1,
                path.display()
            )));
        }

        let match_pos = find_unique(&lines, start, &search)
            .map_err(|e| ToolError::Failed(format!("hunk {} in {}: {e}", i + 1, path.display())))?;

        // Build the replacement — context + adds — preserving the
        // original line's trailing newline where possible so we don't
        // silently strip / add EOL bytes.
        let end = match_pos + search.len();
        let default_nl = lines
            .get(match_pos)
            .map(|l| if l.ends_with('\n') { "\n" } else { "" })
            .unwrap_or("\n");
        let replacement: Vec<String> = hunk
            .lines
            .iter()
            .filter_map(|l| match l {
                HunkLine::Remove(_) => None,
                HunkLine::Context(s) | HunkLine::Add(s) => Some(format!("{s}{default_nl}")),
            })
            .collect();

        // Splice.
        lines.splice(match_pos..end, replacement.iter().cloned());
        cursor = match_pos + replacement.len();
    }

    Ok(lines.concat())
}

/// Locate the unique occurrence of `needle` (a sequence of line bodies,
/// no trailing newline) in `lines[start..]`. Comparison strips the
/// trailing `\n` from each haystack line. Returns the absolute index of
/// the first match, or an error if there is zero or more than one.
fn find_unique(lines: &[String], start: usize, needle: &[&str]) -> Result<usize, String> {
    if needle.is_empty() {
        return Err("empty search block".into());
    }
    if start >= lines.len() {
        return Err(format!(
            "search block runs past end of file (start line {} of {})",
            start + 1,
            lines.len()
        ));
    }
    let last = lines.len().saturating_sub(needle.len());
    let mut hits: Vec<usize> = Vec::new();
    for i in start..=last {
        let mut ok = true;
        for (j, want) in needle.iter().enumerate() {
            if strip_nl(&lines[i + j]) != *want {
                ok = false;
                break;
            }
        }
        if ok {
            hits.push(i);
            if hits.len() > 1 {
                break;
            }
        }
    }
    match hits.len() {
        0 => Err(
            "search block did not match — check context lines byte-for-byte, \
                  or add an `@@` anchor to disambiguate"
                .into(),
        ),
        1 => Ok(hits[0]),
        _ => Err(
            "search block matched more than once — add more context lines \
             or an `@@` anchor above the hunk"
                .into(),
        ),
    }
}

fn strip_nl(s: &str) -> &str {
    let s = s.strip_suffix('\n').unwrap_or(s);
    s.strip_suffix('\r').unwrap_or(s)
}

// ---------- Tests ----------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn ctx(cwd: &Path) -> ToolContext {
        // The sandbox handle is required by the struct but no test path
        // exercises it — the default_scrubbed constructor gives a valid,
        // unused instance.
        let sandbox = Arc::new(mira_sandbox::Sandbox::default_scrubbed());
        ToolContext::new(cwd, sandbox)
    }

    // ----- policy targets (audit Gap #1a regression) -----

    #[test]
    fn policy_targets_lists_every_affected_path_including_move_dest() {
        let patch = "\
*** Begin Patch
*** Add File: src/a.rs
+hi
*** Delete File: .env
*** Move File: old.txt -> new.txt
*** End Patch
";
        let call = ToolCall {
            id: mira_core::ToolCallId::from("t1"),
            kind: mira_core::message::ToolCallKind::Function,
            function: mira_core::message::ToolCallFunction {
                name: "apply_patch".to_owned(),
                arguments: serde_json::to_string(&serde_json::json!({ "patch": patch })).unwrap(),
            },
        };
        let targets = ApplyPatch.policy_targets(&call);
        // Every path shows up: src/a.rs (add), .env (delete), old.txt +
        // new.txt (move source + destination). The old behaviour returned
        // only the FIRST — `src/a.rs` — so a Deny(Edit("**/.env")) rule
        // never fired on this batch.
        assert_eq!(targets, vec!["src/a.rs", ".env", "old.txt", "new.txt"]);
    }

    // ----- parser -----

    #[test]
    fn parse_add_and_delete() {
        let patch = "\
*** Begin Patch
*** Add File: a.txt
+hello
+world
*** Delete File: b.txt
*** End Patch
";
        let ops = parse_patch(patch).unwrap();
        assert_eq!(ops.len(), 2);
        match &ops[0] {
            PatchOp::Add { path, body } => {
                assert_eq!(path, "a.txt");
                assert_eq!(body, "hello\nworld\n");
            }
            _ => panic!("expected Add"),
        }
        match &ops[1] {
            PatchOp::Delete { path } => assert_eq!(path, "b.txt"),
            _ => panic!("expected Delete"),
        }
    }

    #[test]
    fn parse_update_single_hunk() {
        let patch = "\
*** Begin Patch
*** Update File: src/x.rs
 keep
-drop
+add
 keep tail
*** End Patch
";
        let ops = parse_patch(patch).unwrap();
        let PatchOp::Update { path, hunks } = &ops[0] else {
            panic!("expected Update");
        };
        assert_eq!(path, "src/x.rs");
        assert_eq!(hunks.len(), 1);
        assert!(hunks[0].anchor.is_none());
        assert_eq!(hunks[0].lines.len(), 4);
    }

    #[test]
    fn parse_update_with_anchor() {
        let patch = "\
*** Begin Patch
*** Update File: src/x.rs
@@ impl Foo {
 first
-second
+SECOND
 third
*** End Patch
";
        let ops = parse_patch(patch).unwrap();
        let PatchOp::Update { hunks, .. } = &ops[0] else {
            unreachable!()
        };
        assert_eq!(hunks[0].anchor.as_deref(), Some("impl Foo {"));
    }

    #[test]
    fn parse_move_with_hunks() {
        let patch = "\
*** Begin Patch
*** Move File: old.rs -> new.rs
 keep
-remove
+add
*** End Patch
";
        let ops = parse_patch(patch).unwrap();
        let PatchOp::Move { from, to, hunks } = &ops[0] else {
            panic!("expected Move");
        };
        assert_eq!(from, "old.rs");
        assert_eq!(to, "new.rs");
        assert_eq!(hunks.len(), 1);
    }

    #[test]
    fn parse_rejects_missing_end() {
        let err = parse_patch("*** Begin Patch\n*** Add File: x\n+a\n").unwrap_err();
        assert!(err.contains("End Patch"), "err: {err}");
    }

    #[test]
    fn parse_rejects_bad_body_prefix() {
        let err = parse_patch("*** Begin Patch\n*** Update File: x\nnaked line\n*** End Patch\n")
            .unwrap_err();
        assert!(err.contains("must start with"), "err: {err}");
    }

    #[test]
    fn parse_rejects_noop_hunk() {
        let err =
            parse_patch("*** Begin Patch\n*** Update File: x\n context only\n*** End Patch\n")
                .unwrap_err();
        assert!(err.contains("no-op"), "err: {err}");
    }

    #[test]
    fn parse_tolerates_leading_junk() {
        let patch = "\
some preamble
```patch
*** Begin Patch
*** Delete File: gone.txt
*** End Patch
```
trailing
";
        let ops = parse_patch(patch).unwrap();
        assert_eq!(ops.len(), 1);
    }

    // ----- applier -----

    #[test]
    fn apply_hunk_basic_replace() {
        let src = "one\ntwo\nthree\n";
        let hunks = vec![Hunk {
            anchor: None,
            lines: vec![
                HunkLine::Context("one".into()),
                HunkLine::Remove("two".into()),
                HunkLine::Add("TWO".into()),
                HunkLine::Context("three".into()),
            ],
        }];
        let out = apply_hunks_to_string(src, &hunks, Path::new("x")).unwrap();
        assert_eq!(out, "one\nTWO\nthree\n");
    }

    #[test]
    fn apply_hunk_multiple_hunks_in_order() {
        let src = "a\nb\nc\nd\ne\n";
        let hunks = vec![
            Hunk {
                anchor: None,
                lines: vec![
                    HunkLine::Context("a".into()),
                    HunkLine::Remove("b".into()),
                    HunkLine::Add("B".into()),
                ],
            },
            Hunk {
                anchor: None,
                lines: vec![
                    HunkLine::Context("d".into()),
                    HunkLine::Remove("e".into()),
                    HunkLine::Add("E".into()),
                ],
            },
        ];
        let out = apply_hunks_to_string(src, &hunks, Path::new("x")).unwrap();
        assert_eq!(out, "a\nB\nc\nd\nE\n");
    }

    #[test]
    fn apply_hunk_ambiguous_context_rejected() {
        let src = "x\ny\nz\nx\ny\nz\n";
        let hunks = vec![Hunk {
            anchor: None,
            lines: vec![
                HunkLine::Context("x".into()),
                HunkLine::Remove("y".into()),
                HunkLine::Add("Y".into()),
            ],
        }];
        let err = apply_hunks_to_string(src, &hunks, Path::new("x")).unwrap_err();
        assert!(format!("{err}").contains("more than once"), "err: {err}");
    }

    #[test]
    fn apply_hunk_anchor_narrows_scope() {
        let src = "impl Foo {\n    fn a() {}\n}\nimpl Bar {\n    fn a() {}\n}\n";
        let hunks = vec![Hunk {
            anchor: Some("impl Bar {".into()),
            lines: vec![
                HunkLine::Context("    fn a() {}".into()),
                HunkLine::Add("    fn b() {}".into()),
                HunkLine::Context("}".into()),
            ],
        }];
        let out = apply_hunks_to_string(src, &hunks, Path::new("x")).unwrap();
        assert_eq!(
            out,
            "impl Foo {\n    fn a() {}\n}\nimpl Bar {\n    fn a() {}\n    fn b() {}\n}\n"
        );
    }

    #[test]
    fn apply_hunk_missing_context_errors() {
        let src = "one\ntwo\n";
        let hunks = vec![Hunk {
            anchor: None,
            lines: vec![
                HunkLine::Context("not there".into()),
                HunkLine::Remove("two".into()),
                HunkLine::Add("TWO".into()),
            ],
        }];
        let err = apply_hunks_to_string(src, &hunks, Path::new("x")).unwrap_err();
        assert!(format!("{err}").contains("did not match"), "err: {err}");
    }

    // ----- end-to-end via the Tool trait -----

    #[tokio::test]
    async fn end_to_end_add_update_delete_move() {
        let dir = TempDir::new().unwrap();
        let cwd = dir.path().to_path_buf();
        // Seed: two existing files.
        std::fs::write(cwd.join("keep.txt"), "one\ntwo\nthree\n").unwrap();
        std::fs::write(cwd.join("gone.txt"), "bye\n").unwrap();

        let patch = "\
*** Begin Patch
*** Add File: added.txt
+alpha
+beta
*** Update File: keep.txt
 one
-two
+TWO
 three
*** Delete File: gone.txt
*** End Patch
";
        let call = make_call(patch);
        let ctx = ctx(&cwd);
        let out = ApplyPatch.invoke(&call, &ctx).await.unwrap();
        assert!(!out.is_error, "invoke returned error: {}", out.content);
        assert_eq!(
            std::fs::read_to_string(cwd.join("added.txt")).unwrap(),
            "alpha\nbeta\n"
        );
        assert_eq!(
            std::fs::read_to_string(cwd.join("keep.txt")).unwrap(),
            "one\nTWO\nthree\n"
        );
        assert!(!cwd.join("gone.txt").exists());
    }

    #[tokio::test]
    async fn end_to_end_move_with_hunk() {
        let dir = TempDir::new().unwrap();
        let cwd = dir.path().to_path_buf();
        std::fs::write(cwd.join("from.txt"), "keep\nbefore\ntail\n").unwrap();

        let patch = "\
*** Begin Patch
*** Move File: from.txt -> to.txt
 keep
-before
+AFTER
 tail
*** End Patch
";
        let call = make_call(patch);
        let ctx = ctx(&cwd);
        ApplyPatch.invoke(&call, &ctx).await.unwrap();
        assert!(!cwd.join("from.txt").exists());
        assert_eq!(
            std::fs::read_to_string(cwd.join("to.txt")).unwrap(),
            "keep\nAFTER\ntail\n"
        );
    }

    #[tokio::test]
    async fn add_refuses_existing_file() {
        let dir = TempDir::new().unwrap();
        let cwd = dir.path().to_path_buf();
        std::fs::write(cwd.join("exists.txt"), "old\n").unwrap();
        let patch = "\
*** Begin Patch
*** Add File: exists.txt
+new
*** End Patch
";
        let call = make_call(patch);
        let ctx = ctx(&cwd);
        let err = ApplyPatch.invoke(&call, &ctx).await.unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("already exists"), "msg: {msg}");
    }

    fn make_call(patch: &str) -> ToolCall {
        ToolCall {
            id: mira_core::ToolCallId::from("call_test"),
            kind: mira_core::message::ToolCallKind::Function,
            function: mira_core::message::ToolCallFunction {
                name: "apply_patch".into(),
                arguments: serde_json::to_string(&serde_json::json!({ "patch": patch })).unwrap(),
            },
        }
    }
}
