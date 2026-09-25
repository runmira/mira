//! Hooks in Claude Code's format, from enabled plugins and `hooks:` in
//! `~/.mira/mira.yaml`.
//!
//! ```json
//! {"PreToolUse": [{"matcher": "Edit|Write", "hooks": [
//!     {"type": "command", "command": "python3 ${CLAUDE_PLUGIN_ROOT}/check.py", "timeout": 10}
//! ]}]}
//! ```
//!
//! Each matching command runs with the event as JSON on stdin (in
//! parallel with the event's other hooks). It answers the way Claude
//! Code's hooks do:
//!
//! - exit 0: success. Stdout may be JSON (`decision`, `reason`,
//!   `systemMessage`, `hookSpecificOutput.{permissionDecision,
//!   permissionDecisionReason, additionalContext, updatedInput}`); plain
//!   stdout from `SessionStart` / `UserPromptSubmit` is added as context.
//! - exit 2: block. Stderr is the reason, shown to the model (for a
//!   prompt, to the user).
//! - anything else: a non-blocking error, shown to the user.
//!
//! Tool events use Claude Code's tool names (`Bash`, `Read`, `Edit`,
//! `Write`, …) and add `file_path` beside Mira's `path`, so hooks written
//! for Claude Code work unchanged; matchers match either name.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use regex::Regex;
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;

/// Events Mira runs. Others in a config are kept but never fire.
pub const EVENTS: [&str; 5] = [
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "Stop",
];

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone, Debug)]
pub struct Hook {
    pub event: String,
    /// `None` matches everything.
    pub matcher: Option<Regex>,
    pub command: String,
    pub timeout: Duration,
    /// Plugin directory, for `${CLAUDE_PLUGIN_ROOT}`.
    pub plugin_root: Option<PathBuf>,
    /// Where it came from, for messages: a plugin id or `mira.yaml`.
    pub source: String,
}

/// Every configured hook.
#[derive(Clone, Debug, Default)]
pub struct HookSet {
    pub hooks: Vec<Hook>,
    pub problems: Vec<String>,
}

/// A permission decision from a PreToolUse hook.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Permission {
    Allow,
    Deny(String),
    Ask,
}

/// The combined answer of an event's hooks.
#[derive(Clone, Debug, Default)]
pub struct Outcome {
    pub block: Option<String>,
    pub permission: Option<Permission>,
    pub updated_input: Option<Value>,
    pub context: Vec<String>,
    pub messages: Vec<String>,
}

impl HookSet {
    /// Add the hooks in `config` (`{event: [{matcher, hooks: [...]}]}`,
    /// or wrapped in `{"hooks": …}`).
    pub fn add(&mut self, config: &Value, plugin_root: Option<&Path>, source: &str) {
        let events = config.get("hooks").unwrap_or(config);
        let Some(events) = events.as_object() else {
            self.problems
                .push(format!("{source}: hooks must be an object of events"));
            return;
        };
        for (event, groups) in events {
            let Some(groups) = groups.as_array() else {
                self.problems
                    .push(format!("{source}: `{event}` must be a list"));
                continue;
            };
            for group in groups {
                let matcher = match group.get("matcher").and_then(Value::as_str).map(str::trim) {
                    None | Some("") | Some("*") => None,
                    Some(m) => match Regex::new(&format!("^(?:{m})$")) {
                        Ok(r) => Some(r),
                        Err(e) => {
                            self.problems
                                .push(format!("{source}: `{event}` matcher `{m}`: {e}"));
                            continue;
                        }
                    },
                };
                for h in group
                    .get("hooks")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let kind = h.get("type").and_then(Value::as_str).unwrap_or("command");
                    if kind != "command" {
                        self.problems.push(format!(
                            "{source}: `{event}` hook type `{kind}` isn't supported (only `command`)"
                        ));
                        continue;
                    }
                    let Some(command) = h.get("command").and_then(Value::as_str) else {
                        self.problems
                            .push(format!("{source}: a `{event}` hook has no `command`"));
                        continue;
                    };
                    let timeout = h
                        .get("timeout")
                        .and_then(Value::as_f64)
                        .filter(|t| *t > 0.0)
                        .map(Duration::from_secs_f64)
                        .unwrap_or(DEFAULT_TIMEOUT);
                    self.hooks.push(Hook {
                        event: event.clone(),
                        matcher: matcher.clone(),
                        command: command.to_owned(),
                        timeout,
                        plugin_root: plugin_root.map(Path::to_path_buf),
                        source: source.to_owned(),
                    });
                }
            }
        }
    }

    pub fn has(&self, event: &str) -> bool {
        self.hooks.iter().any(|h| h.event == event)
    }

    /// Run `event`'s hooks matching `target` (a tool name or the
    /// SessionStart source) with `input`.
    pub async fn run(&self, event: &str, target: &str, mut input: Value) -> Outcome {
        let tool_event = event == "PreToolUse" || event == "PostToolUse";
        let cc_name = if tool_event {
            claude_tool_name(target)
        } else {
            target.to_owned()
        };
        let matching: Vec<&Hook> = self
            .hooks
            .iter()
            .filter(|h| h.event == event)
            .filter(|h| {
                h.matcher
                    .as_ref()
                    .is_none_or(|m| m.is_match(&cc_name) || m.is_match(target))
            })
            .collect();
        if matching.is_empty() {
            return Outcome::default();
        }
        let original_input = input.get("tool_input").cloned();
        if let Some(obj) = input.as_object_mut() {
            obj.insert("hook_event_name".into(), json!(event));
            obj.insert("transcript_path".into(), json!(""));
            if tool_event {
                obj.insert("tool_name".into(), json!(cc_name));
                obj.insert("mira_tool_name".into(), json!(target));
                if let Some(ti) = obj.get_mut("tool_input") {
                    alias_paths(ti);
                }
            }
        }
        let cwd = input.get("cwd").and_then(Value::as_str).map(PathBuf::from);
        let stdin = input.to_string();
        let runs = matching.iter().map(|h| run_one(h, &stdin, cwd.as_deref()));
        let results = futures::future::join_all(runs).await;

        let mut out = Outcome::default();
        for (hook, result) in matching.iter().zip(results) {
            merge(&mut out, hook, event, result);
        }
        if let (Some(updated), Some(original)) = (out.updated_input.as_mut(), original_input) {
            unalias_paths(updated, &original);
        }
        out
    }
}

/// What one command did.
enum RunResult {
    Exited {
        code: i32,
        stdout: String,
        stderr: String,
    },
    Failed(String),
}

async fn run_one(hook: &Hook, stdin: &str, cwd: Option<&Path>) -> RunResult {
    let mut cmd = if cfg!(windows) {
        let mut c = tokio::process::Command::new("cmd");
        c.args(["/C", &hook.command]);
        c
    } else {
        let mut c = tokio::process::Command::new("sh");
        c.args(["-c", &hook.command]);
        c
    };
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
        cmd.env("CLAUDE_PROJECT_DIR", dir)
            .env("MIRA_PROJECT_DIR", dir);
    }
    if let Some(root) = &hook.plugin_root {
        cmd.env("CLAUDE_PLUGIN_ROOT", root)
            .env("MIRA_PLUGIN_ROOT", root);
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return RunResult::Failed(format!("couldn't start: {e}")),
    };
    if let Some(mut pipe) = child.stdin.take() {
        // A hook that doesn't read stdin closes it early; that's fine.
        let _ = pipe.write_all(stdin.as_bytes()).await;
        drop(pipe);
    }
    match tokio::time::timeout(hook.timeout, child.wait_with_output()).await {
        Err(_) => RunResult::Failed(format!("timed out after {}s", hook.timeout.as_secs())),
        Ok(Err(e)) => RunResult::Failed(e.to_string()),
        Ok(Ok(o)) => RunResult::Exited {
            code: o.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&o.stdout).trim().to_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).trim().to_owned(),
        },
    }
}

fn merge(out: &mut Outcome, hook: &Hook, event: &str, result: RunResult) {
    let label = format!("{} hook ({})", event, hook.source);
    let (code, stdout, stderr) = match result {
        RunResult::Failed(e) => {
            out.messages.push(format!("{label} {e}"));
            return;
        }
        RunResult::Exited {
            code,
            stdout,
            stderr,
        } => (code, stdout, stderr),
    };
    if code == 2 {
        let reason = if stderr.is_empty() {
            "blocked".to_owned()
        } else {
            stderr
        };
        add_block(out, reason);
        return;
    }
    if code != 0 {
        let detail = if stderr.is_empty() { stdout } else { stderr };
        out.messages.push(format!(
            "{label} failed (exit {code}): {}",
            first_lines(&detail)
        ));
        return;
    }
    let json: Option<Value> = stdout
        .starts_with('{')
        .then(|| serde_json::from_str(&stdout).ok())
        .flatten();
    let Some(json) = json else {
        if !stdout.is_empty() && (event == "SessionStart" || event == "UserPromptSubmit") {
            out.context.push(stdout);
        }
        return;
    };
    if let Some(m) = json.get("systemMessage").and_then(Value::as_str) {
        out.messages.push(m.to_owned());
    }
    let reason = json
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    match json.get("decision").and_then(Value::as_str) {
        Some("block") => add_block(
            out,
            if reason.is_empty() {
                "blocked".into()
            } else {
                reason.clone()
            },
        ),
        // Older PreToolUse form.
        Some("approve") if out.permission.is_none() => out.permission = Some(Permission::Allow),
        _ => {}
    }
    if event == "UserPromptSubmit" && json.get("continue") == Some(&Value::Bool(false)) {
        let why = json
            .get("stopReason")
            .and_then(Value::as_str)
            .unwrap_or("stopped by a hook");
        add_block(out, why.to_owned());
    }
    if let Some(specific) = json.get("hookSpecificOutput") {
        if let Some(ctx) = specific.get("additionalContext").and_then(Value::as_str) {
            if !ctx.trim().is_empty() {
                out.context.push(ctx.to_owned());
            }
        }
        if let Some(updated) = specific.get("updatedInput") {
            if updated.is_object() {
                out.updated_input = Some(updated.clone());
            }
        }
        let why = specific
            .get("permissionDecisionReason")
            .and_then(Value::as_str)
            .unwrap_or(&reason)
            .to_owned();
        let decision = match specific.get("permissionDecision").and_then(Value::as_str) {
            Some("deny") => Some(Permission::Deny(if why.is_empty() {
                "denied".into()
            } else {
                why
            })),
            Some("ask") => Some(Permission::Ask),
            Some("allow") => Some(Permission::Allow),
            _ => None,
        };
        if let Some(d) = decision {
            out.permission = Some(strongest(out.permission.take(), d));
        }
    }
}

fn add_block(out: &mut Outcome, reason: String) {
    out.block = Some(match out.block.take() {
        Some(prev) => format!("{prev}\n{reason}"),
        None => reason,
    });
}

/// Deny beats ask beats allow.
fn strongest(a: Option<Permission>, b: Permission) -> Permission {
    let rank = |p: &Permission| match p {
        Permission::Deny(_) => 2,
        Permission::Ask => 1,
        Permission::Allow => 0,
    };
    match a {
        Some(a) if rank(&a) >= rank(&b) => a,
        _ => b,
    }
}

fn first_lines(s: &str) -> String {
    s.lines().take(3).collect::<Vec<_>>().join(" / ")
}

/// Mira tool name → Claude Code's, for matchers and hook input.
pub fn claude_tool_name(mira: &str) -> String {
    match mira {
        "bash" => "Bash",
        "read_file" => "Read",
        "write_file" => "Write",
        "edit_file" => "Edit",
        "apply_patch" => "MultiEdit",
        "glob" => "Glob",
        "grep" => "Grep",
        "web_fetch" => "WebFetch",
        "web_search" => "WebSearch",
        "agent" => "Task",
        "skill" => "Skill",
        "task_create" | "task_update" | "task_list" | "task_get" => "TodoWrite",
        other => return other.to_owned(),
    }
    .to_owned()
}

/// Add `file_path` beside `path`, as Claude Code's tools call it.
fn alias_paths(input: &mut Value) {
    if let Some(obj) = input.as_object_mut() {
        if let Some(p) = obj.get("path").cloned() {
            obj.entry("file_path").or_insert(p);
        }
    }
}

/// Map a hook's `updatedInput` back to Mira's argument names.
fn unalias_paths(updated: &mut Value, original: &Value) {
    let had_path = original.get("path").is_some();
    if let Some(obj) = updated.as_object_mut() {
        if had_path {
            if let Some(fp) = obj.remove("file_path") {
                obj.insert("path".into(), fp);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(config: Value) -> HookSet {
        let mut s = HookSet::default();
        s.add(&config, None, "test");
        assert!(s.problems.is_empty(), "{:?}", s.problems);
        s
    }

    fn input(tool: &str, args: Value) -> Value {
        json!({"session_id": "s", "cwd": std::env::temp_dir(), "tool_name": tool, "tool_input": args})
    }

    #[tokio::test]
    async fn exit_codes_and_matchers() {
        let s = set(json!({"PreToolUse": [
            {"matcher": "Bash", "hooks": [{"type": "command", "command": "echo 'no rm' >&2; exit 2"}]},
            {"matcher": "Edit|Write", "hooks": [{"type": "command", "command": "exit 0"}]}
        ]}));
        let out = s
            .run(
                "PreToolUse",
                "bash",
                input("bash", json!({"command": "rm -rf /"})),
            )
            .await;
        assert_eq!(out.block.as_deref(), Some("no rm"));
        let out = s
            .run("PreToolUse", "read_file", input("read_file", json!({})))
            .await;
        assert!(out.block.is_none());
    }

    #[tokio::test]
    async fn json_output_and_claude_code_input() {
        // The hook sees Claude Code's names and answers with JSON.
        let s = set(
            json!({"PreToolUse": [{"matcher": "Write", "hooks": [{"type": "command", "command":
                "python3 -c \"import json,sys; d=json.load(sys.stdin); ok = d['tool_name']=='Write' and d['tool_input']['file_path']=='a.txt'; print(json.dumps({'systemMessage': 'checked', 'hookSpecificOutput': {'hookEventName': 'PreToolUse', 'permissionDecision': 'deny' if ok else 'allow', 'permissionDecisionReason': 'no writes', 'updatedInput': {'file_path': 'b.txt', 'content': 'x'}}}))\""
            }]}]}),
        );
        let out = s
            .run(
                "PreToolUse",
                "write_file",
                input("write_file", json!({"path": "a.txt", "content": "x"})),
            )
            .await;
        assert_eq!(out.permission, Some(Permission::Deny("no writes".into())));
        assert_eq!(out.messages, ["checked"]);
        // updatedInput maps back to Mira's `path`.
        assert_eq!(out.updated_input.unwrap()["path"], "b.txt");
    }

    #[tokio::test]
    async fn context_errors_and_timeouts() {
        let s = set(json!({
            "UserPromptSubmit": [{"hooks": [
                {"type": "command", "command": "echo 'today is friday'"},
                {"type": "command", "command": "echo oops >&2; exit 1"},
                {"type": "command", "command": "sleep 5", "timeout": 0.2}
            ]}],
            "SessionStart": [{"matcher": "resume", "hooks": [{"type": "command", "command": "echo resumed"}]}]
        }));
        let out = s
            .run(
                "UserPromptSubmit",
                "",
                json!({"prompt": "hi", "cwd": std::env::temp_dir()}),
            )
            .await;
        assert_eq!(out.context, ["today is friday"]);
        assert!(out.block.is_none());
        assert_eq!(out.messages.len(), 2, "{:?}", out.messages);
        assert!(out
            .messages
            .iter()
            .any(|m| m.contains("exit 1") && m.contains("oops")));
        assert!(out.messages.iter().any(|m| m.contains("timed out")));
        // SessionStart matchers match the source.
        let out = s
            .run(
                "SessionStart",
                "startup",
                json!({"cwd": std::env::temp_dir()}),
            )
            .await;
        assert!(out.context.is_empty());
    }

    #[test]
    fn bad_matchers_and_types_are_reported() {
        let mut s = HookSet::default();
        s.add(
            &json!({"PreToolUse": [
                {"matcher": "(", "hooks": [{"type": "command", "command": "true"}]},
                {"hooks": [{"type": "prompt", "prompt": "x"}]}
            ]}),
            None,
            "p",
        );
        assert!(s.hooks.is_empty());
        assert_eq!(s.problems.len(), 2);
    }
}
