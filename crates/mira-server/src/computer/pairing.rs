use super::*;
use mira_computer::backend::cua::validate_endpoint;
#[derive(Deserialize)]
pub(super) struct PairLink {
    link: String,
}
#[derive(Deserialize)]
pub(super) struct CloseCode {
    code: String,
}
#[derive(Deserialize)]
pub(super) struct Claim {
    code: String,
    name: String,
}
#[derive(Serialize, Deserialize)]
pub(super) struct Claimed {
    pub identity: Identity,
    pub device_id: String,
    pub token: String,
}
pub(super) fn parse_link(link: &str) -> Result<(String, String), String> {
    if link.len() > 2048 {
        return Err("Pairing link is too long.".into());
    }
    let mut url = reqwest::Url::parse(link.trim())
        .map_err(|_| "Paste the pairing link shown in Mira on the other computer.")?;
    let code = url
        .fragment()
        .and_then(|f| f.strip_prefix("computer-pair="))
        .ok_or("The pairing link has no code.")?
        .to_owned();
    if code.len() != 10 || !code.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("The pairing code is invalid.".into());
    }
    url.set_fragment(None);
    validate_endpoint(url.as_str()).map_err(|e| e.to_string())?;
    Ok((url.as_str().trim_end_matches('/').into(), code))
}
pub(super) async fn open(
    State(api): State<Api>,
    axum::Extension(caller): axum::Extension<Caller>,
) -> Response {
    if let Some(r) = management(&caller) {
        return r;
    }
    let identity = match local_identity(&api).await {
        Ok(v) => v,
        Err(e) => return failure(StatusCode::SERVICE_UNAVAILABLE, e),
    };
    let remote = api.remote.view();
    if remote["status"] != "connected" || remote["port_stale"] == true {
        return failure(
            StatusCode::CONFLICT,
            "Remote access is not connected yet. Wait for it to connect and try again.",
        );
    }
    let Some(host) = remote["hostname"].as_str() else {
        return failure(StatusCode::CONFLICT, "Remote access has no address.");
    };
    let endpoint = format!("https://{host}");
    if let Err(e) = validate_endpoint(&endpoint) {
        return failure(StatusCode::BAD_REQUEST, e);
    }
    let (code, expires_at) = api.app.computer.grants.open_for(&identity.backend);
    Json(serde_json::json!({"code":code,"expires_at":expires_at,"link":format!("{endpoint}/#computer-pair={code}")})).into_response()
}
pub(super) async fn close(
    State(api): State<Api>,
    axum::Extension(caller): axum::Extension<Caller>,
    Json(body): Json<CloseCode>,
) -> Response {
    if let Some(r) = management(&caller) {
        return r;
    }
    api.app.computer.grants.close(&body.code);
    StatusCode::NO_CONTENT.into_response()
}
pub(super) async fn inspect(
    State(_api): State<Api>,
    axum::Extension(caller): axum::Extension<Caller>,
    Json(body): Json<PairLink>,
) -> Response {
    if let Some(r) = management(&caller) {
        return r;
    }
    let (endpoint, _) = match parse_link(&body.link) {
        Ok(v) => v,
        Err(e) => return failure(StatusCode::BAD_REQUEST, e),
    };
    let client = match client() {
        Ok(c) => c,
        Err(e) => return failure(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    match client
        .get(format!("{endpoint}/api/computer/identity"))
        .send()
        .await
    {
        Ok(r) => match json_response::<Identity>(r).await {
            Ok(v) => Json(v).into_response(),
            Err(e) => failure(StatusCode::BAD_GATEWAY, e),
        },
        Err(e) => failure(StatusCode::BAD_GATEWAY, e.without_url()),
    }
}
pub(super) async fn claim(State(api): State<ControlApi>, Json(body): Json<Claim>) -> Response {
    let backend = match api.computer.grants.validate(&body.code) {
        Ok(backend) => backend,
        Err(e) => return failure(StatusCode::UNAUTHORIZED, e),
    };
    let identity = if backend == "native" {
        native_identity(&api.machine_id)
    } else {
        match api.computer.local_cua(Delivery::Auto, None).await {
            Ok(b) => match cua_identity(b, &api.machine_id).await {
                Ok(v) => v,
                Err(e) => return failure(StatusCode::SERVICE_UNAVAILABLE, e),
            },
            Err(e) => return failure(StatusCode::SERVICE_UNAVAILABLE, e),
        }
    };
    match api.computer.grants.claim(&body.code, &body.name) {
        Ok((g, token)) => Json(Claimed {
            identity,
            device_id: g.id,
            token,
        })
        .into_response(),
        Err(e) => failure(StatusCode::UNAUTHORIZED, e),
    }
}
pub(super) async fn connect(
    State(api): State<Api>,
    axum::Extension(caller): axum::Extension<Caller>,
    Json(body): Json<PairLink>,
) -> Response {
    if let Some(r) = management(&caller) {
        return r;
    }
    let _changes = api.app.computer.changes.lock().await;
    let (endpoint, code) = match parse_link(&body.link) {
        Ok(v) => v,
        Err(e) => return failure(StatusCode::BAD_REQUEST, e),
    };
    let mut connections = match load_connections() {
        Ok(v) => v,
        Err(e) => return failure(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    if connections.iter().any(|c| c.endpoint == endpoint) {
        return failure(StatusCode::CONFLICT, "This computer is already paired.");
    }
    let client = match client() {
        Ok(c) => c,
        Err(e) => return failure(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    let response = client
        .post(format!("{endpoint}/api/computer/pairing/claim"))
        .json(&serde_json::json!({"code":code,"name":computer_name()}))
        .send()
        .await;
    let claimed: Claimed = match response {
        Ok(r) => match json_response(r).await {
            Ok(v) => v,
            Err(e) => return failure(StatusCode::BAD_GATEWAY, e),
        },
        Err(e) => return failure(StatusCode::BAD_GATEWAY, e.without_url()),
    };
    let c = Connection {
        id: uuid::Uuid::new_v4().simple().to_string(),
        name: claimed.identity.name,
        platform: claimed.identity.platform,
        endpoint: endpoint.clone(),
        device_id: claimed.device_id,
        token: claimed.token,
    };
    if c.device_id.len() != 32
        || !c.device_id.bytes().all(|b| b.is_ascii_hexdigit())
        || c.token.len() != 64
        || !c.token.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return failure(StatusCode::BAD_GATEWAY, "Invalid pairing response.");
    }
    let view = ComputerView {
        id: c.id.clone(),
        name: c.name.clone(),
        platform: c.platform.clone(),
    };
    connections.push(c.clone());
    if let Err(e) = save_connections(&connections) {
        let _ = client
            .post(format!("{endpoint}/api/computer/access/revoke"))
            .bearer_auth(&c.token)
            .send()
            .await;
        return failure(StatusCode::INTERNAL_SERVER_ERROR, e);
    }
    Json(view).into_response()
}
