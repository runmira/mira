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
//! `{"type": "prompt", "prompt": "…"}` hooks ask a model instead: it gets
//! the prompt (with `$ARGUMENTS` replaced by the event's JSON input) and
//! answers `{"ok": true}` or `{"ok": false, "reason": "…"}`. `ok: false`
//! blocks, like exit 2. They work on `Stop`, `SubagentStop`,
//! `UserPromptSubmit` and `PreToolUse`; a model error lets things through.
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
pub const EVENTS: [&str; 9] = [
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "Stop",
    "SubagentStop",
    "Notification",
    "PreCompact",
    "SessionEnd",
];

/// Events where a `prompt` hook's yes/no means something.
pub const PROMPT_EVENTS: [&str; 4] = ["Stop", "SubagentStop", "UserPromptSubmit", "PreToolUse"];

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
const DEFAULT_PROMPT_TIMEOUT: Duration = Duration::from_secs(30);

/// Asks a model for a `prompt` hook's verdict. Mira passes one backed by
/// the user's provider (and `small_model`); without one, prompt hooks
/// are skipped with a message.
#[async_trait::async_trait]
pub trait PromptEvaluator: Send + Sync {
    /// The model's reply text to `user`, under `system`.
    async fn evaluate(&self, system: &str, user: &str) -> Result<String, String>;
}

/// What a hook does.
#[derive(Clone, Debug)]
pub enum HookAction {
    /// A shell command, given the event as JSON on stdin.
    Command(String),
    /// A prompt for a model to judge.
    Prompt(String),
}

#[derive(Clone, Debug)]
pub struct Hook {
    pub event: String,
    /// `None` matches everything.
    pub matcher: Option<Regex>,
    pub action: HookAction,
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
    /// Hooks that failed while running, with their latest error. Each is
    /// reported to the user once; after that it's only listed here, so a
    /// broken plugin hook doesn't add a warning to every message.
    failing: std::sync::Arc<std::sync::Mutex<std::collections::BTreeMap<String, String>>>,
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
                    let (action, default_timeout) = match kind {
                        "command" => {
                            let Some(command) = h.get("command").and_then(Value::as_str) else {
                                self.problems
                                    .push(format!("{source}: a `{event}` hook has no `command`"));
                                continue;
                            };
                            // Some hooks give the program's arguments
                            // separately; run them too, not a bare `node`.
                            let args: Vec<&str> = h
                                .get("args")
                                .and_then(Value::as_array)
                                .into_iter()
                                .flatten()
                                .filter_map(Value::as_str)
                                .collect();
                            (
                                HookAction::Command(with_args(command, &args)),
                                DEFAULT_TIMEOUT,
                            )
                        }
                        "prompt" => {
                            if !PROMPT_EVENTS.contains(&event.as_str()) {
                                self.problems.push(format!(
                                    "{source}: `prompt` hooks work on {}, not `{event}`",
                                    PROMPT_EVENTS.join(", ")
                                ));
                                continue;
                            }
                            let Some(prompt) = h
                                .get("prompt")
                                .and_then(Value::as_str)
                                .filter(|p| !p.trim().is_empty())
                            else {
                                self.problems.push(format!(
                                    "{source}: a `{event}` prompt hook has no `prompt`"
                                ));
                                continue;
                            };
                            (
                                HookAction::Prompt(prompt.to_owned()),
                                DEFAULT_PROMPT_TIMEOUT,
                            )
                        }
                        other => {
                            self.problems.push(format!(
                                "{source}: `{event}` hook type `{other}` isn't supported (use `command` or `prompt`)"
                            ));
                            continue;
                        }
                    };
                    let timeout = h
                        .get("timeout")
                        .and_then(Value::as_f64)
                        .filter(|t| *t > 0.0)
                        .map(Duration::from_secs_f64)
                        .unwrap_or(default_timeout);
                    self.hooks.push(Hook {
                        event: event.clone(),
                        matcher: matcher.clone(),
                        action,
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

    /// Hooks that have failed while running, as "where: error" lines.
    pub fn failing(&self) -> Vec<String> {
        self.failing
            .lock()
            .map(|f| f.iter().map(|(k, v)| format!("{k}: {v}")).collect())
            .unwrap_or_default()
    }

    /// Record a failure; `true` the first time this hook fails.
    fn first_failure(&self, hook: &Hook, error: &str) -> bool {
        let what = match &hook.action {
            HookAction::Command(c) => c,
            HookAction::Prompt(p) => p,
        };
        let what: String = what.chars().take(80).collect();
        let key = format!("{} hook ({}) `{what}`", hook.event, hook.source);
        self.failing
            .lock()
            .map(|mut f| f.insert(key, error.to_owned()).is_none())
            .unwrap_or(true)
    }

    /// Run `event`'s hooks matching `target` (a tool name or the
    /// SessionStart source) with `input`. `prompt` hooks are skipped.
    pub async fn run(&self, event: &str, target: &str, input: Value) -> Outcome {
        self.run_with(event, target, input, None).await
    }

    /// [`run`](Self::run), with a model for `prompt` hooks.
    pub async fn run_with(
        &self,
        event: &str,
        target: &str,
        mut input: Value,
        evaluator: Option<&dyn PromptEvaluator>,
    ) -> Outcome {
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
        let runs = matching.iter().map(|h| async {
            match &h.action {
                HookAction::Command(command) => {
                    Ran::Command(run_one(h, command, &stdin, cwd.as_deref()).await)
                }
                HookAction::Prompt(prompt) => {
                    Ran::Prompt(ask_model(h, prompt, &stdin, evaluator).await)
                }
            }
        });
        let results = futures::future::join_all(runs).await;

        let mut out = Outcome::default();
        for (hook, result) in matching.iter().zip(results) {
            let before = out.messages.len();
            let failed = match result {
                Ran::Command(r) => merge(&mut out, hook, event, r),
                Ran::Prompt(v) => merge_verdict(&mut out, hook, event, v),
            };
            // A failure's message: the first time only.
            if let Some(error) = failed {
                out.messages.truncate(before);
                if self.first_failure(hook, &error) {
                    out.messages.push(format!(
                        "{error} (it keeps running, but this won't be shown again; see Settings → Hooks)"
                    ));
                }
            }
        }
        if let (Some(updated), Some(original)) = (out.updated_input.as_mut(), original_input) {
            unalias_paths(updated, &original);
        }
        out
    }
}

enum Ran {
    Command(RunResult),
    Prompt(Result<Verdict, String>),
}

/// A prompt hook's answer.
#[derive(Debug, PartialEq, Eq)]
struct Verdict {
    ok: bool,
    reason: String,
}

const PROMPT_SYSTEM: &str = "You are a hook in a coding agent. You get a condition to check and the \
    event it applies to, as JSON. Decide whether the condition is met. Reply with JSON only, no \
    other text: {\"ok\": true} to let the agent go ahead, or {\"ok\": false, \"reason\": \"...\"} to \
    stop it, with a short reason the agent will read.";

async fn ask_model(
    hook: &Hook,
    prompt: &str,
    input: &str,
    evaluator: Option<&dyn PromptEvaluator>,
) -> Result<Verdict, String> {
    let evaluator = evaluator.ok_or("skipped: prompt hooks need a model (none is set up)")?;
    let user = if prompt.contains("$ARGUMENTS") {
        prompt.replace("$ARGUMENTS", input)
    } else {
        format!("{prompt}\n\nEvent input:\n{input}")
    };
    let reply = tokio::time::timeout(hook.timeout, evaluator.evaluate(PROMPT_SYSTEM, &user))
        .await
        .map_err(|_| format!("timed out after {}s", hook.timeout.as_secs()))??;
    parse_verdict(&reply).ok_or_else(|| format!("unclear answer: {}", first_lines(&reply)))
}

/// `{"ok": bool, "reason": …}`, or Claude Code's older `{"decision":
/// "approve" | "block"}`, anywhere in the reply.
fn parse_verdict(reply: &str) -> Option<Verdict> {
    let start = reply.find('{')?;
    let end = reply.rfind('}')?;
    let json: Value = serde_json::from_str(reply.get(start..=end)?).ok()?;
    let ok = match (json.get("ok"), json.get("decision").and_then(Value::as_str)) {
        (Some(Value::Bool(b)), _) => *b,
        (_, Some("approve" | "allow")) => true,
        (_, Some("block" | "deny")) => false,
        _ => return None,
    };
    let reason = json
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_owned();
    Some(Verdict { ok, reason })
}

/// Returns the error when the hook failed.
fn merge_verdict(
    out: &mut Outcome,
    hook: &Hook,
    event: &str,
    v: Result<Verdict, String>,
) -> Option<String> {
    match v {
        Ok(Verdict { ok: true, .. }) => None,
        Ok(Verdict { ok: false, reason }) => {
            add_block(
                out,
                if reason.is_empty() {
                    "blocked by a prompt hook".into()
                } else {
                    reason
                },
            );
            None
        }
        Err(e) => Some(format!("{event} prompt hook ({}) {e}", hook.source)),
    }
}

/// One hook as a flat rule, for editing in a UI: "when `event`
/// (matching `matcher`), run `command` / ask `prompt`". Converts to and
/// from Claude Code's nested `{event: [{matcher, hooks: [...]}]}` shape.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct HookRule {
    pub event: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matcher: Option<String>,
    /// `command` or `prompt`.
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// Seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<f64>,
}

/// Every hook in a config (`{event: [...]}` or `{"hooks": {...}}`), in
/// order. Entries that aren't objects are skipped.
pub fn rules_from_config(config: &Value) -> Vec<HookRule> {
    let events = config.get("hooks").unwrap_or(config);
    let mut out = Vec::new();
    for (event, groups) in events.as_object().into_iter().flatten() {
        for group in groups.as_array().into_iter().flatten() {
            let matcher = group
                .get("matcher")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|m| !m.is_empty() && *m != "*")
                .map(str::to_owned);
            for h in group
                .get("hooks")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let s = |k: &str| h.get(k).and_then(Value::as_str).map(str::to_owned);
                out.push(HookRule {
                    event: event.clone(),
                    matcher: matcher.clone(),
                    kind: s("type").unwrap_or_else(|| "command".into()),
                    command: s("command"),
                    prompt: s("prompt"),
                    timeout: h.get("timeout").and_then(Value::as_f64),
                });
            }
        }
    }
    out
}

/// The config for `rules`, grouping rules with the same event and
/// matcher. `None` when there are none.
pub fn config_from_rules(rules: &[HookRule]) -> Option<Value> {
    let mut events = serde_json::Map::new();
    for r in rules {
        let groups = events
            .entry(r.event.clone())
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .expect("groups are arrays");
        let matcher = r
            .matcher
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty());
        let idx = match groups
            .iter()
            .position(|g| g.get("matcher").and_then(Value::as_str) == matcher)
        {
            Some(i) => i,
            None => {
                let mut g = json!({"hooks": []});
                if let Some(m) = matcher {
                    g["matcher"] = json!(m);
                }
                groups.push(g);
                groups.len() - 1
            }
        };
        let mut hook = json!({"type": r.kind});
        if let Some(c) = &r.command {
            hook["command"] = json!(c);
        }
        if let Some(p) = &r.prompt {
            hook["prompt"] = json!(p);
        }
        if let Some(t) = r.timeout {
            hook["timeout"] = json!(t);
        }
        groups[idx]["hooks"]
            .as_array_mut()
            .expect("hooks array")
            .push(hook);
    }
    (!events.is_empty()).then_some(Value::Object(events))
}

/// `command` followed by `args`, each double-quoted so spaces survive
/// while `${CLAUDE_PLUGIN_ROOT}`-style variables still expand.
fn with_args(command: &str, args: &[&str]) -> String {
    let mut out = command.to_owned();
    for a in args {
        let escaped = a
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('`', "\\`");
        out.push_str(&format!(" \"{escaped}\""));
    }
    out
}

/// The plugin folder as its hooks expect to see it. Installed plugins
/// live in `…/<plugin>/<version>/`, but some hooks (hookify's) add the
/// folder's parent to Python's path and `import <plugin>`, which needs
/// the folder to be named after the plugin. So point them at a link
/// `…/<plugin>/.linked/<version>/<plugin>` → the real folder. Falls
/// back to the real folder when the name already fits or linking fails.
fn named_plugin_root(root: &Path, source: &str) -> PathBuf {
    let name = source.split('@').next().unwrap_or(source);
    let Some(version) = root.file_name().and_then(|v| v.to_str()) else {
        return root.to_path_buf();
    };
    if version == name || name.is_empty() || name.contains(['/', '\\']) {
        return root.to_path_buf();
    }
    let Some(plugin_dir) = root.parent() else {
        return root.to_path_buf();
    };
    let link = plugin_dir.join(".linked").join(version).join(name);
    if link.exists() {
        return link;
    }
    #[cfg(unix)]
    {
        let made = std::fs::create_dir_all(link.parent().unwrap_or(plugin_dir))
            .and_then(|_| std::os::unix::fs::symlink(root, &link));
        if made.is_ok() || link.exists() {
            return link;
        }
    }
    root.to_path_buf()
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

async fn run_one(hook: &Hook, command: &str, stdin: &str, cwd: Option<&Path>) -> RunResult {
    let mut cmd = if cfg!(windows) {
        let mut c = tokio::process::Command::new("cmd");
        c.args(["/C", command]);
        c
    } else {
        let mut c = tokio::process::Command::new("sh");
        c.args(["-c", command]);
        c
    };
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
        cmd.env("CLAUDE_PROJECT_DIR", dir)
            .env("MIRA_PROJECT_DIR", dir);
    }
    if let Some(root) = &hook.plugin_root {
        let root = named_plugin_root(root, &hook.source);
        cmd.env("CLAUDE_PLUGIN_ROOT", &root)
            .env("MIRA_PLUGIN_ROOT", &root);
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

/// Returns the error when the hook failed (couldn't run, or exited
/// with a code other than 0 or 2).
fn merge(out: &mut Outcome, hook: &Hook, event: &str, result: RunResult) -> Option<String> {
    let label = format!("{} hook ({})", event, hook.source);
    let (code, stdout, stderr) = match result {
        RunResult::Failed(e) => return Some(format!("{label} {e}")),
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
        return None;
    }
    if code != 0 {
        let detail = if stderr.is_empty() { stdout } else { stderr };
        return Some(format!(
            "{label} failed (exit {code}): {}",
            first_lines(&detail)
        ));
    }
    let json: Option<Value> = stdout
        .starts_with('{')
        .then(|| serde_json::from_str(&stdout).ok())
        .flatten();
    let Some(json) = json else {
        if !stdout.is_empty() && (event == "SessionStart" || event == "UserPromptSubmit") {
            out.context.push(stdout);
        }
        return None;
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
    None
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
            &json!({
                "PreToolUse": [
                    {"matcher": "(", "hooks": [{"type": "command", "command": "true"}]},
                    {"hooks": [{"type": "agent", "prompt": "x"}]},
                    {"hooks": [{"type": "prompt", "prompt": " "}]}
                ],
                "PostToolUse": [{"hooks": [{"type": "prompt", "prompt": "x"}]}]
            }),
            None,
            "p",
        );
        assert!(s.hooks.is_empty());
        assert_eq!(s.problems.len(), 4, "{:?}", s.problems);
        assert!(s
            .problems
            .iter()
            .any(|p| p.contains("prompt` hooks work on")));
    }

    /// Answers with a fixed reply and records what it was asked.
    struct FakeModel {
        reply: Result<String, String>,
        asked: std::sync::Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl PromptEvaluator for FakeModel {
        async fn evaluate(&self, _system: &str, user: &str) -> Result<String, String> {
            self.asked.lock().unwrap().push(user.to_owned());
            self.reply.clone()
        }
    }

    fn fake(reply: Result<&str, &str>) -> FakeModel {
        FakeModel {
            reply: reply.map(str::to_owned).map_err(str::to_owned),
            asked: Default::default(),
        }
    }

    #[tokio::test]
    async fn prompt_hooks_ask_the_model_and_block_on_no() {
        let s = set(json!({"Stop": [{"hooks": [
            {"type": "prompt", "prompt": "Are the tests passing? $ARGUMENTS"}
        ]}]}));
        assert!(s.has("Stop"));

        let no = fake(Ok(
            "Checking… {\"ok\": false, \"reason\": \"tests still fail\"}",
        ));
        let out = s
            .run_with("Stop", "", json!({"stop_hook_active": false}), Some(&no))
            .await;
        assert_eq!(out.block.as_deref(), Some("tests still fail"));
        let asked = no.asked.lock().unwrap()[0].clone();
        assert!(asked.starts_with("Are the tests passing? {"), "{asked}");
        assert!(asked.contains("\"hook_event_name\":\"Stop\""));

        let yes = fake(Ok("{\"ok\": true}"));
        let out = s.run_with("Stop", "", json!({}), Some(&yes)).await;
        assert!(out.block.is_none() && out.messages.is_empty());
    }

    #[tokio::test]
    async fn prompt_hook_failures_let_things_through() {
        // A fresh set per case: a hook's failure is only reported once.
        let s = || {
            set(json!({"PreToolUse": [{"matcher": "Bash", "hooks": [
                {"type": "prompt", "prompt": "Is this command safe?"}
            ]}]}))
        };
        let input = || input("bash", json!({"command": "ls"}));

        let down = fake(Err("provider returned 503"));
        let out = s()
            .run_with("PreToolUse", "bash", input(), Some(&down))
            .await;
        assert!(out.block.is_none());
        assert!(out.messages[0].contains("503"), "{:?}", out.messages);
        // No template placeholder: the input is appended.
        assert!(down.asked.lock().unwrap()[0].contains("Event input:"));

        let rambling = fake(Ok("sure, looks fine"));
        let out = s()
            .run_with("PreToolUse", "bash", input(), Some(&rambling))
            .await;
        assert!(out.block.is_none());
        assert!(out.messages[0].contains("unclear answer"));

        let out = s().run("PreToolUse", "bash", input()).await;
        assert!(out.block.is_none());
        assert!(out.messages[0].contains("need a model"));
    }

    #[test]
    fn rules_round_trip_through_the_config_shape() {
        let config = json!({"hooks": {
            "PreToolUse": [
                {"matcher": "Bash", "hooks": [
                    {"type": "command", "command": "a.sh", "timeout": 10.0},
                    {"type": "prompt", "prompt": "safe?"}
                ]},
                {"matcher": "*", "hooks": [{"type": "command", "command": "any.sh"}]}
            ],
            "Stop": [{"hooks": [{"command": "done.sh"}]}]
        }});
        let rules = rules_from_config(&config);
        assert_eq!(rules.len(), 4);
        assert_eq!(rules[0].matcher.as_deref(), Some("Bash"));
        assert_eq!(rules[0].timeout, Some(10.0));
        assert_eq!(rules[1].kind, "prompt");
        assert_eq!(rules[2].matcher, None, "`*` means any");
        assert_eq!(rules[3].kind, "command", "type defaults to command");

        let back = config_from_rules(&rules).unwrap();
        assert_eq!(
            back["PreToolUse"].as_array().unwrap().len(),
            2,
            "grouped by matcher"
        );
        assert_eq!(back["PreToolUse"][0]["hooks"].as_array().unwrap().len(), 2);
        assert_eq!(rules_from_config(&back), rules);
        // The result loads as a working hook set.
        let mut set = HookSet::default();
        set.add(&back, None, "t");
        assert!(set.problems.is_empty(), "{:?}", set.problems);
        assert_eq!(set.hooks.len(), 4);

        assert_eq!(config_from_rules(&[]), None);
    }

    #[tokio::test]
    async fn plugin_folder_is_named_after_the_plugin_for_imports() {
        // Installed like the real cache: …/hookify/0.1.0/, with a package
        // the hook imports the way hookify's own scripts do.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("hookify").join("0.1.0");
        std::fs::create_dir_all(root.join("core")).unwrap();
        std::fs::write(
            root.join("core").join("rules.py"),
            "GREETING = 'imported'\n",
        )
        .unwrap();
        let script = "import os, sys; p = os.environ['CLAUDE_PLUGIN_ROOT']; \
                      sys.path.insert(0, os.path.dirname(p)); \
                      from hookify.core.rules import GREETING; print(GREETING)";
        let mut s = HookSet::default();
        s.add(
            &json!({"SessionStart": [{"hooks": [
                {"type": "command", "command": "python3 -c \"$HOOK_SCRIPT\""}
            ]}]}),
            Some(&root),
            "hookify@claude-code-plugins",
        );
        std::env::set_var("HOOK_SCRIPT", script);
        let out = s
            .run(
                "SessionStart",
                "startup",
                json!({"cwd": std::env::temp_dir()}),
            )
            .await;
        assert_eq!(out.context, ["imported"], "{:?}", out.messages);
        assert!(root
            .parent()
            .unwrap()
            .join(".linked/0.1.0/hookify")
            .exists());
    }

    #[tokio::test]
    async fn args_are_passed_to_the_command() {
        let s = set(json!({"UserPromptSubmit": [{"hooks": [
            {"type": "command", "command": "printf '%s|'", "args": ["a b", "c\"d"]}
        ]}]}));
        let out = s
            .run(
                "UserPromptSubmit",
                "",
                json!({"prompt": "hi", "cwd": std::env::temp_dir()}),
            )
            .await;
        assert_eq!(out.context, ["a b|c\"d|"], "{:?}", out.messages);
    }

    #[tokio::test]
    async fn a_failing_hook_is_reported_once() {
        let s = set(json!({"UserPromptSubmit": [{"hooks": [
            {"type": "command", "command": "echo broken >&2; exit 1"}
        ]}]}));
        let input = || json!({"prompt": "hi", "cwd": std::env::temp_dir()});
        let first = s.run("UserPromptSubmit", "", input()).await;
        assert_eq!(first.messages.len(), 1);
        assert!(first.messages[0].contains("broken"));
        assert!(first.messages[0].contains("won't be shown again"));
        let second = s.run("UserPromptSubmit", "", input()).await;
        assert!(second.messages.is_empty(), "{:?}", second.messages);
        let failing = s.failing();
        assert_eq!(failing.len(), 1);
        assert!(
            failing[0].starts_with("UserPromptSubmit hook (test)"),
            "{failing:?}"
        );
    }

    #[test]
    fn verdicts_in_both_formats() {
        let v = |s: &str| parse_verdict(s).map(|v| (v.ok, v.reason));
        assert_eq!(v("{\"ok\": true}"), Some((true, String::new())));
        assert_eq!(
            v("{\"decision\": \"block\", \"reason\": \"no\"}"),
            Some((false, "no".into()))
        );
        assert_eq!(v("{\"decision\": \"approve\"}").map(|x| x.0), Some(true));
        assert_eq!(v("{\"maybe\": 1}"), None);
        assert_eq!(v("no json here"), None);
    }
}
