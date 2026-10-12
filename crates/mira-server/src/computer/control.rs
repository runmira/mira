use super::*;
use mira_computer::remote::ControlRequest;
pub(super) fn routes(state: ControlApi) -> Router {
    Router::new()
        .route("/api/computer/identity", get(identity))
        .route("/api/computer/pairing/claim", post(pairing::claim))
        .route("/api/computer/control", post(control))
        .route("/api/computer/windows", post(windows))
        .route("/api/computer/access/revoke", post(revoke_self))
        .layer(axum::extract::DefaultBodyLimit::max(2 * 1024 * 1024))
        .with_state(state)
}
async fn identity(State(api): State<ControlApi>, headers: HeaderMap) -> Response {
    let grant = bearer(&headers).and_then(|t| api.computer.grants.authenticate(t));
    let backend = grant
        .map(|g| g.backend)
        .or_else(|| api.computer.grants.open_backend());
    let Some(backend) = backend else {
        return failure(
            StatusCode::GONE,
            "No pairing link is open on that computer.",
        );
    };
    if backend == "native" {
        return Json(native_identity(&api.machine_id)).into_response();
    }
    match api.computer.local_cua(Delivery::Auto, None).await {
        Ok(b) => match cua_identity(b, &api.machine_id).await {
            Ok(v) => Json(v).into_response(),
            Err(e) => failure(StatusCode::SERVICE_UNAVAILABLE, e),
        },
        Err(e) => failure(StatusCode::SERVICE_UNAVAILABLE, e),
    }
}
async fn control(
    State(api): State<ControlApi>,
    headers: HeaderMap,
    Json(body): Json<ControlRequest>,
) -> Response {
    let Some(token) = bearer(&headers) else {
        return failure(StatusCode::UNAUTHORIZED, "Computer pairing required.");
    };
    if api.computer.grants.authenticate(token).is_none() {
        return failure(
            StatusCode::UNAUTHORIZED,
            "Computer pairing has been revoked.",
        );
    }
    match api.computer.shared_control(token, body).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => failure(StatusCode::BAD_REQUEST, e),
    }
}
async fn windows(State(api): State<ControlApi>, headers: HeaderMap) -> Response {
    let Some(grant) = bearer(&headers).and_then(|t| api.computer.grants.authenticate(t)) else {
        return failure(
            StatusCode::UNAUTHORIZED,
            "Computer pairing has been revoked.",
        );
    };
    if grant.backend == "native" {
        return Json(Vec::<TargetWindow>::new()).into_response();
    }
    match api.computer.local_cua(Delivery::Auto, None).await {
        Ok(b) => match b.list_windows().await {
            Ok(v) => Json(v).into_response(),
            Err(e) => failure(StatusCode::BAD_GATEWAY, e),
        },
        Err(e) => failure(StatusCode::BAD_GATEWAY, e),
    }
}
async fn revoke_self(State(api): State<ControlApi>, headers: HeaderMap) -> Response {
    let Some(grant) = bearer(&headers).and_then(|t| api.computer.grants.authenticate(t)) else {
        return failure(
            StatusCode::UNAUTHORIZED,
            "Computer pairing has been revoked.",
        );
    };
    match api.computer.revoke(&grant.id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => failure(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}
