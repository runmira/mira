//! Tool implementations for sandboxed sessions.
//!
//! When [`ToolContext::compute`] is set (`mira --sandbox …`), the routed
//! built-ins (`bash`, `read_file`, `write_file`, `edit_file`, `grep`,
//! `glob`) hand off here instead of touching the local filesystem. Same
//! arguments, same output shapes as the local versions, so the model
//! can't tell the difference beyond the paths.
//!
//! Tools that aren't routed are withheld from sandboxed sessions entirely
//! (see [`crate::Tool::remote_capable`]), so nothing silently falls back
//! to the local machine.

use std::sync::Arc;
use std::time::Duration;

use glob::{MatchOptions, Pattern};
use mira_compute::{ComputeBackend, ComputeError, ComputeEvent, ExecRequest};
use mira_core::{ToolCall, ToolResult};
use serde::Deserialize;
use tokio::sync::mpsc;

use crate::context::ToolContext;
use crate::tool::ToolError;

fn err(e: ComputeError) -> ToolError {
    ToolError::Failed(e.to_string())
}

/// The session's working directory, relative to the workspace root.
fn cwd_rel(ctx: &ToolContext) -> String {
    ctx.cwd
        .strip_prefix(&ctx.repo_root)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Map a tool path onto the workspace (see `mira_compute::path`).
fn rel(ctx: &ToolContext, backend: &dyn ComputeBackend, path: &str) -> Result<String, ToolError> {
    let expanded = shellexpand::tilde(path);
    mira_compute::path::resolve(
        &expanded,
        &ctx.repo_root,
        &backend.workspace_root(),
        &cwd_rel(ctx),
    )
    .map_err(err)
}

/// `'…'`-quote for a POSIX shell.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Run `command` in the workspace, streaming output to the tool tail and
/// stopping early if the turn is cancelled.
async fn exec(
    ctx: &ToolContext,
    backend: &Arc<dyn ComputeBackend>,
    call: &ToolCall,
    req: ExecRequest,
) -> Result<mira_compute::ExecOutput, ToolError> {
    let (tx, mut rx) = mpsc::channel::<ComputeEvent>(256);
    let progress = ctx.progress.clone();
    let call_id = call.id.to_string();
    let forward = tokio::spawn(async move {
        while let Some(ev) = rx.recv().await {
            if let Some(p) = &progress {
                let (ComputeEvent::Stdout(line) | ComputeEvent::Stderr(line)) = ev;
                p.emit(&call_id, &line);
            }
        }
    });
    let run = backend.exec(req, Some(tx));
    let out = match &ctx.cancel {
        Some(token) => tokio::select! {
            out = run => out,
            _ = token.cancelled() => {
                forward.abort();
                return Err(ToolError::Failed("cancelled".into()));
            }
        },
        None => run.await,
    };
    let _ = forward.await;
    out.map_err(err)
}

#[derive(Deserialize)]
struct BashArgs {
    command: String,
    #[serde(default = "default_timeout_ms")]
    timeout_ms: u64,
}

fn default_timeout_ms() -> u64 {
    300_000
}

pub async fn bash(
    backend: &Arc<dyn ComputeBackend>,
    call: &ToolCall,
    ctx: &ToolContext,
) -> Result<ToolResult, ToolError> {
    let args: BashArgs = call.parse_arguments()?;
    let req = ExecRequest::new(args.command)
        .cwd(cwd_rel(ctx))
        .timeout(Duration::from_millis(args.timeout_ms.clamp(100, 600_000)));
    let out = exec(ctx, backend, call, req).await?;

    let mut output = out.stdout;
    if !out.stderr.is_empty() {
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(&out.stderr);
    }
    let mut body = match out.exit_code {
        Some(code) => format!("exit={code}\n"),
        None => "exit=unknown\n".to_owned(),
    };
    if out.timed_out {
        body.push_str("(command timed out)\n");
    }
    body.push_str("--- output ---\n");
    body.push_str(&truncate(&output, 32_000));
    Ok(ToolResult::ok(call.id.clone(), body))
}

fn truncate(s: &str, limit: usize) -> String {
    if s.len() <= limit {
        return s.to_owned();
    }
    let mut head = limit / 2;
    while !s.is_char_boundary(head) {
        head -= 1;
    }
    let mut tail = s.len() - limit / 2;
    while !s.is_char_boundary(tail) {
        tail += 1;
    }
    format!(
        "{}\n... [truncated {} bytes] ...\n{}",
        &s[..head],
        s.len() - limit,
        &s[tail..]
    )
}

#[derive(Deserialize)]
struct ReadArgs {
    path: String,
    #[serde(default)]
    start_line: Option<usize>,
    #[serde(default)]
    end_line: Option<usize>,
}

/// Same cap as the local `read_file`.
const MAX_READ_BYTES: usize = 512 * 1024;

pub async fn read_file(
    backend: &Arc<dyn ComputeBackend>,
    call: &ToolCall,
    ctx: &ToolContext,
) -> Result<ToolResult, ToolError> {
    let args: ReadArgs = call.parse_arguments()?;
    let path = rel(ctx, backend.as_ref(), &args.path)?;
    let bytes = backend.read_file(&path).await.map_err(err)?;
    if bytes.len() > MAX_READ_BYTES {
        return Err(ToolError::Failed(format!(
            "file too large ({} bytes; limit {MAX_READ_BYTES}). Read a range with start_line/end_line.",
            bytes.len()
        )));
    }
    let raw = String::from_utf8_lossy(&bytes);
    let start = args.start_line.unwrap_or(1).saturating_sub(1);
    let end = args.end_line.unwrap_or(usize::MAX);
    let mut out = String::with_capacity(raw.len() + raw.lines().count() * 6);
    for (i, line) in raw.lines().enumerate() {
        let n = i + 1;
        if n <= start {
            continue;
        }
        if n > end {
            break;
        }
        out.push_str(&format!("{n:>6}\t{line}\n"));
    }
    Ok(ToolResult::ok(call.id.clone(), out))
}

#[derive(Deserialize)]
struct WriteArgs {
    path: String,
    content: String,
}

pub async fn write_file(
    backend: &Arc<dyn ComputeBackend>,
    call: &ToolCall,
    ctx: &ToolContext,
) -> Result<ToolResult, ToolError> {
    let args: WriteArgs = call.parse_arguments()?;
    let path = rel(ctx, backend.as_ref(), &args.path)?;
    backend
        .write_file(&path, args.content.as_bytes())
        .await
        .map_err(err)?;
    Ok(ToolResult::ok(
        call.id.clone(),
        format!("wrote {} bytes to {path}", args.content.len()),
    ))
}

#[derive(Deserialize)]
struct EditArgs {
    path: String,
    old_string: String,
    new_string: String,
    #[serde(default)]
    replace_all: bool,
}

pub async fn edit_file(
    backend: &Arc<dyn ComputeBackend>,
    call: &ToolCall,
    ctx: &ToolContext,
) -> Result<ToolResult, ToolError> {
    let args: EditArgs = call.parse_arguments()?;
    if args.old_string == args.new_string {
        return Err(ToolError::InvalidArgs(
            "old_string and new_string are identical".into(),
        ));
    }
    let path = rel(ctx, backend.as_ref(), &args.path)?;
    let bytes = backend.read_file(&path).await.map_err(err)?;
    let contents = String::from_utf8(bytes)
        .map_err(|_| ToolError::Failed(format!("{path} is not UTF-8 text")))?;
    let count = contents.matches(&args.old_string).count();
    let updated = match (count, args.replace_all) {
        (0, _) => return Err(ToolError::Failed(format!("old_string not found in {path}"))),
        (n, false) if n > 1 => {
            return Err(ToolError::Failed(format!(
                "old_string matches {n} times; pass replace_all=true or add context to disambiguate"
            )))
        }
        (_, true) => contents.replace(&args.old_string, &args.new_string),
        (_, false) => contents.replacen(&args.old_string, &args.new_string, 1),
    };
    backend
        .write_file(&path, updated.as_bytes())
        .await
        .map_err(err)?;
    Ok(ToolResult::ok(
        call.id.clone(),
        format!(
            "edited {path} ({count} replacement{})",
            if count == 1 { "" } else { "s" }
        ),
    ))
}

#[derive(Deserialize)]
struct GrepArgs {
    pattern: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    glob: Option<String>,
    #[serde(default)]
    case_insensitive: bool,
    #[serde(default = "default_max_results")]
    max_results: usize,
}

fn default_max_results() -> usize {
    200
}

/// Ripgrep in the sandbox, or `grep -rn` when the image has no `rg`.
pub async fn grep(
    backend: &Arc<dyn ComputeBackend>,
    call: &ToolCall,
    ctx: &ToolContext,
) -> Result<ToolResult, ToolError> {
    let args: GrepArgs = call.parse_arguments()?;
    if args.max_results == 0 {
        return Err(ToolError::Failed(
            "max_results must be greater than zero".into(),
        ));
    }
    let max = args.max_results.min(2000);
    let target = match &args.path {
        Some(p) => rel(ctx, backend.as_ref(), p)?,
        None => cwd_rel(ctx),
    };
    let target = if target.is_empty() {
        ".".to_owned()
    } else {
        target
    };
    let pat = sh_quote(&args.pattern);
    let tgt = sh_quote(&target);
    let ci = if args.case_insensitive { " -i" } else { "" };
    let rg_glob = args
        .glob
        .as_deref()
        .map(|g| format!(" --glob {}", sh_quote(g)))
        .unwrap_or_default();
    let grep_glob = args
        .glob
        .as_deref()
        .map(|g| format!(" --include={}", sh_quote(g)))
        .unwrap_or_default();
    let command = format!(
        "if command -v rg >/dev/null 2>&1; then \
           rg --line-number --no-heading --color never --max-count {max}{ci}{rg_glob} -e {pat} -- {tgt}; \
         else \
           grep -rnE{ci} --exclude-dir=.git{grep_glob} -m {max} -e {pat} -- {tgt}; \
         fi"
    );
    let out = exec(
        ctx,
        backend,
        call,
        ExecRequest::new(command).timeout(Duration::from_secs(30)),
    )
    .await?;
    if out.timed_out {
        return Ok(ToolResult::ok(
            call.id.clone(),
            format!(
                "search timed out after 30 seconds for /{}/\n{}",
                args.pattern, out.stdout
            ),
        ));
    }
    match out.exit_code {
        Some(0) => Ok(ToolResult::ok(call.id.clone(), out.stdout)),
        Some(1) => Ok(ToolResult::ok(
            call.id.clone(),
            format!("no matches for /{}/", args.pattern),
        )),
        code => Err(ToolError::Failed(format!(
            "search failed (exit {code:?}): {}",
            out.stderr.trim()
        ))),
    }
}

#[derive(Deserialize)]
struct GlobArgs {
    pattern: String,
    #[serde(default = "default_true")]
    case_sensitive: bool,
    #[serde(default)]
    include_hidden: bool,
    #[serde(default)]
    include_ignored: bool,
    #[serde(default = "default_glob_max")]
    max_results: usize,
}

fn default_true() -> bool {
    true
}

fn default_glob_max() -> usize {
    500
}

/// List files in the sandbox (git-aware), then match locally.
pub async fn glob(
    backend: &Arc<dyn ComputeBackend>,
    call: &ToolCall,
    ctx: &ToolContext,
) -> Result<ToolResult, ToolError> {
    let args: GlobArgs = call.parse_arguments()?;
    let cwd = cwd_rel(ctx);
    // Absolute patterns under the workspace become cwd-relative.
    let pattern = {
        let expanded = shellexpand::tilde(&args.pattern).into_owned();
        let local = ctx.cwd.to_string_lossy().into_owned();
        let remote = mira_compute::path::join(&backend.workspace_root(), &cwd);
        expanded
            .strip_prefix(&format!("{local}/"))
            .or_else(|| expanded.strip_prefix(&format!("{remote}/")))
            .map(str::to_owned)
            .unwrap_or(expanded)
    };
    if pattern.starts_with('/') {
        return Err(ToolError::Failed(format!(
            "{pattern} is outside the workspace; use a pattern relative to it"
        )));
    }
    let matcher = Pattern::new(&pattern).map_err(|e| ToolError::InvalidArgs(e.to_string()))?;
    let options = MatchOptions {
        case_sensitive: args.case_sensitive,
        require_literal_separator: false,
        require_literal_leading_dot: !args.include_hidden,
    };
    let list = if args.include_ignored {
        "find . -type f -not -path './.git/*' | sed 's|^\\./||'"
    } else {
        "git ls-files --cached --others --exclude-standard"
    };
    let out = exec(
        ctx,
        backend,
        call,
        ExecRequest::new(list)
            .cwd(cwd)
            .timeout(Duration::from_secs(30)),
    )
    .await?;
    if out.exit_code != Some(0) {
        return Err(ToolError::Failed(format!(
            "listing files failed: {}",
            out.stderr.trim()
        )));
    }
    let mut matches: Vec<&str> = out
        .stdout
        .lines()
        .filter(|f| matcher.matches_with(f, options))
        .collect();
    matches.sort_unstable();
    let limit = args.max_results.max(1);
    let hit = matches.len() > limit;
    matches.truncate(limit);
    let body = if matches.is_empty() {
        format!("no matches for `{}`", args.pattern)
    } else {
        let mut s = matches.join("\n");
        s.push('\n');
        if hit {
            s.push_str(&format!("(truncated at {limit} results)\n"));
        }
        s
    };
    Ok(ToolResult::ok(call.id.clone(), body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_survives_single_quotes() {
        assert_eq!(sh_quote("it's"), r"'it'\''s'");
    }
}
