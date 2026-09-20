//! Structural code search via the `ast-grep` CLI.
//!
//! Where `grep` matches lines and `find_symbol` matches definition
//! shapes, this tool matches syntax patterns.
//!
//! The `ast-grep` binary is invoked directly through the sandbox using
//! argv rather than constructing a shell command.

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
    /// The `ast-grep` pattern.
    pattern: String,

    /// Language name.
    #[serde(default)]
    language: Option<String>,

    /// Subdirectory to restrict to.
    #[serde(default)]
    path: Option<String>,

    /// Cap on returned matches.
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
             `if $C { $BODY }` finds any if-block. Requires the \
             `ast-grep` binary on PATH.",
            json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string"
                    },
                    "language": {
                        "type": "string",
                        "description": "Language hint: rust, typescript, javascript, tsx, python, go, java, c, cpp, …"
                    },
                    "path": {
                        "type": "string",
                        "description": "Restrict search to this subdirectory."
                    },
                    "max_results": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 500,
                        "default": 100
                    }
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

        if args.pattern.trim().is_empty() {
            return Err(ToolError::InvalidArgs("pattern is empty".into()));
        }

        if args.max_results == 0 {
            return Err(ToolError::InvalidArgs(
                "max_results must be greater than zero".into(),
            ));
        }

        let mut command_args = vec!["run".to_owned(), "--json=stream".to_owned()];

        if let Some(language) = &args.language {
            command_args.push("--lang".to_owned());
            command_args.push(language.clone());
        }

        command_args.push("--pattern".to_owned());
        command_args.push(args.pattern.clone());

        if let Some(path) = &args.path {
            let resolved = ctx
                .resolve(path)
                .ok_or_else(|| ToolError::Failed(format!("path escapes repository: {path}")))?;

            command_args.push(resolved.to_string_lossy().into_owned());
        } else {
            command_args.push(ctx.cwd.to_string_lossy().into_owned());
        }

        let outcome = ctx
            .sandbox
            .run_with_timeout("ast-grep", &command_args, &ctx.cwd, 45)
            .await
            .map_err(|e| ToolError::Failed(e.to_string()))?;

        if outcome.timed_out {
            return Ok(ToolResult::ok(
                call.id.clone(),
                format!(
                    "ast-grep timed out after 45 seconds for pattern `{}`",
                    args.pattern
                ),
            ));
        }

        let matches = parse_stream(&outcome.stdout, args.max_results.min(500));

        if outcome.exit_code.is_none() {
            return Err(ToolError::Failed(
                "ast-grep terminated without an exit code".into(),
            ));
        }

        let exit_code = outcome.exit_code.unwrap_or(1);

        let body = render_matches(&args.pattern, &matches, exit_code);

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
            + 1;

        let end_line = range
            .and_then(|r| r.get("end"))
            .and_then(|e| e.get("line"))
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32
            + 1;

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
        return if exit_code == 0 || exit_code == 1 {
            format!("no matches for pattern `{pattern}`")
        } else {
            format!(
                "ast-grep exited with code {exit_code} while \
                 searching for `{pattern}` — check pattern syntax \
                 and language flag"
            )
        };
    }

    let mut output = String::new();

    for m in matches {
        let snippet = m.text.replace('\n', " \u{21b5} ");

        output.push_str(&format!(
            "{}:{}-{}\t{snippet}\n",
            m.file, m.start_line, m.end_line
        ));
    }

    output
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
        assert_eq!(matches[0].start_line, 11);
        assert_eq!(matches[1].end_line, 23);
    }

    #[test]
    fn stream_parser_tolerates_junk_lines() {
        let out = concat!(
            "not json\n",
            r#"{"file":"x.rs","range":{"start":{"line":0},"end":{"line":0}},"text":"hi"}"#,
            "\n",
            "garbage\n"
        );

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

        assert!(s.contains("no matches"));
    }

    #[test]
    fn render_no_matches_error_exit_hints_at_bad_pattern() {
        let s = render_matches("$$$bad", &[], 2);

        assert!(s.contains("check pattern syntax"));
    }
}
