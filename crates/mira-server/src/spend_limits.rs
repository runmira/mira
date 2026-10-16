//! Session spend controls. Raising a session never bypasses daily or monthly policy.
use crate::{slot::SessionSlot, state::AppState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use mira_harness::spend::SpendAccount;

pub async fn account(slot: &SessionSlot) -> Result<SpendAccount, String> {
    SpendAccount::user(slot.id.to_string(), slot.cwd.read().await.clone())
        .ok_or_else(|| "Cannot locate the spend ledger: HOME is unavailable".into())
}
pub async fn check(slot: &SessionSlot) -> Result<(), String> {
    let account = account(slot).await?;
    tokio::task::spawn_blocking(move || account.ensure_allowed())
        .await
        .map_err(|e| e.to_string())?
}
pub async fn status(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let Some(slot) = state.slot_str(&id).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let result = match account(&slot).await {
        Ok(a) => tokio::task::spawn_blocking(move || a.status())
            .await
            .map_err(|e| e.to_string())
            .and_then(|r| r),
        Err(e) => Err(e),
    };
    match result {
        Ok(status) => Json(status).into_response(),
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": e})),
        )
            .into_response(),
    }
}
pub async fn raise(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let Some(slot) = state.slot_str(&id).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let result = match account(&slot).await {
        Ok(a) => tokio::task::spawn_blocking(move || a.raise_session_limit())
            .await
            .map_err(|e| e.to_string())
            .and_then(|r| r),
        Err(e) => Err(e),
    };
    match result {
        Ok(limit) => Json(serde_json::json!({"limit": limit})).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e})),
        )
            .into_response(),
    }
}
