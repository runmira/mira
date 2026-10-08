//! Explicit, allowlisted CLI updates. No shell or client-supplied arguments.
use axum::{
    extract::Path,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use std::time::Duration;

fn command(kind: &str) -> Option<(&'static str, &'static str)> {
    match kind {
        "codex" => Some(("codex", "update")),
        "claude-code" => Some(("claude", "update")),
        "opencode" => Some(("opencode", "upgrade")),
        _ => None,
    }
}

pub async fn update(Path(kind): Path<String>) -> Response {
    static UPDATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let Some((binary, argument)) = command(&kind) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"This agent does not support updating from Mira."})),
        )
            .into_response();
    };
    let Ok(_guard) = UPDATE.try_lock() else {
        return (
            StatusCode::CONFLICT,
            Json(
                json!({"error":"An agent update is already running. Try again when it finishes."}),
            ),
        )
            .into_response();
    };
    let resolved = tokio::task::spawn_blocking(move || mira_acp::which::resolve(binary))
        .await
        .ok()
        .flatten();
    let Some(resolved) = resolved else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error":format!("{binary} is not installed.")})),
        )
            .into_response();
    };
    let mut process = tokio::process::Command::new(resolved);
    process
        .arg(argument)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    if let Some(path) = mira_acp::which::effective_path() {
        process.env("PATH", path);
    }
    match tokio::time::timeout(Duration::from_secs(180), process.output()).await {
        Ok(Ok(output)) if output.status.success() => Json(json!({"updated":true})).into_response(),
        Ok(Ok(output)) => {
            let error = String::from_utf8_lossy(&output.stderr);
            let error = if error.trim().is_empty() { String::from_utf8_lossy(&output.stdout).into_owned() } else { error.into_owned() };
            let tail = error.chars().rev().take(2000).collect::<Vec<_>>().into_iter().rev().collect::<String>();
            (StatusCode::BAD_GATEWAY, Json(json!({"error":format!("Agent update failed: {}",tail.trim())}))).into_response()
        }
        Ok(Err(error)) => (StatusCode::BAD_GATEWAY, Json(json!({"error":format!("Could not start update: {error}")}))).into_response(),
        Err(_) => (StatusCode::GATEWAY_TIMEOUT, Json(json!({"error":"The update timed out. Check the agent in a terminal before retrying."}))).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn unsupported_updates_return_an_error_without_starting_a_process() {
        let response = update(Path("not-a-driver".into())).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
    #[test]
    fn updates_only_accept_known_drivers_and_fixed_arguments() {
        assert_eq!(command("codex"), Some(("codex", "update")));
        assert_eq!(command("claude-code"), Some(("claude", "update")));
        assert_eq!(command("opencode"), Some(("opencode", "upgrade")));
        assert_eq!(command("codex; echo injected"), None);
        assert_eq!(command("unknown"), None);
    }
}
