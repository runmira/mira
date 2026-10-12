use super::*;
use axum::{
    body::{to_bytes, Body},
    http::Request,
};
use mira_computer::{remote::ControlRequest, ComputerAction, ComputerController};
use tower::ServiceExt;
fn fixture() -> (
    Arc<runtime::Runtime>,
    Arc<mira_computer::backend::mock::MockBackend>,
    Router,
) {
    let backend = Arc::new(mira_computer::backend::mock::MockBackend::new(
        (320, 200),
        (320, 200),
    ));
    let computer = runtime::Runtime::ephemeral(backend.clone());
    let routes = control::routes(ControlApi {
        computer: computer.clone(),
        machine_id: "test-machine".into(),
    })
    .route("/api/settings", get(|| async { "private" }))
    .layer(axum::middleware::from_fn_with_state(
        Arc::new(devices::Devices::ephemeral()),
        devices::guard,
    ));
    (computer, backend, routes)
}
fn request(path: &str, token: Option<&str>, body: serde_json::Value) -> Request<Body> {
    let mut builder = Request::post(path)
        .header("host", "remote.example")
        .header("content-type", "application/json");
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    builder.body(Body::from(body.to_string())).unwrap()
}
#[test]
fn pairing_links_require_encryption_and_cannot_redirect_or_embed_credentials() {
    assert!(pairing::parse_link("https://machine.example/#computer-pair=ABCDEF1234").is_ok());
    for link in [
        "http://192.168.1.20/#computer-pair=ABCDEF1234",
        "https://user:secret@machine.example/#computer-pair=ABCDEF1234",
        "https://machine.example/path#computer-pair=ABCDEF1234",
        "https://machine.example/?redirect=evil#computer-pair=ABCDEF1234",
    ] {
        assert!(pairing::parse_link(link).is_err(), "{link}");
    }
}
#[tokio::test]
async fn computer_tokens_cannot_access_settings_and_revocation_stops_future_input() {
    let (computer, backend, routes) = fixture();
    let (code, _) = computer.grants.open_for("native");
    let response = routes
        .clone()
        .oneshot(request(
            "/api/computer/pairing/claim",
            None,
            serde_json::json!({"code":code,"name":"Laptop"}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let claimed: pairing::Claimed =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    let response = routes
        .clone()
        .oneshot(request(
            "/api/settings",
            Some(&claimed.token),
            serde_json::json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let action = serde_json::to_value(ControlRequest {
        action: ComputerAction::Type("hello".into()),
        delivery: Delivery::Foreground,
        window: None,
        run_id: "test".into(),
    })
    .unwrap();
    let response = routes
        .clone()
        .oneshot(request(
            "/api/computer/control",
            Some(&claimed.token),
            action.clone(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(!backend.events().is_empty());
    computer.revoke(&claimed.device_id).await.unwrap();
    backend.reset();
    let response = routes
        .oneshot(request(
            "/api/computer/control",
            Some(&claimed.token),
            action,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(backend.events().is_empty());
}
#[tokio::test]
async fn native_pairing_cannot_silently_switch_to_background_or_accept_a_window() {
    let (computer, backend, routes) = fixture();
    let (code, _) = computer.grants.open_for("native");
    let (_, token) = computer.grants.claim(&code, "Laptop").unwrap();
    let action = serde_json::to_value(ControlRequest {
        action: ComputerAction::Type("hello".into()),
        delivery: Delivery::Background,
        window: None,
        run_id: "test".into(),
    })
    .unwrap();
    let response = routes
        .oneshot(request("/api/computer/control", Some(&token), action))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(backend.events().is_empty());
}
#[tokio::test]
async fn remote_controller_runs_against_a_scoped_gateway_without_real_desktop_input() {
    let (computer, backend, routes) = fixture();
    let (code, _) = computer.grants.open_for("native");
    let (device, token) = computer.grants.claim(&code, "Laptop").unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, routes.into_make_service())
            .await
            .unwrap();
    });
    let connection = Connection {
        id: "test-machine".into(),
        name: "Test computer".into(),
        platform: "mock".into(),
        endpoint: format!("http://{address}"),
        device_id: device.id,
        token,
    };
    let remote =
        mira_computer::remote::RemoteComputer::new(connection, Delivery::Foreground, None).unwrap();
    let screenshot = remote.execute(&ComputerAction::Screenshot).await.unwrap();
    assert_eq!(screenshot.screenshot.unwrap().width, 320);
    remote
        .execute(&ComputerAction::Type("fixture input".into()))
        .await
        .unwrap();
    assert!(backend.events().iter().any(|event|matches!(event,mira_computer::backend::mock::Event::Type(text)if text=="fixture input")));
    server.abort();
}
