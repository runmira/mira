//! Structural code search via the `ast-grep` CLI.
//!
//! Where `grep` matches lines and `find_symbol` matches definition
//! shapes, this tool matches **syntax patterns**: "every call to
//! `unwrap()` inside a `match` arm", "every `for` loop that mutates a
//! collection", "every `impl Foo for Bar where …`". The pattern
//! language and language autodetection come from `ast-grep`; we're a
//! thin structured wrapper so the model gets JSON back instead of
//! having to parse text.
//!
//! We shell out to the binary rather than depending on `ast-grep-core`
//! for the same reason `grep` shells to ripgrep: the binary is fast,
//! stable, already on most developer paths, and links its own bundled
//! parsers we'd otherwise need to duplicate. When it isn't installed
//! we return a clear "install ast-grep" message rather than a shell
//! error.

use std::time::Duration;

use async_trait::async_trait;
use mira_ai::ToolSpec;
use mira_core::{ToolCall, ToolResult};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::context::ToolContext;
use crate::tool::{spec, Action, Tool, ToolError};

pub struct AstGrep;

#[derive(Deserialize)]
struct Args {
    /// The `ast-grep` pattern. Uses meta-variables like `$X` for
    /// captures — e.g. `foo($X)` matches any call to `foo` with one
    /// argument.
    pattern: String,
    /// Language name (`rust`, `typescript`, `javascript`, `tsx`,
    /// `python`, `go`, `java`, `c`, `cpp`, …). `ast-grep` can often
    /// infer from file extensions, but pinning it avoids wasted work.
    #[serde(default)]
    language: Option<String>,
    /// Subdirectory to restrict to (relative to cwd). Defaults to the
    /// whole repo.
    #[serde(default)]
    path: Option<String>,
    /// Cap on returned matches. Prevents a broad pattern from blowing
    /// the tool result window.
    #[serde(default = "default_max")]
    max_results: usize,
}

fn default_max() -> usize {
    100
}

#[async_trait]
impl Tool for AstGrep {
    fn spec(&self) -> ToolSpec {
        spec(
            "ast_grep",
            "Structural / syntactic code search. Unlike `grep`, matches \
             the parse tree — meta-variables like `$X` capture subtrees. \
             Examples: `unwrap()` finds every unwrap call; \
             `Result<$T, $E>` finds every Result type; \
             `if $C { $BODY }` finds any if-block. \
             Requires the `ast-grep` binary on PATH (install: \
             `cargo install ast-grep` or `brew install ast-grep`).",
            json!({
                "type": "object",
                "properties": {
                    "pattern":     { "type": "string" },
                    "language":    { "type": "string", "description": "Language hint: rust, typescript, javascript, tsx, python, go, java, c, cpp, …" },
                    "path":        { "type": "string", "description": "Restrict search to this subdirectory." },
                    "max_results": { "type": "integer", "minimum": 1, "maximum": 500, "default": 100 }
                },
                "required": ["pattern"],
                "additionalProperties": false
            }),
        )
    }

    fn action(&self) -> Action {
        Action::Read
    }

    async fn invoke(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let args: Args = call.parse_arguments()?;

        // Build the command line. `--json=stream` emits one JSON object
        // per line — cheaper to parse defensively than the pretty
        // format, and won't blow memory on huge match sets.
        let mut cmd = String::from("ast-grep run --json=stream");
        if let Some(lang) = &args.language {
            cmd.push_str(" --lang ");
            cmd.push_str(&shell_quote(lang));
        }
        cmd.push_str(" --pattern ");
        cmd.push_str(&shell_quote(&args.pattern));
        if let Some(p) = &args.path {
            let resolved = ctx
                .resolve(p)
                .ok_or_else(|| ToolError::Failed(format!("path escapes cwd: {p}")))?;
            cmd.push(' ');
            cmd.push_str(&shell_quote(&resolved.to_string_lossy()));
        }

        let outcome = ctx
            .sandbox
            .run(&cmd, &ctx.cwd, Duration::from_secs(45))
            .await
            .map_err(|e| ToolError::Failed(e.to_string()))?;

        // `ast-grep` returns non-zero on `command not found` via bash —
        // check for that signature specifically so we give the model
        // an actionable message instead of a generic "exit 127".
        if outcome.exit_code == 127 || outcome.output.contains("ast-grep: command not found") {
            return Ok(ToolResult::ok(
                call.id.clone(),
                "ast-grep binary not found on PATH. Install with `cargo install ast-grep` \
                 or `brew install ast-grep`, then retry."
                    .to_owned(),
            ));
        }

        let matches = parse_stream(&outcome.output, args.max_results);
        let body = render_matches(&args.pattern, &matches, outcome.exit_code);
        let data = json!({
            "pattern": args.pattern,
            "language": args.language,
            "total_matches": matches.len(),
            "truncated": matches.len() >= args.max_results,
            "matches": matches,
        });
        Ok(ToolResult {
            call_id: call.id.clone(),
            content: body,
            is_error: false,
            data: Some(data),
        })
    }
}

#[derive(serde::Serialize, Debug, Clone)]
struct AstMatch {
    file: String,
    start_line: u32,
    end_line: u32,
    text: String,
}

/// Parse `ast-grep --json=stream` output. Tolerant: skips malformed
/// lines (a garbled emit shouldn't kill the run). Returns at most
/// `max_results` matches — we stop early once the cap is hit.
fn parse_stream(output: &str, max_results: usize) -> Vec<AstMatch> {
    let mut out = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() || !line.starts_with('{') {
            continue;
        }
        let Ok(v): Result<Value, _> = serde_json::from_str(line) else {
            continue;
        };
        let file = v
            .get("file")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let range = v.get("range");
        let start_line = range
            .and_then(|r| r.get("start"))
            .and_then(|s| s.get("line"))
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32
            + 1; // ast-grep uses 0-based line numbers.
        let end_line = range
            .and_then(|r| r.get("end"))
            .and_then(|e| e.get("line"))
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32
            + 1;
        // Prefer the `text` field (single-line summary); fall back to
        // `lines` (multi-line block) if present.
        let text = v
            .get("text")
            .and_then(Value::as_str)
            .or_else(|| v.get("lines").and_then(Value::as_str))
            .unwrap_or_default()
            .to_owned();
        out.push(AstMatch {
            file,
            start_line,
            end_line,
            text,
        });
        if out.len() >= max_results {
            break;
        }
    }
    out
}

fn render_matches(pattern: &str, matches: &[AstMatch], exit_code: i32) -> String {
    if matches.is_empty() {
        // Non-zero exit with no matches is the "not found" case in
        // ast-grep; anything else usually means a bad pattern.
        return if exit_code == 0 || exit_code == 1 {
            format!("no matches for pattern `{pattern}`")
        } else {
            format!(
                "no matches for pattern `{pattern}` (ast-grep exit {exit_code}) — \
                 check pattern syntax and language flag"
            )
        };
    }
    let mut s = String::new();
    for m in matches {
        // file:start-end\tsnippet — same shape as `grep` for
        // consistency, plus the range so the model doesn't have to
        // re-read to know where the match ends.
        let snippet = m.text.replace('\n', " \u{21b5} ");
        s.push_str(&format!(
            "{}:{}-{}\t{snippet}\n",
            m.file, m.start_line, m.end_line
        ));
    }
    s
}

fn shell_quote(s: &str) -> String {
    let escaped = s.replace('\'', "'\\''");
    format!("'{escaped}'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_parser_basic() {
        let out = concat!(
            r#"{"file":"src/a.rs","range":{"start":{"line":10},"end":{"line":10}},"text":"unwrap()"}"#,
            "\n",
            r#"{"file":"src/b.rs","range":{"start":{"line":20},"end":{"line":22}},"text":"expect(\"x\")"}"#,
            "\n"
        );
        let matches = parse_stream(out, 100);
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].file, "src/a.rs");
        assert_eq!(matches[0].start_line, 11); // 0-based → 1-based
        assert_eq!(matches[1].end_line, 23);
    }

    #[test]
    fn stream_parser_tolerates_junk_lines() {
        let out = "not json\n{\"file\":\"x.rs\",\"range\":{\"start\":{\"line\":0},\"end\":{\"line\":0}},\"text\":\"hi\"}\ngarbage\n";
        let matches = parse_stream(out, 100);
        assert_eq!(matches.len(), 1);
    }

    #[test]
    fn stream_parser_respects_cap() {
        let mut out = String::new();
        for i in 0..50 {
            out.push_str(&format!(
                r#"{{"file":"x.rs","range":{{"start":{{"line":{i}}},"end":{{"line":{i}}}}},"text":"m"}}"#
            ));
            out.push('\n');
        }
        assert_eq!(parse_stream(&out, 10).len(), 10);
    }

    #[test]
    fn render_no_matches_normal_exit() {
        let s = render_matches("foo()", &[], 1);
        assert!(s.contains("no matches"), "s: {s}");
    }

    #[test]
    fn render_no_matches_error_exit_hints_at_bad_pattern() {
        let s = render_matches("$$$bad", &[], 2);
        assert!(s.contains("check pattern syntax"), "s: {s}");
    }
}
