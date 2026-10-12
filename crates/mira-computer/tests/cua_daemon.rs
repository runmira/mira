//! Wire-level tests for the cua backend.
//!
//! These run the real `CuaBackend` against a stand-in `cua-spacesd` served
//! over real gRPC, so the tests cover protobuf encoding, the service paths,
//! metadata and the coordinate translation — everything except the guest's
//! own behaviour. Point `MIRA_CUA_TEST_ENDPOINT` at a live daemon to run the
//! same assertions against it; see `proto/README.md`.

use std::sync::{Arc, Mutex};

use mira_computer::backend::cua::{CuaBackend, CuaOptions, Delivery};
use mira_computer::{
    Computer, ComputerAction, ComputerOptions, MouseButton, Point, ScrollDirection,
};
use tonic::transport::Server;
use tonic::{Request, Response, Status};

use mira_computer::cua_proto as pb;
// The backend's operations are trait methods, so bring the trait into scope
// once here rather than in every test.
use mira_computer::ComputerBackend as _;

/// One RPC as it arrived, so a test can assert on what actually went over the
/// wire rather than on what we meant to send.
#[derive(Clone, Debug)]
struct Call(String);

#[derive(Default)]
struct State {
    calls: Mutex<Vec<Call>>,
    /// Primary display bounds, in global logical points.
    primary: Mutex<pb::Display>,
    displays: Mutex<Vec<pb::Display>>,
    cursor: Mutex<pb::Point>,
    png: Mutex<Vec<u8>>,
    background_input: Mutex<bool>,
    /// Make `ListDisplays` answer empty while `GetCapabilities` still
    /// reports the display, the way cua-spacesd 0.5.3 does on macOS.
    list_displays_empty: Mutex<bool>,
    authorization: Mutex<Option<String>>,
    /// Set when a request arrived without the expected bearer token.
    bad_auth: Mutex<Vec<String>>,
    expect_token: Mutex<Option<String>>,
}

impl State {
    fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    fn call_strings(&self) -> Vec<String> {
        self.calls().into_iter().map(|c| c.0).collect()
    }

    /// Record a call, and check the bearer token if one is expected.
    fn record(&self, rpc: &str, detail: String, md: &tonic::metadata::MetadataMap) {
        let auth = md
            .get("authorization")
            .map(|v| v.to_str().unwrap_or_default().to_owned());
        *self.authorization.lock().unwrap() = auth.clone();
        if let Some(want) = self.expect_token.lock().unwrap().clone() {
            if auth.as_deref() != Some(want.as_str()) {
                self.bad_auth.lock().unwrap().push(rpc.to_owned());
            }
        }
        self.calls
            .lock()
            .unwrap()
            .push(Call(format!("{rpc} {detail}")));
    }
}

struct Fake {
    state: Arc<State>,
}

#[tonic::async_trait]
impl pb::computer_service_server::ComputerService for Fake {
    async fn screenshot(
        &self,
        req: Request<pb::ScreenshotRequest>,
    ) -> Result<Response<pb::ScreenshotResponse>, Status> {
        let source = match req.get_ref().source.as_ref() {
            Some(pb::screenshot_request::Source::DisplayId(d)) => format!("display={d}"),
            Some(pb::screenshot_request::Source::Window(_)) => "window".into(),
            None => "default".into(),
        };
        let md = req.metadata().clone();
        self.state.record("Screenshot", source, &md);
        Ok(Response::new(pb::ScreenshotResponse {
            image: self.state.png.lock().unwrap().clone(),
            format: pb::ImageFormat::Png as i32,
            image_size: Some(pb::PixelSize {
                width: 2880,
                height: 1800,
            }),
            native_size: Some(pb::PixelSize {
                width: 2880,
                height: 1800,
            }),
            scale: 2.0,
            screenshot_id: "shot-1".into(),
            ..Default::default()
        }))
    }

    async fn pointer(
        &self,
        req: Request<pb::PointerRequest>,
    ) -> Result<Response<pb::PointerResponse>, Status> {
        let md = req.metadata().clone();
        let r = req.into_inner();
        let detail = summarize_pointer(&r);
        self.state.record("Pointer", detail, &md);
        Ok(Response::new(pb::PointerResponse::default()))
    }

    async fn keyboard(
        &self,
        req: Request<pb::KeyboardRequest>,
    ) -> Result<Response<pb::KeyboardResponse>, Status> {
        let md = req.metadata().clone();
        let r = req.into_inner();
        let detail = summarize_keyboard(&r);
        self.state.record("Keyboard", detail, &md);
        Ok(Response::new(pb::KeyboardResponse::default()))
    }

    async fn get_clipboard(
        &self,
        _: Request<pb::GetClipboardRequest>,
    ) -> Result<Response<pb::GetClipboardResponse>, Status> {
        Ok(Response::new(pb::GetClipboardResponse::default()))
    }

    async fn set_clipboard(
        &self,
        _: Request<pb::SetClipboardRequest>,
    ) -> Result<Response<pb::SetClipboardResponse>, Status> {
        Ok(Response::new(pb::SetClipboardResponse::default()))
    }

    async fn get_cursor_position(
        &self,
        req: Request<pb::GetCursorPositionRequest>,
    ) -> Result<Response<pb::GetCursorPositionResponse>, Status> {
        let md = req.metadata().clone();
        self.state.record("GetCursorPosition", String::new(), &md);
        Ok(Response::new(pb::GetCursorPositionResponse {
            position: Some(*self.state.cursor.lock().unwrap()),
            display_id: "1".into(),
        }))
    }

    async fn list_displays(
        &self,
        req: Request<pb::ListDisplaysRequest>,
    ) -> Result<Response<pb::ListDisplaysResponse>, Status> {
        self.state
            .record("ListDisplays", String::new(), req.metadata());
        let displays = if *self.state.list_displays_empty.lock().unwrap() {
            Vec::new()
        } else {
            self.state.displays.lock().unwrap().clone()
        };
        Ok(Response::new(pb::ListDisplaysResponse { displays }))
    }
}

#[tonic::async_trait]
impl pb::system_service_server::SystemService for Fake {
    async fn get_capabilities(
        &self,
        req: Request<pb::GetCapabilitiesRequest>,
    ) -> Result<Response<pb::GetCapabilitiesResponse>, Status> {
        let md = req.metadata().clone();
        self.state.record("GetCapabilities", String::new(), &md);
        // Read once: two guards in one struct literal would deadlock.
        let background = *self.state.background_input.lock().unwrap();
        Ok(Response::new(pb::GetCapabilitiesResponse {
            version: "0.5.3".into(),
            protocol_version: 1,
            protocol_revision: 8,
            hostname: "test-space".into(),
            displays: self.state.displays.lock().unwrap().clone(),
            features: vec![pb::Feature {
                name: "background_input".into(),
                supported: background,
                limitation: if background {
                    String::new()
                } else {
                    "no compositor support on this guest".into()
                },
                attributes: Default::default(),
            }],
            ..Default::default()
        }))
    }

    async fn init(
        &self,
        _: Request<pb::InitRequest>,
    ) -> Result<Response<pb::InitResponse>, Status> {
        unimplemented!("not needed by these tests")
    }

    async fn health(
        &self,
        _: Request<pb::HealthRequest>,
    ) -> Result<Response<pb::HealthResponse>, Status> {
        Ok(Response::new(pb::HealthResponse::default()))
    }

    async fn metrics(
        &self,
        _: Request<pb::MetricsRequest>,
    ) -> Result<Response<pb::MetricsResponse>, Status> {
        Ok(Response::new(pb::MetricsResponse::default()))
    }

    async fn shutdown(
        &self,
        _: Request<pb::ShutdownRequest>,
    ) -> Result<Response<pb::ShutdownResponse>, Status> {
        Ok(Response::new(pb::ShutdownResponse::default()))
    }

    type DiagnoseStream = std::pin::Pin<
        Box<dyn tokio_stream::Stream<Item = Result<pb::DiagnoseResponse, Status>> + Send>,
    >;

    async fn diagnose(
        &self,
        _: Request<pb::DiagnoseRequest>,
    ) -> Result<Response<Self::DiagnoseStream>, Status> {
        // Nothing emits on this stream; the backend never calls it.
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        Ok(Response::new(Box::pin(
            tokio_stream::wrappers::ReceiverStream::new(rx),
        )))
    }

    async fn diagnose_once(
        &self,
        _: Request<pb::DiagnoseOnceRequest>,
    ) -> Result<Response<pb::DiagnoseOnceResponse>, Status> {
        Ok(Response::new(pb::DiagnoseOnceResponse::default()))
    }

    async fn create_viewer_ticket(
        &self,
        _: Request<pb::CreateViewerTicketRequest>,
    ) -> Result<Response<pb::CreateViewerTicketResponse>, Status> {
        Ok(Response::new(pb::CreateViewerTicketResponse::default()))
    }

    async fn attach_relay(
        &self,
        _: Request<pb::AttachRelayRequest>,
    ) -> Result<Response<pb::AttachRelayResponse>, Status> {
        Ok(Response::new(pb::AttachRelayResponse::default()))
    }

    async fn detach_relay(
        &self,
        _: Request<pb::DetachRelayRequest>,
    ) -> Result<Response<pb::DetachRelayResponse>, Status> {
        Ok(Response::new(pb::DetachRelayResponse::default()))
    }
}

/// A `KeyInput` as something an assertion can read.
fn key_name(k: &Option<pb::KeyInput>) -> String {
    match k.as_ref().and_then(|k| k.key.as_ref()) {
        Some(pb::key_input::Key::Named(n)) => pb::Key::try_from(*n)
            .map(|k| format!("{k:?}"))
            .unwrap_or(format!("?{n}")),
        Some(pb::key_input::Key::Character(c)) => format!("char({c:?})"),
        None => "none".into(),
    }
}

fn keys_name(keys: &[pb::KeyInput]) -> String {
    format!(
        "[{}]",
        keys.iter()
            .map(|k| key_name(&Some(k.clone())))
            .collect::<Vec<_>>()
            .join(",")
    )
}

/// What arrived on the wire, in a form a test can assert on.
///
/// Built from the typed request rather than from its generated `Debug`, so a
/// prost version bump cannot quietly rewrite these expectations.
fn summarize_pointer(r: &pb::PointerRequest) -> String {
    use pb::pointer_request::Action as A;
    let t = r.target.clone().unwrap_or_default();
    let head = format!(
        "space={} delivery={} display={}",
        t.space, t.delivery, t.display_id
    );
    match &r.action {
        Some(A::Move(m)) => format!("{head} move(pos={})", key_pos(&m.position)),
        Some(A::Click(c)) => format!(
            "{head} click(pos={} button={} count={})",
            key_pos(&c.position),
            c.button,
            c.count
        ),
        Some(A::Down(d)) => {
            format!(
                "{head} down(pos={} button={})",
                key_pos(&d.position),
                d.button
            )
        }
        Some(A::Up(u)) => {
            format!(
                "{head} up(pos={} button={})",
                key_pos(&u.position),
                u.button
            )
        }
        Some(A::Drag(d)) => {
            format!(
                "{head} drag(from={} to={})",
                key_pos(&d.from),
                key_pos(&d.to)
            )
        }
        Some(A::Scroll(s)) => format!(
            "{head} scroll(pos={} dx={} dy={} unit={})",
            key_pos(&s.position),
            s.delta_x,
            s.delta_y,
            s.unit
        ),
        None => format!("{head} no-action"),
    }
}

fn summarize_keyboard(r: &pb::KeyboardRequest) -> String {
    use pb::keyboard_request::Action as A;
    let t = r.target.clone().unwrap_or_default();
    let head = format!("delivery={}", t.delivery);
    match &r.action {
        Some(A::Type(k)) => format!("{head} type(text={:?} mode={})", k.text, k.mode),
        Some(A::Press(p)) => format!(
            "{head} press(key={} modifiers={:?} repeat={})",
            key_name(&p.key),
            p.modifiers,
            p.repeat
        ),
        Some(A::Hotkey(h)) => format!("{head} hotkey(keys={})", keys_name(&h.keys)),
        Some(A::Down(k)) => format!("{head} down(key={})", key_name(&k.key)),
        Some(A::Up(k)) => format!("{head} up(key={})", key_name(&k.key)),
        None => format!("{head} no-action"),
    }
}

/// A `Point` as `x,y`, or `none`.
fn key_pos(p: &Option<pb::Point>) -> String {
    match p {
        Some(p) => format!("{},{}", p.x, p.y),
        None => "none".into(),
    }
}

/// A stand-in guest: one Retina primary display plus a second display to its
/// right, a cursor parked on the second one, and a solid-colour PNG that
/// decodes to the primary's native resolution.
async fn fake_space() -> (String, Arc<State>) {
    let state = Arc::new(State::default());

    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        2880,
        1800,
        image::Rgba([10, 20, 30, 255]),
    ))
    .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
    .expect("encoding the fixture PNG");
    *state.png.lock().unwrap() = png;

    let primary = pb::Display {
        id: "1".into(),
        name: "Built-in".into(),
        primary: true,
        bounds: Some(pb::Rect {
            x: 0.0,
            y: 0.0,
            width: 1440.0,
            height: 900.0,
        }),
        native_size: Some(pb::PixelSize {
            width: 2880,
            height: 1800,
        }),
        scale_factor: 2.0,
        ..Default::default()
    };
    let secondary = pb::Display {
        id: "2".into(),
        name: "External".into(),
        primary: false,
        bounds: Some(pb::Rect {
            x: 1440.0,
            y: 0.0,
            width: 2048.0,
            height: 858.0,
        }),
        native_size: Some(pb::PixelSize {
            width: 2048,
            height: 858,
        }),
        scale_factor: 1.0,
        ..Default::default()
    };
    *state.primary.lock().unwrap() = primary.clone();
    *state.displays.lock().unwrap() = vec![primary, secondary];
    *state.cursor.lock().unwrap() = pb::Point {
        x: 1544.0,
        y: 268.0,
    };
    *state.background_input.lock().unwrap() = true;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let served = state.clone();
    tokio::spawn(async move {
        Server::builder()
            .add_service(pb::computer_service_server::ComputerServiceServer::new(
                Fake {
                    state: served.clone(),
                },
            ))
            .add_service(pb::system_service_server::SystemServiceServer::new(Fake {
                state: served,
            }))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });

    (format!("http://{addr}"), state)
}

async fn backend(endpoint: &str) -> CuaBackend {
    CuaBackend::connect(&CuaOptions {
        endpoint: endpoint.to_owned(),
        ..Default::default()
    })
    .await
    .expect("connecting to the stand-in daemon")
}

#[tokio::test]
async fn screen_size_is_the_primary_display_and_ignores_the_others() {
    let (endpoint, _state) = fake_space().await;
    let b = backend(&endpoint).await;
    assert_eq!(
        mira_computer::ComputerBackend::screen_size(&b)
            .await
            .unwrap(),
        (1440, 900)
    );
}

/// cua-spacesd 0.5.3 on macOS answers `ListDisplays` with an empty list
/// while `GetCapabilities` still knows the display. Caught against a real
/// daemon; this keeps the fallback honest.
#[tokio::test]
async fn geometry_falls_back_to_capabilities_when_list_displays_is_empty() {
    let (endpoint, state) = fake_space().await;
    *state.list_displays_empty.lock().unwrap() = true;
    let b = backend(&endpoint).await;

    assert_eq!(b.screen_size().await.unwrap(), (1440, 900));
    assert!(
        state
            .call_strings()
            .iter()
            .any(|c| c.starts_with("GetCapabilities")),
        "should have asked the probe: {:?}",
        state.call_strings()
    );
}

/// With neither call reporting a display, the error has to name the guest
/// side as the thing to fix.
#[tokio::test]
async fn a_guest_with_no_display_says_so() {
    let (endpoint, state) = fake_space().await;
    *state.list_displays_empty.lock().unwrap() = true;
    *state.displays.lock().unwrap() = Vec::new();
    let b = backend(&endpoint).await;

    let err = b.screen_size().await.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("no display to drive"), "{msg}");
    assert!(msg.contains("locked") && msg.contains("headless"), "{msg}");
}

#[tokio::test]
async fn capture_decodes_the_png_the_guest_sent() {
    let (endpoint, _state) = fake_space().await;
    let b = backend(&endpoint).await;
    let img = b.capture().await.unwrap();
    // Native resolution, not the logical size: `Scale` needs the real pixels
    // to map model space onto a Retina display.
    assert_eq!((img.width(), img.height()), (2880, 1800));
    assert_eq!(img.get_pixel(0, 0).0, [10, 20, 30, 255]);
}

#[tokio::test]
async fn a_move_and_a_click_carry_the_same_point_in_global_space() {
    let (endpoint, state) = fake_space().await;
    let b = backend(&endpoint).await;
    b.mouse_move(Point { x: 100, y: 200 }).await.unwrap();
    b.button(MouseButton::Right, mira_computer::ButtonAction::Click(2))
        .await
        .unwrap();

    let calls = state.call_strings();
    let move_call = calls.iter().find(|c| c.contains(" move(")).unwrap();
    // The point travels in the contract's global space (space=1), pinned to
    // the primary display.
    assert_eq!(
        move_call,
        "Pointer space=1 delivery=1 display=primary move(pos=100,200)"
    );

    let click_call = calls.iter().find(|c| c.contains(" click(")).unwrap();
    // The click names the point explicitly rather than "wherever the pointer
    // is", because background delivery never moves the real pointer.
    assert_eq!(
        click_call,
        "Pointer space=1 delivery=1 display=primary click(pos=100,200 button=2 count=2)"
    );
}

#[tokio::test]
async fn scroll_direction_and_magnitude_reach_the_guest() {
    let (endpoint, state) = fake_space().await;
    let b = backend(&endpoint).await;
    b.mouse_move(Point { x: 10, y: 10 }).await.unwrap();
    b.scroll(ScrollDirection::Down, 3).await.unwrap();

    let calls = state.call_strings();
    let call = calls.iter().find(|c| c.contains(" scroll(")).unwrap();
    // Positive Y scrolls content down; `amount` is wheel notches and
    // `unit=1` is the contract's ScrollUnit::Line.
    assert_eq!(
        call,
        "Pointer space=1 delivery=1 display=primary scroll(pos=10,10 dx=0 dy=3 unit=1)"
    );
}

#[tokio::test]
async fn typing_uses_keystrokes_so_apps_see_each_key() {
    let (endpoint, state) = fake_space().await;
    let b = backend(&endpoint).await;
    b.type_text("hi").await.unwrap();

    let calls = state.call_strings();
    let call = calls
        .iter()
        .find(|c| c.contains(" type("))
        .expect("a KeyboardType call");
    // mode=1 is TextEntryMode::Keystrokes.
    assert_eq!(call, "Keyboard delivery=1 type(text=\"hi\" mode=1)");
}

#[tokio::test]
async fn a_chord_is_a_hotkey_but_a_lone_key_is_a_press() {
    let (endpoint, state) = fake_space().await;
    let b = backend(&endpoint).await;

    b.key(
        &mira_computer::KeyChord::parse("ctrl+c").unwrap(),
        mira_computer::KeyAction::Press,
    )
    .await
    .unwrap();
    let calls = state.call_strings();
    // Modifiers go down first, the main key last.
    assert_eq!(
        calls.last().unwrap(),
        "Keyboard delivery=1 hotkey(keys=[ControlLeft,char(\"c\")])"
    );

    state.calls.lock().unwrap().clear();
    b.key(
        &mira_computer::KeyChord::parse("Return").unwrap(),
        mira_computer::KeyAction::Press,
    )
    .await
    .unwrap();
    assert_eq!(
        state.call_strings().last().unwrap(),
        "Keyboard delivery=1 press(key=Enter modifiers=[] repeat=0)"
    );
}

#[tokio::test]
async fn hold_key_presses_in_order_and_releases_in_reverse() {
    let (endpoint, state) = fake_space().await;
    let b = backend(&endpoint).await;
    let chord = mira_computer::KeyChord::parse("ctrl+shift+t").unwrap();

    b.key(&chord, mira_computer::KeyAction::Down).await.unwrap();
    b.key(&chord, mira_computer::KeyAction::Up).await.unwrap();

    let calls = state.call_strings();
    let keyboard: Vec<&String> = calls
        .iter()
        .filter(|c| c.starts_with("Keyboard "))
        .collect();
    assert_eq!(
        keyboard,
        vec![
            // `t` is a `Char`, and characters go out as characters so the
            // guest's own layout picks the key.
            "Keyboard delivery=1 down(key=ControlLeft)",
            "Keyboard delivery=1 down(key=ShiftLeft)",
            "Keyboard delivery=1 down(key=char(\"t\"))",
            // Reverse order on the way up, or the chord would release its
            // modifiers before the key they modify.
            "Keyboard delivery=1 up(key=char(\"t\"))",
            "Keyboard delivery=1 up(key=ShiftLeft)",
            "Keyboard delivery=1 up(key=ControlLeft)",
        ]
    );
}

#[tokio::test]
async fn a_cursor_on_another_display_comes_back_inside_the_screenshot() {
    let (endpoint, _state) = fake_space().await;
    let b = backend(&endpoint).await;
    // The fixture parks the real cursor at global (1544, 268), which is on
    // the second display — past the primary's right edge.
    let at = b.cursor_position().await.unwrap();
    assert!(at.x < 1440 && at.y < 900, "{at:?}");
    assert_eq!(at, Point { x: 1439, y: 268 });
}

#[tokio::test]
async fn the_bearer_token_is_sent_on_every_call() {
    let (endpoint, state) = fake_space().await;
    *state.expect_token.lock().unwrap() = Some("Bearer s3cret".into());
    let b = CuaBackend::connect(&CuaOptions {
        endpoint,
        token: Some("s3cret".into()),
        ..Default::default()
    })
    .await
    .unwrap();

    b.screen_size().await.unwrap();
    b.capture().await.unwrap();
    b.mouse_move(Point { x: 1, y: 1 }).await.unwrap();

    assert!(state.bad_auth.lock().unwrap().is_empty());
    assert_eq!(
        state.authorization.lock().unwrap().as_deref(),
        Some("Bearer s3cret")
    );
}

#[tokio::test]
async fn delivery_is_stated_on_every_input_call() {
    let (endpoint, state) = fake_space().await;
    let b = CuaBackend::connect(&CuaOptions {
        endpoint,
        delivery: Delivery::Background,
        ..Default::default()
    })
    .await
    .unwrap();

    b.mouse_move(Point { x: 5, y: 5 }).await.unwrap();
    b.type_text("x").await.unwrap();
    b.scroll(ScrollDirection::Up, 1).await.unwrap();

    for call in state.call_strings() {
        if call.starts_with("Pointer ") || call.starts_with("Keyboard ") {
            // DELIVERY_BACKGROUND, stated rather than assumed.
            assert!(call.contains("delivery=2"), "{call}");
        }
    }
}

#[tokio::test]
async fn capabilities_report_whether_the_pointer_can_stay_put() {
    let (endpoint, state) = fake_space().await;
    let b = backend(&endpoint).await;
    let caps = b.capabilities().await.unwrap();
    assert_eq!(caps.version, "0.5.3");
    assert_eq!((caps.protocol_version, caps.protocol_revision), (1, 8));
    assert!(caps.background_input);
    assert!(caps.background_input_limitation.is_empty());

    // Now take the capability away and read the guest's own explanation —
    // `mira doctor` surfaces that string rather than a bare "no".
    *state.background_input.lock().unwrap() = false;
    let caps = b.capabilities().await.unwrap();
    assert!(!caps.background_input);
    assert_eq!(
        caps.background_input_limitation,
        "no compositor support on this guest"
    );
}

/// The whole path, end to end: a model-space click on a Retina display has to
/// land on the right logical point, and every step in between has to survive
/// the round trip through `Computer`.
#[tokio::test]
async fn a_model_click_on_a_retina_display_lands_where_the_model_looked() {
    let (endpoint, state) = fake_space().await;
    let b = backend(&endpoint).await;

    // Model space is the resized screenshot; `Computer` owns the mapping.
    let computer = Computer::new(
        Arc::new(b),
        ComputerOptions {
            settle: std::time::Duration::ZERO,
            ..Default::default()
        },
    );
    let (mw, mh) = computer.display_size().await.unwrap();
    assert_eq!((mw, mh), computer_opts_model_size());

    let action = ComputerAction::from_args(&serde_json::json!({
        "action": "left_click",
        "coordinate": [mw / 2, mh / 2],
    }))
    .unwrap();
    computer.execute(&action).await.unwrap();

    let calls = state.call_strings();
    let move_call = calls.iter().find(|c| c.contains(" move(")).unwrap();
    // Half the primary display is logical (720, 449) whatever the model
    // space measured.
    let coords = move_call
        .rsplit_once("pos=")
        .unwrap()
        .1
        .trim_end_matches(')')
        .split(',')
        .map(|v| v.parse::<f64>().unwrap())
        .collect::<Vec<_>>();
    assert!((coords[0] - 720.0).abs() <= 2.0, "{move_call}");
    assert!((coords[1] - 449.0).abs() <= 2.0, "{move_call}");

    // And the screenshot the model sees is pinned to the same display, so
    // the coordinates it reads back describe what it was shown.
    assert!(
        calls.iter().any(|c| c == "Screenshot display=primary"),
        "{:?}",
        calls
    );
}

/// `ImageLimits::default().fit(1440, 900)`, spelled out so a change to the
/// limits shows up here as a failure to explain rather than a silent drift.
fn computer_opts_model_size() -> (u32, u32) {
    (1356, 847)
}

#[tokio::test]
async fn a_bad_endpoint_fails_at_connect_not_at_the_first_click() {
    let err = CuaBackend::connect(&CuaOptions {
        // Port 1 on loopback: nothing listens there.
        endpoint: "http://127.0.0.1:1".into(),
        ..Default::default()
    })
    .await
    .unwrap_err();
    assert!(
        matches!(err, mira_computer::ComputerError::Remote(_)),
        "{err}"
    );
    assert!(err.to_string().contains("computer.cua.endpoint"), "{err}");

    let err = CuaBackend::connect(&CuaOptions {
        endpoint: "not a url".into(),
        ..Default::default()
    })
    .await
    .unwrap_err();
    assert!(
        matches!(err, mira_computer::ComputerError::InvalidArgs(_)),
        "{err}"
    );
}

/// Read-only smoke test against a real `cua-spacesd`.
///
/// Ignored by default because it needs a daemon:
/// `MIRA_CUA_TEST_ENDPOINT=http://127.0.0.1:3211 cargo test -p
/// mira-computer --test cua_daemon -- --ignored`.
///
/// Deliberately stops short of any input: a real guest would move the
/// pointer and type wherever it was sent. What this covers is everything
/// the stand-in daemon cannot vouch for — that a real daemon answers on
/// these paths, that its display geometry parses, and that a real
/// screenshot decodes.
#[tokio::test]
#[ignore = "needs a live cua-spacesd; set MIRA_CUA_TEST_ENDPOINT"]
async fn live_readonly_smoke() {
    let endpoint = std::env::var("MIRA_CUA_TEST_ENDPOINT")
        .expect("set MIRA_CUA_TEST_ENDPOINT to the daemon's base URL");
    let b = CuaBackend::connect(&CuaOptions {
        endpoint,
        token: std::env::var("MIRA_CUA_TEST_TOKEN").ok(),
        ..Default::default()
    })
    .await
    .expect("connecting to the live daemon");

    let caps = b.capabilities().await.expect("GetCapabilities");
    println!(
        "cua-spacesd {} · cua.env.v1 {}·{} · background_input={}{}",
        caps.version,
        caps.protocol_version,
        caps.protocol_revision,
        caps.background_input,
        if caps.background_input_limitation.is_empty() {
            String::new()
        } else {
            format!(" ({})", caps.background_input_limitation)
        }
    );

    let (w, h) = b.screen_size().await.expect("ListDisplays");
    println!("primary display: {w}x{h} logical");
    assert!(
        w > 0 && h > 0,
        "a real guest has a non-empty primary display"
    );

    let img = b.capture().await.expect("Screenshot");
    println!("screenshot: {}x{}", img.width(), img.height());

    // The capture must be at least as large as the logical size: anything
    // smaller means the mapping between model space and input space would
    // land clicks on the wrong pixel.
    assert!(
        img.width() as u32 >= w && img.height() as u32 >= h,
        "capture {}x{} is smaller than the {w}x{h} input space",
        img.width(),
        img.height()
    );

    let at = b.cursor_position().await.expect("GetCursorPosition");
    println!("cursor: {at:?} (model space)");
    assert!(
        (0..w as i32).contains(&at.x) && (0..h as i32).contains(&at.y),
        "cursor {at:?} outside the {w}x{h} input space"
    );
}
