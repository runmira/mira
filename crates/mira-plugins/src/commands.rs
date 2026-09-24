//! Custom slash commands: Markdown prompt templates, the same format as
//! Claude Code's.
//!
//! ```markdown
//! ---
//! description: Commit, push, and open a PR
//! argument-hint: [message]
//! ---
//! Current status: !`git status`
//!
//! Commit with message: $ARGUMENTS
//! ```
//!
//! Sources: `~/.mira/commands/`, the project's `.mira/commands/` and
//! `.claude/commands/`, and enabled plugins. Running one expands
//! `$ARGUMENTS` / `$1`…`$9`, runs each `` !`command` `` in the project
//! and splices in its output, and sends the result as the user message.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CommandSource {
    User,
    Project,
    Plugin { plugin: String },
}

#[derive(Clone, Debug, Serialize)]
pub struct SlashCommand {
    /// What you type after `/`.
    pub name: String,
    /// For plugin commands, `plugin:name`, which always works even when
    /// `name` is taken by a user or project command.
    pub qualified: Option<String>,
    pub description: String,
    pub argument_hint: Option<String>,
    pub source: CommandSource,
    pub path: PathBuf,
    #[serde(skip)]
    pub body: String,
}

impl SlashCommand {
    pub fn matches(&self, name: &str) -> bool {
        self.name == name || self.qualified.as_deref() == Some(name)
    }
}

/// Split YAML frontmatter from the body.
pub fn split_frontmatter(text: &str) -> (Value, &str) {
    let rest = text.strip_prefix("\u{feff}").unwrap_or(text);
    let Some(after) = rest.strip_prefix("---") else {
        return (Value::Null, text);
    };
    let after = after.trim_start_matches(['\r', ' ', '\t']);
    let Some(after) = after.strip_prefix('\n') else {
        return (Value::Null, text);
    };
    let Some(end) = after.find("\n---") else {
        return (Value::Null, text);
    };
    let yaml = &after[..end];
    let body = after[end + 4..].trim_start_matches(['-']);
    let body = body
        .strip_prefix("\r\n")
        .or_else(|| body.strip_prefix('\n'))
        .unwrap_or(body);
    let meta: Value = serde_yaml::from_str(yaml).unwrap_or(Value::Null);
    (meta, body)
}

pub fn parse(path: &Path, name: String, source: CommandSource) -> Option<SlashCommand> {
    let text = std::fs::read_to_string(path).ok()?;
    let (meta, body) = split_frontmatter(&text);
    let field = |k: &str| {
        meta.get(k).and_then(|v| match v {
            Value::String(s) => Some(s.clone()),
            Value::Array(a) => Some(
                a.iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            _ => None,
        })
    };
    let description = field("description").unwrap_or_else(|| {
        body.lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .map(|l| l.trim_start_matches('#').trim().chars().take(80).collect())
            .unwrap_or_default()
    });
    Some(SlashCommand {
        name,
        qualified: None,
        description,
        argument_hint: field("argument-hint").or_else(|| field("argument_hint")),
        source,
        path: path.to_path_buf(),
        body: body.to_owned(),
    })
}

fn collect_dir(dir: &Path, source: &CommandSource, out: &mut Vec<SlashCommand>) {
    let mut files = Vec::new();
    walk(dir, &mut files);
    for f in files {
        let Some(stem) = f.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
            continue;
        };
        if out.iter().any(|c| c.name == stem) {
            continue; // an earlier (higher-priority) source has it
        }
        if let Some(c) = parse(&f, stem, source.clone()) {
            out.push(c);
        }
    }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|e| e == "md") {
            out.push(p);
        }
    }
}

/// All commands, highest priority first: project, user, then plugins.
/// A plugin command whose short name is taken keeps only its qualified
/// `plugin:name` form.
pub fn load(
    user_dir: Option<&Path>,
    project: Option<&Path>,
    plugins: &[(String, Vec<PathBuf>)],
) -> Vec<SlashCommand> {
    let mut out = Vec::new();
    if let Some(p) = project {
        collect_dir(
            &p.join(".mira").join("commands"),
            &CommandSource::Project,
            &mut out,
        );
        collect_dir(
            &p.join(".claude").join("commands"),
            &CommandSource::Project,
            &mut out,
        );
    }
    if let Some(u) = user_dir {
        collect_dir(u, &CommandSource::User, &mut out);
    }
    for (plugin, files) in plugins {
        for f in files {
            let Some(stem) = f.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
                continue;
            };
            let qualified = format!("{plugin}:{stem}");
            let taken = out.iter().any(|c| c.name == stem);
            let name = if taken { qualified.clone() } else { stem };
            if let Some(mut c) = parse(
                f,
                name,
                CommandSource::Plugin {
                    plugin: plugin.clone(),
                },
            ) {
                c.qualified = Some(qualified);
                out.push(c);
            }
        }
    }
    out
}

/// Expand a command for sending: arguments, then `` !`shell` `` context
/// run in `cwd` (each capped at 30 s).
pub async fn render(cmd: &SlashCommand, args: &str, cwd: &Path) -> String {
    let mut text = substitute_args(&cmd.body, args);
    if !cmd.body.contains("$ARGUMENTS") && !args.trim().is_empty() && !has_positional(&cmd.body) {
        // Like Claude Code: arguments with no placeholder are appended.
        text.push_str("\n\nARGUMENTS: ");
        text.push_str(args.trim());
    }
    run_inline_shell(&text, cwd).await
}

fn has_positional(body: &str) -> bool {
    (1..=9).any(|i| body.contains(&format!("${i}")))
}

pub fn substitute_args(body: &str, args: &str) -> String {
    let words = shell_words(args);
    let mut out = body.replace("$ARGUMENTS", args.trim());
    for i in (1..=9).rev() {
        let v = words.get(i - 1).map(String::as_str).unwrap_or("");
        out = out.replace(&format!("${i}"), v);
    }
    out
}

/// Split arguments like a shell would for `$1`…`$9` (quotes group).
fn shell_words(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut any = false;
    for c in s.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => cur.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                any = true;
            }
            (None, c) if c.is_whitespace() => {
                if any || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            (None, c) => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Replace each `` !`command` `` with the command's output.
async fn run_inline_shell(text: &str, cwd: &Path) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("!`") {
        let after = &rest[start + 2..];
        let Some(end) = after.find('`') else { break };
        let command = &after[..end];
        out.push_str(&rest[..start]);
        if command.trim().is_empty() || command.contains('\n') {
            out.push_str(&rest[start..start + 2 + end + 1]);
        } else {
            out.push_str(&shell_output(command, cwd).await);
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

async fn shell_output(command: &str, cwd: &Path) -> String {
    let mut cmd = if cfg!(windows) {
        let mut c = tokio::process::Command::new("cmd");
        c.args(["/C", command]);
        c
    } else {
        let mut c = tokio::process::Command::new("sh");
        c.args(["-c", command]);
        c
    };
    cmd.current_dir(cwd).kill_on_drop(true);
    match tokio::time::timeout(Duration::from_secs(30), cmd.output()).await {
        Ok(Ok(o)) => {
            let mut s = String::from_utf8_lossy(&o.stdout).into_owned();
            let err = String::from_utf8_lossy(&o.stderr);
            if !err.trim().is_empty() {
                s.push_str(&err);
            }
            s.trim_end().to_owned()
        }
        Ok(Err(e)) => format!("[`{command}` failed: {e}]"),
        Err(_) => format!("[`{command}` timed out]"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_and_args() {
        let (meta, body) = split_frontmatter(
            "---\ndescription: Do it\nargument-hint: [x]\n---\nHello $1 and $2 ($ARGUMENTS)\n",
        );
        assert_eq!(meta["description"], "Do it");
        assert_eq!(body, "Hello $1 and $2 ($ARGUMENTS)\n");
        assert_eq!(
            substitute_args(body, r#"world "big planet""#),
            "Hello world and big planet (world \"big planet\")\n"
        );
        let (meta, body) = split_frontmatter("no frontmatter");
        assert!(meta.is_null());
        assert_eq!(body, "no frontmatter");
    }

    #[tokio::test]
    async fn loads_with_priority_and_renders_shell_context() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("p");
        let user = dir.path().join("u");
        let plugin = dir.path().join("plug");
        for (p, text) in [
            (project.join(".mira/commands/review.md"), "project review"),
            (user.join("review.md"), "user review"),
            (
                user.join("hello.md"),
                "---\ndescription: Say hi\n---\nSay hi to $ARGUMENTS. Dir: !`echo inline-ok`",
            ),
            (plugin.join("commands/review.md"), "plugin review"),
            (plugin.join("commands/commit.md"), "commit it"),
        ] {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, text).unwrap();
        }
        let cmds = load(
            Some(&user),
            Some(&project),
            &[(
                "git-tools".to_owned(),
                vec![
                    plugin.join("commands/review.md"),
                    plugin.join("commands/commit.md"),
                ],
            )],
        );
        let find = |n: &str| cmds.iter().find(|c| c.matches(n)).unwrap();
        assert_eq!(find("review").source, CommandSource::Project);
        assert!(find("git-tools:review").body.contains("plugin"));
        assert_eq!(
            find("commit").qualified.as_deref(),
            Some("git-tools:commit")
        );
        assert_eq!(find("git-tools:commit").name, "commit");
        let hello = find("hello");
        assert_eq!(hello.description, "Say hi");
        let text = render(hello, "Ada", &project).await;
        assert_eq!(text, "Say hi to Ada. Dir: inline-ok");
        // No placeholder: arguments are appended.
        let text = render(find("commit"), "fast", &project).await;
        assert!(text.ends_with("ARGUMENTS: fast"), "{text}");
    }
}
