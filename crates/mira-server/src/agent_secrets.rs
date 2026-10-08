//! Agent tools that put something in front of the user rather than act on
//! the project: a private prompt for a secret, and publishing a rendered
//! HTML page into the chat.
//!
//! A secret never enters the transcript or the model's context. The user
//! types it into a private input; Mira writes it to a file only the user can
//! read and tells the agent the file's path, so its commands can use the
//! value (`$(cat path)`) without the agent ever seeing it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use crate::interactive::PromptResponse;
use crate::protocol::ServerMsg;
use crate::slot::SessionSlot;
use crate::state::AppState;

/// How long a secret prompt waits for the user.
const SECRET_WAIT: Duration = Duration::from_secs(15 * 60);

/// Largest page `html_render` accepts.
pub const MAX_HTML_BYTES: usize = 2 * 1024 * 1024;

/// `NAME`-style: what the agent will export it as.
fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_uppercase())
        && name.len() <= 64
        && chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// Where this chat's secrets live: beside its transcript, out of the project.
fn secrets_dir(state: &AppState, slot: &SessionSlot) -> Option<PathBuf> {
    let log = state.store.as_ref()?.agent_log_path(&slot.id)?;
    Some(log.parent()?.join("secrets").join(slot.id.to_string()))
}

/// Write a file only the user can read. `lock_dir` also restricts its
/// folder — for Mira's own secrets folder, never for a project directory.
fn write_private(path: &Path, value: &str, lock_dir: bool) -> std::io::Result<()> {
    use std::io::Write;
    let dir = path
        .parent()
        .ok_or_else(|| std::io::Error::other("no parent"))?;
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        if lock_dir {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(value.as_bytes())
    }
    #[cfg(not(unix))]
    {
        let _ = lock_dir;
        std::fs::write(path, value)
    }
}

/// Set `NAME=value` in a dotenv file, replacing an existing line.
fn upsert_dotenv(path: &Path, name: &str, value: &str) -> std::io::Result<()> {
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let quoted = if value
        .chars()
        .any(|c| c.is_whitespace() || "#\"'$`\\".contains(c))
    {
        format!("'{}'", value.replace('\'', "'\\''"))
    } else {
        value.to_string()
    };
    let line = format!("{name}={quoted}");
    let mut found = false;
    let mut out: Vec<String> = existing
        .lines()
        .map(|l| {
            let key = l
                .trim_start()
                .trim_start_matches("export ")
                .split('=')
                .next()
                .unwrap_or("");
            if key.trim() == name {
                found = true;
                line.clone()
            } else {
                l.to_string()
            }
        })
        .collect();
    if !found {
        out.push(line);
    }
    write_private(path, &(out.join("\n") + "\n"), false)
}

/// A project-relative dotenv path, kept inside the project.
async fn dotenv_path(slot: &SessionSlot, rel: &str) -> Result<PathBuf, String> {
    let rel = Path::new(rel);
    if rel.is_absolute()
        || rel
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err("`dotenv` must be a path inside the project, like .env.local".into());
    }
    let name = rel.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if !name.starts_with(".env") {
        return Err("`dotenv` must name a .env file (.env, .env.local, …)".into());
    }
    let cwd = slot.cwd.read().await.clone();
    Ok(cwd.join(rel))
}

/// `request_secret`: ask the user privately, store the answer, say where.
pub async fn request_secret(
    state: &AppState,
    slot: &SessionSlot,
    args: &Value,
) -> Result<String, String> {
    let name = args
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if !valid_name(name) {
        return Err("`name` must look like an environment variable: STRIPE_SECRET_KEY".into());
    }
    let reason = args
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if reason.is_empty() {
        return Err("say in `reason` what the secret is for".into());
    }
    let dotenv = args
        .get("dotenv")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|d| !d.is_empty());
    let dotenv_target = match dotenv {
        Some(d) => Some(dotenv_path(slot, d).await?),
        None => None,
    };
    let dir = secrets_dir(state, slot).ok_or("this chat has no private storage for secrets")?;

    let prompt_id = format!("secret-{}", uuid::Uuid::new_v4().simple());
    let (tx, rx) = tokio::sync::oneshot::channel();
    slot.prompt_pending
        .lock()
        .await
        .insert(prompt_id.clone(), tx);
    let _ = slot.events_tx.send(ServerMsg::SecretRequest {
        prompt_id: prompt_id.clone(),
        name: name.to_string(),
        reason: reason.to_string(),
        dotenv: dotenv.map(str::to_string),
    });
    let answer = tokio::time::timeout(SECRET_WAIT, rx).await;
    slot.prompt_pending.lock().await.remove(&prompt_id);
    let value = match answer {
        Ok(Ok(PromptResponse::Secret(s))) if !s.cancelled => s.value.filter(|v| !v.is_empty()),
        Ok(Ok(_)) => None,
        Ok(Err(_)) => return Err("The chat closed before the user answered.".into()),
        Err(_) => return Err("The user didn't answer the secret prompt.".into()),
    };
    let Some(value) = value else {
        return Ok(format!(
            "The user declined to provide {name}. Don't ask again unless they bring it up."
        ));
    };
    let path = dir.join(name);
    write_private(&path, &value, true).map_err(|e| format!("could not store the secret: {e}"))?;
    let mut text = format!(
        "The user provided {name}. It is stored privately at {p} (readable only by the user). \
         Use it in commands without printing it, e.g. `export {name}=\"$(cat '{p}')\"`. \
         Never echo, log or commit its value.",
        p = path.display()
    );
    if let (Some(target), Some(rel)) = (dotenv_target, dotenv) {
        upsert_dotenv(&target, name, &value).map_err(|e| format!("could not write {rel}: {e}"))?;
        text.push_str(&format!(
            " It was also set in {rel}; make sure that file is git-ignored."
        ));
    }
    Ok(text)
}

/// `html_render`: publish a self-contained page into the chat.
pub fn html_render(slot: &SessionSlot, args: &Value) -> Result<String, String> {
    let html = args.get("html").and_then(Value::as_str).unwrap_or("");
    if html.trim().is_empty() {
        return Err("`html` is empty".into());
    }
    if html.len() > MAX_HTML_BYTES {
        return Err(format!(
            "the page is {} KB; the limit is 2 MB",
            html.len() / 1024
        ));
    }
    let title = args
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .unwrap_or("Page")
        .chars()
        .take(120)
        .collect::<String>();
    let _ = slot.events_tx.send(ServerMsg::HtmlRender {
        id: uuid::Uuid::new_v4().simple().to_string(),
        title: title.clone(),
        html: html.to_string(),
    });
    Ok(format!(
        "Published \"{title}\" to the chat. The user sees it above your reply, so don't restate it."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_names_look_like_env_vars() {
        assert!(valid_name("STRIPE_SECRET_KEY"));
        assert!(valid_name("K2"));
        assert!(!valid_name("stripe"));
        assert!(!valid_name("2KEY"));
        assert!(!valid_name("A-B"));
        assert!(!valid_name(""));
    }

    #[test]
    fn dotenv_upsert_replaces_or_appends_and_quotes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(".env.local");
        std::fs::write(&p, "A=1\nexport KEY=old\n").unwrap();
        upsert_dotenv(&p, "KEY", "new value").unwrap();
        upsert_dotenv(&p, "B", "x").unwrap();
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            "A=1\nKEY='new value'\nB=x\n"
        );
    }
}
