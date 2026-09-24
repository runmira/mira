//! Remote servers: streamable HTTP with the full OAuth sign-in (401 →
//! discovery → dynamic registration → PKCE → browser redirect → token →
//! reconnect → sign out), static-header auth, and the older SSE
//! transport.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Form, Json, Router};
use futures::stream::Stream;
use mira_mcp::{McpManager, McpOptions, Scope, ServerSpec, Status, Transport};
use serde_json::{json, Value};

fn options(dir: &Path) -> McpOptions {
    McpOptions {
        connect_timeout: Duration::from_secs(10),
        tool_timeout: Duration::from_secs(10),
        max_output_chars: 10_000,
        credentials: dir.join("credentials.json"),
        state_file: dir.join("state.json"),
        log_dir: dir.join("logs"),
    }
}

#[derive(Clone, Default)]
struct Fake {
    base: Arc<Mutex<String>>,
    /// code → PKCE challenge, to check the verifier at /token.
    codes: Arc<Mutex<HashMap<String, String>>>,
    tokens_issued: Arc<Mutex<u32>>,
    /// How `/register` behaves: `ok`, `none` (not advertised) or
    /// `forbidden` (only approved apps, like Figma's).
    registration: Arc<Mutex<&'static str>>,
    /// For SSE: messages to push down the open stream.
    sse_tx: Arc<Mutex<Option<tokio::sync::mpsc::UnboundedSender<String>>>>,
}

impl Fake {
    fn base(&self) -> String {
        self.base.lock().unwrap().clone()
    }
}

/// Minimal MCP request handling shared by both transports.
fn handle_rpc(msg: &Value) -> Option<Value> {
    let id = msg.get("id")?.clone();
    let result = match msg["method"].as_str()? {
        "initialize" => json!({
            "protocolVersion": msg["params"]["protocolVersion"],
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "remote", "version": "9.9.9"}
        }),
        "tools/list" => json!({"tools": [{
            "name": "whoami", "description": "Who am I", "inputSchema": {"type": "object"}
        }]}),
        "tools/call" => json!({"content": [{"type": "text", "text": "you are signed in"}]}),
        "ping" => json!({}),
        _ => {
            return Some(
                json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "nope"}}),
            )
        }
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

fn authorized(h: &HeaderMap) -> bool {
    let bearer = h
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("Bearer tok-"));
    let key = h.get("x-api-key").and_then(|v| v.to_str().ok()) == Some("secret");
    bearer || key
}

async fn mcp_post(State(f): State<Fake>, h: HeaderMap, Json(msg): Json<Value>) -> Response {
    if !authorized(&h) {
        let challenge = format!(
            "Bearer resource_metadata=\"{}/.well-known/oauth-protected-resource\"",
            f.base()
        );
        return (StatusCode::UNAUTHORIZED, [("www-authenticate", challenge)]).into_response();
    }
    match handle_rpc(&msg) {
        Some(reply) => (
            [
                ("mcp-session-id", "s1"),
                ("content-type", "application/json"),
            ],
            Json(reply),
        )
            .into_response(),
        None => StatusCode::ACCEPTED.into_response(),
    }
}

async fn protected_resource(State(f): State<Fake>) -> Json<Value> {
    Json(json!({"resource": format!("{}/mcp", f.base()), "authorization_servers": [f.base()]}))
}

async fn auth_server(State(f): State<Fake>) -> Json<Value> {
    let b = f.base();
    Json(json!({
        "issuer": b,
        "authorization_endpoint": format!("{b}/authorize"),
        "token_endpoint": format!("{b}/token"),
        "registration_endpoint": (*f.registration.lock().unwrap() != "none").then(|| format!("{b}/register")),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"]
    }))
}

async fn register(State(f): State<Fake>, Json(body): Json<Value>) -> (StatusCode, Json<Value>) {
    if *f.registration.lock().unwrap() == "forbidden" {
        return (StatusCode::FORBIDDEN, Json(json!({"error": "forbidden"})));
    }
    (
        StatusCode::CREATED,
        Json(json!({
            "client_id": "client-1",
            "redirect_uris": body["redirect_uris"],
            "client_name": body["client_name"],
            "token_endpoint_auth_method": "none"
        })),
    )
}

async fn authorize(State(f): State<Fake>, Query(q): Query<HashMap<String, String>>) -> Redirect {
    assert_eq!(q.get("client_id").map(String::as_str), Some("client-1"));
    assert_eq!(
        q.get("code_challenge_method").map(String::as_str),
        Some("S256")
    );
    f.codes
        .lock()
        .unwrap()
        .insert("code-1".into(), q["code_challenge"].clone());
    let redirect = format!("{}?code=code-1&state={}", q["redirect_uri"], q["state"]);
    Redirect::to(&redirect)
}

async fn token(State(f): State<Fake>, Form(form): Form<HashMap<String, String>>) -> Response {
    use sha2::Digest;
    if form.get("grant_type").map(String::as_str) == Some("authorization_code") {
        let challenge = f.codes.lock().unwrap().remove(&form["code"]);
        let verifier = form.get("code_verifier").cloned().unwrap_or_default();
        let digest = sha2::Sha256::digest(verifier.as_bytes());
        let computed = base64_url(&digest);
        if challenge.as_deref() != Some(computed.as_str()) {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "invalid_grant"})),
            )
                .into_response();
        }
    }
    let mut n = f.tokens_issued.lock().unwrap();
    *n += 1;
    Json(json!({
        "access_token": format!("tok-{n}"),
        "token_type": "Bearer",
        "expires_in": 3600,
        "refresh_token": "refresh-1"
    }))
    .into_response()
}

fn base64_url(bytes: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..(chunk.len() + 1) {
            out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out
}

async fn sse_open(
    State(f): State<Fake>,
    h: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>>, StatusCode> {
    if h.get("x-api-key").and_then(|v| v.to_str().ok()) != Some("secret") {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    *f.sse_tx.lock().unwrap() = Some(tx);
    let stream = async_stream(move |yield_| async move {
        yield_(
            Event::default()
                .event("endpoint")
                .data("/messages?session=1"),
        );
        while let Some(msg) = rx.recv().await {
            yield_(Event::default().event("message").data(msg));
        }
    });
    Ok(Sse::new(stream))
}

/// A tiny generator: `body` pushes events through the callback.
fn async_stream<F, Fut>(body: F) -> impl Stream<Item = Result<Event, std::convert::Infallible>>
where
    F: FnOnce(Box<dyn Fn(Event) + Send>) -> Fut,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Event>();
    let fut = body(Box::new(move |e| {
        let _ = tx.send(e);
    }));
    tokio::spawn(fut);
    futures::stream::unfold(
        rx,
        |mut rx| async move { rx.recv().await.map(|e| (Ok(e), rx)) },
    )
}

async fn sse_post(State(f): State<Fake>, Json(msg): Json<Value>) -> StatusCode {
    if let Some(reply) = handle_rpc(&msg) {
        if let Some(tx) = f.sse_tx.lock().unwrap().as_ref() {
            let _ = tx.send(reply.to_string());
        }
    }
    StatusCode::ACCEPTED
}

async fn start() -> (Fake, String) {
    let fake = Fake::default();
    *fake.registration.lock().unwrap() = "ok";
    let app = Router::new()
        .route(
            "/mcp",
            post(mcp_post).get(|| async { StatusCode::METHOD_NOT_ALLOWED }),
        )
        .route(
            "/.well-known/oauth-protected-resource",
            get(protected_resource),
        )
        .route(
            "/.well-known/oauth-protected-resource/mcp",
            get(protected_resource),
        )
        .route("/.well-known/oauth-authorization-server", get(auth_server))
        .route("/register", post(register))
        .route("/authorize", get(authorize))
        .route("/token", post(token))
        .route("/sse", get(sse_open))
        .route("/messages", post(sse_post))
        .with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    *fake.base.lock().unwrap() = base.clone();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (fake, base)
}

fn remote(name: &str, transport: Transport) -> ServerSpec {
    ServerSpec {
        name: name.into(),
        scope: Scope::User,
        transport,
        source: None,
        plugin_root: None,
    }
}

async fn wait_for(mgr: &McpManager, name: &str, want: Status) {
    for _ in 0..200 {
        if mgr.server(name).is_some_and(|s| s.status == want) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!(
        "{name}: wanted {want:?}, got {:?}",
        mgr.server(name).map(|s| s.status)
    );
}

#[tokio::test]
async fn oauth_sign_in_end_to_end() {
    let (fake, base) = start().await;
    let dir = tempfile::tempdir().unwrap();
    let mgr = McpManager::new(options(dir.path()));
    let url = format!("{base}/mcp");
    mgr.apply(
        vec![remote(
            "remote",
            Transport::Http {
                url: url.clone(),
                headers: BTreeMap::new(),
                oauth: None,
            },
        )],
        None,
    );
    wait_for(&mgr, "remote", Status::NeedsAuth).await;
    assert!(mgr.server("remote").unwrap().can_sign_in);

    // The "browser": follow the authorization URL, which redirects to
    // the loopback listener.
    mgr.sign_in_with_loopback("remote", |auth_url| {
        let auth_url = auth_url.to_owned();
        tokio::spawn(async move {
            let _ = reqwest::get(&auth_url).await;
        });
    })
    .await
    .expect("sign-in");
    wait_for(&mgr, "remote", Status::Connected).await;
    let view = mgr.server("remote").unwrap();
    assert!(view.signed_in);
    assert_eq!(view.server_version.as_deref(), Some("9.9.9"));
    let out = mgr.call_tool("remote", "whoami", None).await.unwrap();
    assert_eq!(
        serde_json::to_value(&out.content).unwrap()[0]["text"],
        "you are signed in"
    );
    assert_eq!(*fake.tokens_issued.lock().unwrap(), 1);

    // The token is stored per URL: a fresh manager connects without
    // signing in again.
    let mgr2 = McpManager::new(options(dir.path()));
    mgr2.apply(
        vec![remote(
            "same-server",
            Transport::Http {
                url: url.clone(),
                headers: BTreeMap::new(),
                oauth: None,
            },
        )],
        None,
    );
    wait_for(&mgr2, "same-server", Status::Connected).await;
    mgr2.shutdown();

    // Signing out forgets it.
    mgr.sign_out("remote").await.unwrap();
    wait_for(&mgr, "remote", Status::NeedsAuth).await;
    let creds = std::fs::read_to_string(dir.path().join("credentials.json")).unwrap();
    assert!(!creds.contains("client-1"), "{creds}");
    mgr.shutdown();
}

#[tokio::test]
async fn static_headers_and_sse() {
    let (_fake, base) = start().await;
    let dir = tempfile::tempdir().unwrap();
    let mgr = McpManager::new(options(dir.path()));
    let key: BTreeMap<String, String> = [(
        "X-Api-Key".to_owned(),
        "${MIRA_TEST_KEY_UNSET:-secret}".to_owned(),
    )]
    .into();
    mgr.apply(
        vec![
            remote(
                "keyed",
                Transport::Http {
                    url: format!("{base}/mcp"),
                    headers: key.clone(),
                    oauth: None,
                },
            ),
            remote(
                "legacy",
                Transport::Sse {
                    url: format!("{base}/sse"),
                    headers: key,
                    oauth: None,
                },
            ),
            remote(
                "legacy-no-key",
                Transport::Sse {
                    url: format!("{base}/sse"),
                    headers: BTreeMap::new(),
                    oauth: None,
                },
            ),
        ],
        None,
    );
    mgr.wait_settled(Duration::from_secs(10)).await;
    assert_eq!(mgr.server("keyed").unwrap().status, Status::Connected);
    assert_eq!(
        mgr.server("legacy").unwrap().status,
        Status::Connected,
        "{:?}",
        mgr.server("legacy")
    );
    assert_eq!(
        mgr.server("legacy-no-key").unwrap().status,
        Status::NeedsAuth
    );
    let out = mgr.call_tool("legacy", "whoami", None).await.unwrap();
    assert!(!out.content.is_empty());
    mgr.shutdown();
}

/// A plugin header like `Bearer ${GITHUB_PERSONAL_ACCESS_TOKEN}` with the
/// variable unset: the header is left out, the server asks for sign-in,
/// and when sign-in isn't possible the message says to add the token.
#[tokio::test]
async fn unset_token_header_falls_back_to_sign_in() {
    let (fake, base) = start().await;
    let dir = tempfile::tempdir().unwrap();
    let mgr = McpManager::new(options(dir.path()));
    let headers: BTreeMap<String, String> = [(
        "Authorization".to_owned(),
        "Bearer ${MIRA_TEST_GH_TOKEN_UNSET}".to_owned(),
    )]
    .into();
    mgr.apply(
        vec![remote(
            "gh",
            Transport::Http {
                url: format!("{base}/mcp"),
                headers,
                oauth: None,
            },
        )],
        None,
    );
    wait_for(&mgr, "gh", Status::NeedsAuth).await;
    assert_eq!(mgr.server("gh").unwrap().missing_vars, ["MIRA_TEST_GH_TOKEN_UNSET"]);

    *fake.registration.lock().unwrap() = "none";
    let err = mgr
        .begin_sign_in("gh", "http://127.0.0.1:1/callback")
        .await
        .unwrap_err();
    assert!(err.contains("doesn't let apps register"), "{err}");
    assert!(err.contains("MIRA_TEST_GH_TOKEN_UNSET"), "{err}");

    *fake.registration.lock().unwrap() = "forbidden";
    let err = mgr
        .begin_sign_in("gh", "http://127.0.0.1:1/callback")
        .await
        .unwrap_err();
    assert!(err.contains("only lets apps it has approved"), "{err}");

    // Pasting the token (the server accepts any `Bearer tok-…`) connects.
    mgr.set_variable("MIRA_TEST_GH_TOKEN_UNSET", Some("tok-pasted"))
        .unwrap();
    wait_for(&mgr, "gh", Status::Connected).await;
    assert!(mgr.server("gh").unwrap().missing_vars.is_empty());
    mgr.shutdown();
}
