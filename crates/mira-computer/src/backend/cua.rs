//! A [`ComputerBackend`] that drives a *remote* desktop through
//! [cua-spacesd](https://github.com/trycua/cua) instead of the machine Mira
//! runs on.
//!
//! The local `macos` and `x11` backends drive the OS through helper binaries,
//! which means they take the user's pointer and focus for every keystroke.
//! cua-spacesd exposes the same primitive operations over gRPC and can
//! deliver them *in the background* — the guest gets the input, the pointer
//! in front of the user never moves — which is what makes it possible to let
//! an agent work while you keep using your computer. It also means an agent
//! can drive a machine Mira has no seat on: another box on the LAN, or a
//! sandbox behind a cua relay.
//!
//! # Coordinates
//!
//! [`ComputerBackend`] speaks *input space*: the logical points the guest OS
//! takes for mouse events. cua-spacesd uses the same unit and calls the space
//! `COORDINATE_SPACE_SCREEN`, with `Display.bounds` in it. Mira screenshots
//! and drives the **primary** display, so the backend pins every request to
//! that display: it reads the primary display's bounds from `ListDisplays`,
//! translates Mira's primary-relative points into the global space the
//! contract defines, and translates the cursor position back.
//!
//! HiDPI needs nothing special here. cua-spacesd reports the capture's scale
//! and Mira's [`crate::imaging`] already does the model-space mapping, so a
//! Retina guest behaves exactly like a 1× one.
//!
//! # Delivery
//!
//! [`Delivery`] is the one setting that changes what the *user* sees, and it
//! defaults to [`Delivery::Auto`] — prefer background, fall back to
//! foreground when the guest cannot do better. See [`CuaOptions::delivery`].

use std::time::{Duration, Instant};

use async_trait::async_trait;
use image::RgbaImage;
use tokio::sync::{Mutex, RwLock};
use tonic::metadata::{Ascii, MetadataValue};
use tonic::transport::{Channel, ClientTlsConfig, Endpoint};
use tonic::Request;

use crate::action::{MouseButton, Point, ScrollDirection};
use crate::backend::{ButtonAction, ComputerBackend, KeyAction};
use crate::cua_proto as pb;
use crate::keys::{Key, KeyChord, Modifier};
use crate::ComputerError;

/// The alias cua-spacesd accepts anywhere a display id is expected, so the
/// backend never has to care what the opaque id happens to be.
const PRIMARY: &str = "primary";

/// tonic's default ceiling on one message is 4 MiB. A full-resolution PNG of
/// a 2880×1800 Retina desktop measures ~6 MB, so the default rejects the
/// first screenshot and every action that follows it.
const MAX_MESSAGE_BYTES: usize = 32 * 1024 * 1024;

/// How long [`Geometry`] is reused before we ask the guest again. Resolutions
/// change, monitors get unplugged, and a long agent session outlives all of
/// it; a stale `Scale` is a click that lands on the wrong pixel.
const GEOMETRY_TTL: Duration = Duration::from_secs(30);

/// How long to wait for the daemon to answer before giving up. Generous,
/// because a cold guest may still be starting its desktop session.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const RPC_TIMEOUT: Duration = Duration::from_secs(20);

/// The port `cua-spacesd` serves the contract on.
const DEFAULT_PORT: u16 = 3211;

/// How synthetic input reaches the guest.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Delivery {
    /// Prefer background delivery and fall back to foreground when the guest
    /// cannot avoid activating the target. The safe default: on a guest that
    /// supports it the user's pointer stays put, and on one that does not
    /// the action still happens.
    #[default]
    Auto,
    /// Never move the pointer or change focus. Fails instead of falling back,
    /// so an agent cannot quietly start stealing focus halfway through a
    /// session. The honest choice for a machine you are working on.
    Background,
    /// Always deliver through the global input stream, moving the real
    /// pointer and focus — the same behaviour as the local backends.
    Foreground,
}

impl Delivery {
    fn as_i32(self) -> i32 {
        match self {
            Delivery::Auto => pb::Delivery::Auto as i32,
            Delivery::Background => pb::Delivery::Background as i32,
            Delivery::Foreground => pb::Delivery::Foreground as i32,
        }
    }
}

impl std::str::FromStr for Delivery {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Delivery::Auto),
            "background" => Ok(Delivery::Background),
            "foreground" => Ok(Delivery::Foreground),
            other => Err(format!(
                "`{other}` is not a delivery mode; expected auto, background or foreground"
            )),
        }
    }
}

impl std::fmt::Display for Delivery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Delivery::Auto => "auto",
            Delivery::Background => "background",
            Delivery::Foreground => "foreground",
        })
    }
}

/// Everything needed to reach one guest.
#[derive(Clone, PartialEq, Eq)]
pub struct CuaOptions {
    /// Base URL of cua-spacesd, including the scheme. A daemon on this
    /// machine is `http://127.0.0.1:3211`; one behind a cua relay is an
    /// an HTTPS origin terminating TLS for the daemon. URL prefixes are not supported.
    pub endpoint: String,
    /// Bearer token. Mandatory for any non-loopback bind — a daemon will not
    /// listen on a routable address without one. Prefer
    /// [`CuaOptions::token_from_env`] over a literal.
    pub token: Option<String>,
    /// See [`Delivery`].
    pub delivery: Delivery,
    pub window: Option<TargetWindow>,
    pub rpc_timeout: Duration,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TargetWindow {
    pub id: String,
    #[serde(with = "epoch_json")]
    pub epoch: u64,
    pub title: String,
    pub app: String,
}
mod epoch_json {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(value: &u64, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&value.to_string())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Value {
            Text(String),
            Number(u64),
        }
        match Value::deserialize(d)? {
            Value::Text(v) => v.parse().map_err(serde::de::Error::custom),
            Value::Number(v) => Ok(v),
        }
    }
}
impl TargetWindow {
    fn reference(&self) -> pb::WindowRef {
        pb::WindowRef {
            id: self.id.clone(),
            epoch: self.epoch,
        }
    }
}
impl std::fmt::Debug for CuaOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CuaOptions")
            .field("endpoint", &self.endpoint)
            .field("authenticated", &self.token.is_some())
            .field("delivery", &self.delivery)
            .field("window", &self.window)
            .finish()
    }
}

/// Accept only encrypted remote connections or direct loopback HTTP.
pub fn validate_endpoint(endpoint: &str) -> Result<url::Url, ComputerError> {
    let url = url::Url::parse(endpoint)
        .map_err(|_| ComputerError::InvalidArgs("Invalid computer address".into()))?;
    let loopback = url.host_str().is_some_and(|h| {
        h == "localhost"
            || h.trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
        || !matches!(url.scheme(), "http" | "https")
        || (url.scheme() == "http" && !loopback)
    {
        return Err(ComputerError::InvalidArgs("Use an HTTPS computer address. HTTP is only supported on loopback; credentials, paths, queries and fragments are not allowed.".into()));
    }
    Ok(url)
}

impl Default for CuaOptions {
    fn default() -> Self {
        Self {
            endpoint: format!("http://127.0.0.1:{DEFAULT_PORT}"),
            token: None,
            delivery: Delivery::default(),
            window: None,
            rpc_timeout: RPC_TIMEOUT,
        }
    }
}

impl CuaOptions {
    /// Fill `token` from `var` when the options do not already carry one, so
    /// a per-repo config can name the variable without holding the secret.
    pub fn token_from_env(mut self, var: &str) -> Self {
        if self.token.is_none() {
            self.token = std::env::var(var).ok().filter(|t| !t.is_empty());
        }
        self
    }

    /// The header value sent on every call, if a token is configured.
    fn auth_header(&self) -> Result<Option<MetadataValue<Ascii>>, ComputerError> {
        match &self.token {
            None => Ok(None),
            Some(t) => MetadataValue::try_from(format!("Bearer {t}"))
                .map(|mut value| {
                    value.set_sensitive(true);
                    Some(value)
                })
                .map_err(|_| {
                    ComputerError::InvalidArgs("cua: token is not valid HTTP metadata".into())
                }),
        }
    }
}

/// The primary display, as Mira needs it: where it sits in the guest's global
/// logical space, how big it is, and how many physical pixels back it.
#[derive(Clone, Copy, Debug)]
struct Geometry {
    /// Global logical origin, added to Mira's primary-relative points.
    origin: Point,
    /// Logical size — Mira's input space.
    size: (u32, u32),
    /// Framebuffer size in physical pixels.
    native: (u32, u32),
    fetched: Instant,
}

impl Geometry {
    fn validated(bounds: pb::Rect, native: (u32, u32)) -> Result<Self, ComputerError> {
        if ![bounds.x, bounds.y, bounds.width, bounds.height]
            .iter()
            .all(|value| value.is_finite())
            || bounds.width < 1.0
            || bounds.height < 1.0
            || bounds.width > 32768.0
            || bounds.height > 32768.0
            || bounds.x.abs() > 1_000_000.0
            || bounds.y.abs() > 1_000_000.0
        {
            return Err(ComputerError::Remote(
                "The guest reported invalid capture geometry. Input was refused.".into(),
            ));
        }
        Ok(Self {
            origin: Point {
                x: bounds.x.round() as i32,
                y: bounds.y.round() as i32,
            },
            size: (bounds.width.round() as u32, bounds.height.round() as u32),
            native,
            fetched: Instant::now(),
        })
    }
    /// Mira's primary-relative point → the contract's global logical space.
    fn to_global(self, p: Point) -> Point {
        Point {
            x: p.x.saturating_add(self.origin.x),
            y: p.y.saturating_add(self.origin.y),
        }
    }

    /// The contract's global logical point → Mira's primary-relative space.
    ///
    /// Clamped, because the cursor can be on a display we do not screenshot.
    /// Returning the raw global coordinate would land outside the model
    /// space and read as a nonsensical position; clamping keeps
    /// `Scale::to_model` in bounds. `debug!` says when it happened.
    fn to_primary(self, p: Point) -> Point {
        let rel = Point {
            x: p.x.saturating_sub(self.origin.x),
            y: p.y.saturating_sub(self.origin.y),
        };
        let inside =
            rel.x >= 0 && rel.y >= 0 && rel.x < self.size.0 as i32 && rel.y < self.size.1 as i32;
        if !inside {
            tracing::debug!(
                cursor = ?p,
                primary = ?self.size,
                "cua: cursor is outside the primary display; reporting it clamped to the edge"
            );
        }
        Point {
            x: rel.x.clamp(0, self.size.0.saturating_sub(1) as i32),
            y: rel.y.clamp(0, self.size.1.saturating_sub(1) as i32),
        }
    }
}

/// A `ComputerBackend` backed by cua-spacesd.
#[derive(Debug)]
pub struct CuaBackend {
    computer: pb::computer_service_client::ComputerServiceClient<Channel>,
    system: pb::system_service_client::SystemServiceClient<Channel>,
    windows: pb::windows_service_client::WindowsServiceClient<Channel>,
    driver: pb::driver_service_client::DriverServiceClient<Channel>,
    cursor_session: String,
    cursor_available: Mutex<Option<bool>>,
    window: Option<TargetWindow>,
    report: Mutex<Option<pb::DeliveryReport>>,
    geometry_changed: std::sync::atomic::AtomicBool,
    input_blocked: std::sync::atomic::AtomicBool,
    auth: Option<MetadataValue<Ascii>>,
    delivery: i32,
    rpc_timeout: Duration,
    geometry: RwLock<Option<Geometry>>,
    /// The last point Mira selected, relative to the capture source.
    ///
    /// `ComputerBackend::button` takes no coordinates — `Computer` moves and
    /// then clicks — so without this a click would have to mean "wherever the
    /// pointer happens to be". That is fine when delivery is foreground, but
    /// background delivery never moves the real pointer, so the click would
    /// land under it instead of at the place the agent looked.
    pointer: Mutex<Option<Point>>,
}

/// What [`CuaBackend::capabilities`] found out about a guest, for `mira
/// doctor` and for deciding whether background delivery can work at all.
#[derive(Clone, Debug)]
pub struct Capabilities {
    /// cua-spacesd release, e.g. `0.5.3`.
    pub version: String,
    /// Major contract version. Always 1 for `cua.env.v1`.
    pub protocol_version: u32,
    /// Additive revision within the major version.
    pub protocol_revision: u32,
    /// Guest hostname, when it reports one.
    pub hostname: String,
    pub os: String,
    /// Whether the guest can take input without moving the user's pointer.
    pub background_input: bool,
    /// Why [`Capabilities::background_input`] is degraded or absent, in the
    /// guest's own words. Empty when fully supported.
    pub background_input_limitation: String,
}

impl CuaBackend {
    /// Connect to a guest and check that it is really there.
    ///
    /// Fails fast on a bad endpoint or an unreachable daemon rather than
    /// leaving `mira` to start and fail on the model's first click.
    pub async fn connect(opts: &CuaOptions) -> Result<Self, ComputerError> {
        let url = validate_endpoint(&opts.endpoint)?;
        let mut endpoint = Endpoint::from_shared(opts.endpoint.clone()).map_err(|e| {
            ComputerError::InvalidArgs(format!("cua endpoint `{}`: {e}", opts.endpoint))
        })?;
        if url.scheme() == "https" {
            endpoint = endpoint
                .tls_config(ClientTlsConfig::new().with_webpki_roots())
                .map_err(|_| {
                    ComputerError::InvalidArgs("Could not configure computer TLS".into())
                })?;
        }
        let channel = endpoint
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(opts.rpc_timeout)
            .connect()
            .await
            .map_err(|e| {
                ComputerError::Remote(format!(
                    "cannot reach cua-spacesd at {}: {e}. Is the daemon running, and does \
                     `computer.cua.endpoint` point at it?",
                    opts.endpoint
                ))
            })?;

        tracing::debug!(endpoint = %opts.endpoint, delivery = ?opts.delivery, "cua: connected");
        Ok(Self {
            computer: pb::computer_service_client::ComputerServiceClient::new(channel.clone())
                .max_decoding_message_size(MAX_MESSAGE_BYTES)
                .max_encoding_message_size(MAX_MESSAGE_BYTES),
            windows: pb::windows_service_client::WindowsServiceClient::new(channel.clone()),
            driver: pb::driver_service_client::DriverServiceClient::new(channel.clone()),
            cursor_session: format!("Mira-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]),
            cursor_available: Mutex::new(None),
            window: opts.window.clone(),
            report: Mutex::new(None),
            geometry_changed: std::sync::atomic::AtomicBool::new(false),
            input_blocked: std::sync::atomic::AtomicBool::new(false),
            system: pb::system_service_client::SystemServiceClient::new(channel)
                .max_decoding_message_size(MAX_MESSAGE_BYTES)
                .max_encoding_message_size(MAX_MESSAGE_BYTES),
            auth: opts.auth_header()?,
            delivery: opts.delivery.as_i32(),
            rpc_timeout: opts.rpc_timeout,
            geometry: RwLock::new(None),
            pointer: Mutex::new(None),
        })
    }

    /// Ask the guest what it can do.
    ///
    /// Worth calling before trusting `Delivery::Background`: a guest that
    /// cannot do background input should say so at startup, not on the first
    /// click that steals the user's cursor.
    pub async fn capabilities(&self) -> Result<Capabilities, ComputerError> {
        let mut client = self.system.clone();
        let caps = client
            .get_capabilities(self.request(pb::GetCapabilitiesRequest {}))
            .await
            .map_err(|e| call_error("GetCapabilities", e))?
            .into_inner();

        // The contract says every enum's zero value means "not set" and that
        // unknown values must be tolerated, so match on the name rather than
        // trusting the discriminant.
        let background = caps
            .features
            .iter()
            .find(|f| f.name == "background_input")
            .map(|f| (f.supported, f.limitation.clone()));

        Ok(Capabilities {
            version: caps.version,
            protocol_version: caps.protocol_version,
            protocol_revision: caps.protocol_revision,
            hostname: caps.hostname,
            os: caps
                .os
                .map(|os| {
                    if os.pretty_name.is_empty() {
                        os.name
                    } else {
                        os.pretty_name
                    }
                })
                .unwrap_or_default(),
            background_input: background.as_ref().is_some_and(|(ok, _)| *ok),
            background_input_limitation: background.map(|(_, l)| l).unwrap_or_default(),
        })
    }

    fn request<T>(&self, message: T) -> Request<T> {
        let mut req = Request::new(message);
        req.set_timeout(self.rpc_timeout);
        if let Some(auth) = &self.auth {
            req.metadata_mut().insert("authorization", auth.clone());
        }
        req
    }

    /// Target for input: the primary display, in the contract's global space.
    fn target(&self) -> pb::InputTarget {
        pb::InputTarget {
            window: self.window.as_ref().map(TargetWindow::reference),
            display_id: PRIMARY.to_owned(),
            delivery: self.delivery,
            space: pb::CoordinateSpace::Screen as i32,
            ..Default::default()
        }
    }

    /// The Driver window-scoped cursor is visual only, distinct from OS input.
    async fn show_agent_cursor(&self, point: (f64, f64)) {
        let mut available = self.cursor_available.lock().await;
        if *available == Some(false) {
            return;
        }
        let mut calls = Vec::new();
        if available.is_none() {
            calls.push((
                "set_agent_cursor_enabled",
                serde_json::json!({"session":self.cursor_session,"enabled":true}),
            ));
        }
        calls.push(("move_cursor", serde_json::json!({"session":self.cursor_session,"scope":"window","x":point.0,"y":point.1})));
        for (name, arguments) in calls {
            let mut request = self.request(pb::CallToolRequest {
                name: name.into(),
                arguments_json: arguments.to_string(),
                timeout: Some(prost_types::Duration {
                    seconds: 1,
                    nanos: 0,
                }),
            });
            request.set_timeout(Duration::from_secs(1));
            let result = tokio::time::timeout(
                Duration::from_secs(1),
                self.driver.clone().call_tool(request),
            )
            .await;
            if !matches!(result, Ok(Ok(ref response)) if !response.get_ref().is_error) {
                *available = Some(false);
                tracing::warn!(
                    "CUA agent cursor unavailable; desktop control continues without an overlay"
                );
                return;
            }
        }
        *available = Some(true);
    }

    async fn pointer(&self, action: pb::pointer_request::Action) -> Result<(), ComputerError> {
        self.check_input_target()?;
        let position = match &action {
            pb::pointer_request::Action::Click(c) => c.position.as_ref(),
            pb::pointer_request::Action::Move(m) => m.position.as_ref(),
            pb::pointer_request::Action::Drag(d) => d.to.as_ref(),
            pb::pointer_request::Action::Scroll(s) => s.position.as_ref(),
            _ => None,
        };
        if let Some(position) = position {
            self.show_agent_cursor((position.x, position.y)).await;
        }
        let mut client = self.computer.clone();
        let response = client
            .pointer(self.request(pb::PointerRequest {
                target: Some(self.target()),
                action: Some(action),
            }))
            .await
            .map_err(|e| self.input_error("Pointer", e))?
            .into_inner();
        self.record_report(response.report).await
    }

    async fn keyboard(&self, action: pb::keyboard_request::Action) -> Result<(), ComputerError> {
        self.check_input_target()?;
        let mut client = self.computer.clone();
        let response = client
            .keyboard(self.request(pb::KeyboardRequest {
                // `KeyboardRequest` ignores `space` and any points; only
                // `delivery` is meaningful here.
                target: Some(self.target()),
                action: Some(action),
            }))
            .await
            .map_err(|e| self.input_error("Keyboard", e))?
            .into_inner();
        self.record_report(response.report).await
    }

    fn input_error(&self, op: &str, error: tonic::Status) -> ComputerError {
        if matches!(
            error.code(),
            tonic::Code::DeadlineExceeded | tonic::Code::Cancelled | tonic::Code::Unavailable
        ) {
            self.input_blocked
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
        call_error(op, error)
    }
    fn check_input_target(&self) -> Result<(), ComputerError> {
        if self.input_blocked.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(ComputerError::Remote("Computer input is paused after an unconfirmed delivery. Reconnect in Computer settings before continuing.".into()));
        }
        if self.delivery == Delivery::Background.as_i32() && self.window.is_none() {
            return Err(ComputerError::InvalidArgs(
                "Select an application window before using background input.".into(),
            ));
        }
        Ok(())
    }
    async fn record_report(&self, report: Option<pb::DeliveryReport>) -> Result<(), ComputerError> {
        if report.as_ref().is_none_or(|r| {
            r.delivery != pb::Delivery::Background as i32
                && r.delivery != pb::Delivery::Foreground as i32
        }) {
            self.input_blocked
                .store(true, std::sync::atomic::Ordering::SeqCst);
            return Err(ComputerError::Remote("The guest did not confirm input delivery. Input has been stopped; inspect the target computer before reconnecting.".into()));
        }
        if self.delivery == Delivery::Background.as_i32()
            && report.as_ref().is_none_or(|r| {
                r.delivery != pb::Delivery::Background as i32 || r.focus_changed || r.pointer_moved
            })
        {
            self.input_blocked
                .store(true, std::sync::atomic::Ordering::SeqCst);
            return Err(ComputerError::Remote("The guest did not confirm background-only delivery. Input has been stopped; check the target computer.".into()));
        }
        if let Some(report) = report {
            let mut previous = self.report.lock().await;
            if let Some(old) = previous.as_mut() {
                old.pointer_moved |= report.pointer_moved;
                old.focus_changed |= report.focus_changed;
                if report.delivery == pb::Delivery::Foreground as i32 {
                    old.delivery = report.delivery;
                }
            } else {
                *previous = Some(report);
            }
        }
        Ok(())
    }
    pub async fn list_windows(&self) -> Result<Vec<TargetWindow>, ComputerError> {
        let response = self
            .windows
            .clone()
            .list_windows(self.request(pb::ListWindowsRequest {
                filter: Some(pb::WindowFilter {
                    on_screen_only: true,
                    ..Default::default()
                }),
            }))
            .await
            .map_err(|e| call_error("ListWindows", e))?
            .into_inner();
        Ok(response
            .windows
            .into_iter()
            .filter_map(|w| {
                let reference = w.r#ref?;
                Some(TargetWindow {
                    id: reference.id,
                    epoch: reference.epoch,
                    title: w.title,
                    app: w.app.map(|a| a.name).unwrap_or_default(),
                })
            })
            .collect())
    }

    /// The primary display's geometry, cached briefly.
    ///
    /// `Computer` asks for `screen_size()` on every screenshot, so this would
    /// otherwise be an RPC per model action.
    async fn geometry(&self) -> Result<Geometry, ComputerError> {
        {
            let cached = self.geometry.read().await;
            if let Some(g) = *cached {
                if g.fetched.elapsed() < GEOMETRY_TTL {
                    return Ok(g);
                }
            }
        }

        if let Some(window) = &self.window {
            let response = self
                .windows
                .clone()
                .get_window(self.request(pb::GetWindowRequest {
                    window: Some(window.reference()),
                }))
                .await
                .map_err(|e| call_error("GetWindow", e))?
                .into_inner();
            let bounds = response.window.and_then(|w| w.bounds).ok_or_else(|| {
                ComputerError::Remote(
                    "Selected window is unavailable. Choose another window in Computer settings."
                        .into(),
                )
            })?;
            let g = Geometry::validated(bounds, (0, 0))?;
            *self.geometry.write().await = Some(g);
            return Ok(g);
        }
        let primary = self.primary_display().await?.ok_or_else(|| {
            // A real guest genuinely reports no display: its desktop
            // session has not come up, the screen is locked, or it is
            // running headless. The fix is on that end, so say so.
            ComputerError::Remote(
                "cua: the guest has no display to drive — its desktop session may not \
                     have started, the screen may be locked, or it may be running headless"
                    .into(),
            )
        })?;
        let bounds = primary.bounds.ok_or_else(|| {
            ComputerError::Remote("cua: the primary display reports no bounds".into())
        })?;
        let native = primary
            .native_size
            .map(|s| (s.width, s.height))
            .unwrap_or((0, 0));

        let geometry = Geometry::validated(bounds, native)?;
        *self.geometry.write().await = Some(geometry);
        Ok(geometry)
    }

    /// The primary display, or `None` when the guest reports none.
    ///
    /// `ComputerService.ListDisplays` is the purpose-built call, but it is
    /// not always the one that knows: against cua-spacesd 0.5.3 on macOS it
    /// answers with an empty list while `SystemService.GetCapabilities`
    /// reports the display perfectly well. So ask the purpose-built call
    /// first and fall back to the probe rather than trusting either alone.
    async fn primary_display(&self) -> Result<Option<pb::Display>, ComputerError> {
        let mut computer = self.computer.clone();
        let listed = computer
            .list_displays(self.request(pb::ListDisplaysRequest {}))
            .await
            .map_err(|e| call_error("ListDisplays", e))?
            .into_inner();
        if let Some(d) = listed.displays.into_iter().find(|d| d.primary) {
            return Ok(Some(d));
        }
        tracing::debug!("cua: ListDisplays has no primary display; asking GetCapabilities instead");

        let mut system = self.system.clone();
        let caps = system
            .get_capabilities(self.request(pb::GetCapabilitiesRequest {}))
            .await
            .map_err(|e| call_error("GetCapabilities", e))?
            .into_inner();
        Ok(caps.displays.into_iter().find(|d| d.primary))
    }

    async fn input_geometry(&self) -> Result<Geometry, ComputerError> {
        if self
            .geometry_changed
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Err(ComputerError::Remote(
                "The capture source was resized. Take a fresh screenshot before sending input."
                    .into(),
            ));
        }
        let previous = *self.geometry.read().await;
        if self.window.is_some() {
            *self.geometry.write().await = None;
        }
        let current = self.geometry().await?;
        if previous.is_some_and(|g| g.size != current.size) {
            self.geometry_changed
                .store(true, std::sync::atomic::Ordering::SeqCst);
            return Err(ComputerError::Remote(
                "The capture source was resized. Take a fresh screenshot before sending input."
                    .into(),
            ));
        }
        Ok(current)
    }
    async fn input_point(&self) -> Result<Option<pb::Point>, ComputerError> {
        self.check_input_target()?;
        let point = *self.pointer.lock().await;
        if self.delivery == Delivery::Background.as_i32() && point.is_none() {
            return Err(ComputerError::InvalidArgs(
                "Provide a coordinate before clicking or scrolling in the background.".into(),
            ));
        }
        let geometry = self.input_geometry().await?;
        Ok(point.map(|p| self::point(geometry.to_global(p))))
    }
}

#[async_trait]
impl ComputerBackend for CuaBackend {
    fn name(&self) -> &'static str {
        "cua"
    }

    async fn screen_size(&self) -> Result<(u32, u32), ComputerError> {
        Ok(self.geometry().await?.size)
    }

    async fn capture(&self) -> Result<RgbaImage, ComputerError> {
        let mut client = self.computer.clone();
        let shot = client
            .screenshot(self.request(pb::ScreenshotRequest {
                source: Some(match &self.window {
                    Some(window) => pb::screenshot_request::Source::Window(window.reference()),
                    None => pb::screenshot_request::Source::DisplayId(PRIMARY.to_owned()),
                }),
                format: pb::ImageFormat::Png as i32,
                max_dimension: 4096,
                // The cursor is part of what the user sees, and seeing it
                // stops a model from hunting for a pointer it already drew.
                include_cursor: true,
                ..Default::default()
            }))
            .await
            .map_err(|e| call_error("Screenshot", e))?
            .into_inner();

        if let Some(bounds) = shot.logical_bounds {
            let g = Geometry::validated(
                bounds,
                shot.native_size
                    .map(|s| (s.width, s.height))
                    .unwrap_or_default(),
            )?;
            *self.geometry.write().await = Some(g);
        }
        if shot.image.is_empty() {
            return Err(ComputerError::Remote(
                "cua: the guest returned an empty screenshot".into(),
            ));
        }
        let mut reader = image::ImageReader::with_format(
            std::io::Cursor::new(&shot.image),
            image::ImageFormat::Png,
        );
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(8192);
        limits.max_image_height = Some(8192);
        limits.max_alloc = Some(128 * 1024 * 1024);
        reader.limits(limits);
        let image = reader
            .decode()
            .map(|image| image.to_rgba8())
            .map_err(|e| ComputerError::Image(format!("cua screenshot: {e}")))?;
        self.geometry_changed
            .store(false, std::sync::atomic::Ordering::SeqCst);
        Ok(image)
    }

    async fn mouse_move(&self, p: Point) -> Result<(), ComputerError> {
        let g = self.input_geometry().await?;
        let global = g.to_global(p);
        if self.delivery == Delivery::Background.as_i32() {
            self.check_input_target()?;
            self.show_agent_cursor((global.x as f64, global.y as f64))
                .await;
            *self.pointer.lock().await = Some(p);
            *self.report.lock().await = Some(pb::DeliveryReport {
                delivery: pb::Delivery::Background as i32,
                detail: "Target point selected; the visible pointer was not moved.".into(),
                ..Default::default()
            });
            return Ok(());
        }
        self.pointer(pb::pointer_request::Action::Move(pb::PointerMove {
            position: Some(point(global)),
            // Unset jumps instantly, matching what Mira's local backends do.
            duration: None,
        }))
        .await?;
        *self.pointer.lock().await = Some(p);
        Ok(())
    }

    async fn button(&self, button: MouseButton, action: ButtonAction) -> Result<(), ComputerError> {
        if self.delivery == Delivery::Background.as_i32()
            && !matches!(action, ButtonAction::Click(_))
        {
            return Err(ComputerError::Unsupported(
                "Background input does not support separate button down/up. Use click or drag."
                    .into(),
            ));
        }
        let at = self.input_point().await?;
        let button = mouse_button(button) as i32;
        let action = match action {
            ButtonAction::Click(count) => pb::pointer_request::Action::Click(pb::PointerClick {
                position: at,
                button,
                count: count.max(1) as u32,
                modifiers: Vec::new(),
            }),
            ButtonAction::Down => pb::pointer_request::Action::Down(pb::PointerDown {
                position: at,
                button,
            }),
            ButtonAction::Up => pb::pointer_request::Action::Up(pb::PointerUp {
                position: at,
                button,
            }),
        };
        self.pointer(action).await
    }

    async fn scroll(&self, direction: ScrollDirection, amount: u32) -> Result<(), ComputerError> {
        let at = self.input_point().await?;
        // `amount` is wheel clicks, as on the local backends, and the contract
        // takes a delta in the chosen unit: positive scrolls content down and
        // right.
        let notches = amount.max(1) as f64;
        let (delta_x, delta_y) = match direction {
            ScrollDirection::Up => (0.0, -notches),
            ScrollDirection::Down => (0.0, notches),
            ScrollDirection::Left => (-notches, 0.0),
            ScrollDirection::Right => (notches, 0.0),
        };
        self.pointer(pb::pointer_request::Action::Scroll(pb::PointerScroll {
            position: at,
            delta_x,
            delta_y,
            unit: pb::ScrollUnit::Line as i32,
        }))
        .await
    }

    async fn type_text(&self, text: &str) -> Result<(), ComputerError> {
        if text.is_empty() {
            return Ok(());
        }
        // Keystrokes, not a single insert: a human types character by
        // character, and apps with per-keystroke handlers only see that.
        self.keyboard(pb::keyboard_request::Action::Type(pb::KeyboardType {
            text: text.to_owned(),
            mode: pb::TextEntryMode::Keystrokes as i32,
            delay: None,
        }))
        .await
    }

    async fn key(&self, chord: &KeyChord, action: KeyAction) -> Result<(), ComputerError> {
        if self.delivery == Delivery::Background.as_i32() && !matches!(action, KeyAction::Press) {
            return Err(ComputerError::Unsupported(
                "Background input does not support holding keys across requests. Use a key chord."
                    .into(),
            ));
        }
        // Modifiers first, main key last — the order a human presses them in.
        let keys: Vec<pb::KeyInput> = chord
            .modifiers
            .iter()
            .map(|m| key_input(&Key::Named(modifier_keysym(*m))))
            .chain(chord.key.iter().map(key_input))
            .collect::<Result<_, _>>()?;
        if keys.is_empty() {
            return Ok(());
        }

        match action {
            // One key, or a chord. A lone modifier (`hold_key shift`) goes
            // through `Press`, which taps it — down then up — which is what
            // `with_modifiers` expects of a bare modifier.
            KeyAction::Press => {
                let action = if keys.len() == 1 {
                    pb::keyboard_request::Action::Press(pb::KeyboardPress {
                        key: Some(keys.into_iter().next().expect("len is 1")),
                        modifiers: Vec::new(),
                        // 0 means one press.
                        repeat: 0,
                    })
                } else {
                    pb::keyboard_request::Action::Hotkey(pb::KeyboardHotkey { keys })
                };
                self.keyboard(action).await
            }
            // `hold_key` and `with_modifiers` hold a chord across separate
            // calls, so `Down`/`Up` have to be one key per RPC. Up goes in
            // reverse, or the chord would un-modify before releasing its key.
            KeyAction::Down => {
                for key in keys {
                    self.keyboard(pb::keyboard_request::Action::Down(pb::KeyboardDown {
                        key: Some(key),
                    }))
                    .await?;
                }
                Ok(())
            }
            KeyAction::Up => {
                for key in keys.into_iter().rev() {
                    self.keyboard(pb::keyboard_request::Action::Up(pb::KeyboardUp {
                        key: Some(key),
                    }))
                    .await?;
                }
                Ok(())
            }
        }
    }

    async fn click(
        &self,
        button: MouseButton,
        count: u8,
        modifiers: Option<&KeyChord>,
    ) -> Result<(), ComputerError> {
        let modifiers = pointer_modifiers(modifiers)?;
        self.pointer(pb::pointer_request::Action::Click(pb::PointerClick {
            position: self.input_point().await?,
            button: mouse_button(button) as i32,
            count: count.max(1) as u32,
            modifiers,
        }))
        .await
    }
    async fn drag(&self, from: Point, to: Point) -> Result<(), ComputerError> {
        let g = self.input_geometry().await?;
        self.pointer(pb::pointer_request::Action::Drag(pb::PointerDrag {
            from: Some(point(g.to_global(from))),
            to: Some(point(g.to_global(to))),
            button: pb::MouseButton::Left as i32,
            ..Default::default()
        }))
        .await?;
        *self.pointer.lock().await = Some(to);
        Ok(())
    }
    async fn scroll_modified(
        &self,
        direction: ScrollDirection,
        amount: u32,
        modifiers: Option<&KeyChord>,
    ) -> Result<(), ComputerError> {
        if modifiers.is_none() {
            return self.scroll(direction, amount).await;
        }
        if self.delivery == Delivery::Background.as_i32() {
            return Err(ComputerError::Unsupported("The guest contract does not support modified background scrolling. Use a keyboard shortcut.".into()));
        }
        let chord = modifiers.expect("checked");
        self.key(chord, KeyAction::Down).await?;
        let result = self.scroll(direction, amount).await;
        let release = self.key(chord, KeyAction::Up).await;
        result.and(release)
    }
    async fn delivery_note(&self) -> Option<String> {
        let cursor_unavailable = *self.cursor_available.lock().await == Some(false);
        self.report.lock().await.take().map(|r| {
            format!(
                "Input delivered in {}. Pointer moved: {}. Focus changed: {}.{}",
                if r.delivery == pb::Delivery::Background as i32 {
                    "the background"
                } else {
                    "the foreground"
                },
                r.pointer_moved,
                r.focus_changed,
                if cursor_unavailable {
                    " The guest does not provide a visible agent cursor."
                } else {
                    ""
                }
            )
        })
    }

    async fn cursor_position(&self) -> Result<Point, ComputerError> {
        let g = self.geometry().await?;
        let mut client = self.computer.clone();
        let at = client
            .get_cursor_position(self.request(pb::GetCursorPositionRequest {}))
            .await
            .map_err(|e| call_error("GetCursorPosition", e))?
            .into_inner();
        let p = at.position.ok_or_else(|| {
            ComputerError::Remote("cua: GetCursorPosition returned no position".into())
        })?;
        Ok(g.to_primary(Point {
            x: p.x.round() as i32,
            y: p.y.round() as i32,
        }))
    }
}

fn pointer_modifiers(chord: Option<&KeyChord>) -> Result<Vec<i32>, ComputerError> {
    let Some(chord) = chord else {
        return Ok(vec![]);
    };
    if chord.key.is_some() {
        return Err(ComputerError::InvalidArgs(
            "Click modifiers must contain only modifier keys".into(),
        ));
    }
    Ok(chord
        .modifiers
        .iter()
        .map(|m| named_key(modifier_keysym(*m)).expect("known modifier") as i32)
        .collect())
}

fn point(p: Point) -> pb::Point {
    pb::Point {
        x: p.x as f64,
        y: p.y as f64,
    }
}

fn mouse_button(b: MouseButton) -> pb::MouseButton {
    match b {
        MouseButton::Left => pb::MouseButton::Left,
        MouseButton::Right => pb::MouseButton::Right,
        MouseButton::Middle => pb::MouseButton::Middle,
    }
}

/// One Mira modifier, as the contract's [`Key`] value.
///
/// Returned as a name because `key_input` takes a `Key`, and this crate's
/// `Key::Named` is spelled in X11 keysyms.
fn modifier_keysym(m: Modifier) -> &'static str {
    match m {
        Modifier::Ctrl => "Control_L",
        Modifier::Shift => "Shift_L",
        Modifier::Alt => "Alt_L",
        Modifier::Meta => "Meta_L",
    }
}

/// One [`KeyChord`] key, as the contract's `KeyInput`.
///
/// Printable characters go through `KeyInput.character`, which the guest maps
/// through its own keyboard layout — right for `a`, `A` and `/` alike, and
/// layout-independent in the way a US-keysym would not be. The guest fails
/// with `DELIVERY_FAILED` for a character no key produces, which surfaces as
/// a clear error rather than a silent no-op.
fn key_input(key: &Key) -> Result<pb::KeyInput, ComputerError> {
    let character = |s: String| pb::KeyInput {
        key: Some(pb::key_input::Key::Character(s)),
    };
    let named = |k: pb::Key| pb::KeyInput {
        key: Some(pb::key_input::Key::Named(k as i32)),
    };

    Ok(match key {
        Key::Char(c) => character(c.to_string()),
        // `keys.rs` canonicalises `plus` to a keysym of that name, but it is
        // a character rather than a key: sending the character lets the guest
        // work out which key carries it on its own layout.
        Key::Named("plus") => character("+".into()),
        Key::Named(name) => match named_key(name) {
            Some(k) => named(k),
            None => {
                return Err(ComputerError::Unsupported(format!(
                    "cua: no equivalent for key `{name}`"
                )))
            }
        },
    })
}

/// Mira's canonical X11 keysyms to the contract's `Key`.
///
/// `keys.rs` canonicalises model-written names to these spellings, so the
/// match is total over what it can produce. `plus` is absent on purpose: it is
/// Shift+Equal on a US layout, and `KeyInput.character` gets the character
/// right on any layout instead of guessing which key carries it.
fn named_key(name: &str) -> Option<pb::Key> {
    use pb::Key as K;
    Some(match name {
        "Control_L" => K::ControlLeft,
        "Shift_L" => K::ShiftLeft,
        "Alt_L" => K::AltLeft,
        "Meta_L" => K::MetaLeft,
        "Return" => K::Enter,
        "Escape" => K::Escape,
        "Tab" => K::Tab,
        "BackSpace" => K::Backspace,
        "Delete" => K::Delete,
        "Insert" => K::Insert,
        "space" => K::Space,
        "Up" => K::ArrowUp,
        "Down" => K::ArrowDown,
        "Left" => K::ArrowLeft,
        "Right" => K::ArrowRight,
        "Home" => K::Home,
        "End" => K::End,
        "Page_Up" => K::PageUp,
        "Page_Down" => K::PageDown,
        "Caps_Lock" => K::CapsLock,
        "F1" => K::F1,
        "F2" => K::F2,
        "F3" => K::F3,
        "F4" => K::F4,
        "F5" => K::F5,
        "F6" => K::F6,
        "F7" => K::F7,
        "F8" => K::F8,
        "F9" => K::F9,
        "F10" => K::F10,
        "F11" => K::F11,
        "F12" => K::F12,
        "minus" => K::Minus,
        "equal" => K::Equal,
        "comma" => K::Comma,
        "period" => K::Period,
        "slash" => K::Slash,
        _ => return None,
    })
}

/// Turn a gRPC failure into something the model can act on.
///
/// cua-spacesd puts a machine-readable reason in the status trailer and a
/// human sentence in the message; we keep the message and add the one hint
/// that reliably fixes the most common failure.
fn call_error(op: &str, e: tonic::Status) -> ComputerError {
    let hint = match e.code() {
        tonic::Code::DeadlineExceeded | tonic::Code::Cancelled if matches!(op, "Pointer" | "Keyboard") => " — input delivery could not be confirmed. Do not repeat the action; inspect the screen before continuing",
        tonic::Code::Unauthenticated => {
            " — the daemon wants a token; set the environment variable named by `computer.cua.token_env`"
        }
        tonic::Code::PermissionDenied => {
            " — the guest denied input. On macOS that means a Screen Recording or \
             Accessibility grant; it can also mean `delivery: background` cannot reach \
             this target, which fails on purpose rather than stealing focus"
        }
        _ => "",
    };
    ComputerError::Remote(format!("cua {op} failed: {}{hint}", e.message()))
}

/// Readiness checks for the cua backend, for `mira doctor`.
pub async fn preflight(opts: &CuaOptions) -> Vec<crate::backend::Check> {
    let mut out = vec![crate::backend::Check {
        label: "cua endpoint",
        ok: true,
        detail: opts.endpoint.clone(),
    }];

    let backend = match CuaBackend::connect(opts).await {
        Ok(b) => b,
        Err(e) => {
            out.push(crate::backend::Check {
                label: "cua endpoint",
                ok: false,
                detail: e.to_string(),
            });
            return out;
        }
    };
    // `connect` already proved the daemon is there.
    out[0].detail = format!("{} (reachable)", opts.endpoint);

    match backend.capabilities().await {
        Err(e) => out.push(crate::backend::Check {
            label: "cua capabilities",
            ok: false,
            detail: e.to_string(),
        }),
        Ok(caps) => {
            let where_ = if caps.hostname.is_empty() {
                String::new()
            } else {
                format!(" on {}", caps.hostname)
            };
            out.push(crate::backend::Check {
                label: "cua daemon",
                ok: true,
                detail: format!(
                    "cua-spacesd {} · cua.env.v1 {}·{}{where_}",
                    caps.version, caps.protocol_version, caps.protocol_revision,
                ),
            });
            out.push(crate::backend::Check {
                label: "background input",
                // A guest without it still works under `Auto`; under
                // `Background` every action would fail.
                ok: caps.background_input || matches!(opts.delivery, Delivery::Auto),
                detail: if caps.background_input {
                    "supported".to_owned()
                } else if caps.background_input_limitation.is_empty() {
                    "not reported by the guest".to_owned()
                } else {
                    caps.background_input_limitation.clone()
                },
            });
            match backend.geometry().await {
                Ok(g) => out.push(crate::backend::Check {
                    label: "display",
                    ok: true,
                    detail: format!(
                        "{}x{} logical ({}x{} physical)",
                        g.size.0, g.size.1, g.native.0, g.native.1
                    ),
                }),
                Err(e) => out.push(crate::backend::Check {
                    label: "display",
                    ok: false,
                    detail: e.to_string(),
                }),
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geometry(origin: Point, size: (u32, u32)) -> Geometry {
        Geometry {
            origin,
            size,
            native: (size.0 * 2, size.1 * 2),
            fetched: Instant::now(),
        }
    }

    #[test]
    fn points_translate_through_the_global_origin() {
        // A guest whose primary display is not at the global origin, as on a
        // machine with a monitor to the left.
        let g = geometry(Point { x: -1920, y: 0 }, (1920, 1080));
        assert_eq!(
            g.to_global(Point { x: 10, y: 20 }),
            Point { x: -1910, y: 20 }
        );
        assert_eq!(
            g.to_primary(Point { x: -1910, y: 20 }),
            Point { x: 10, y: 20 }
        );
    }

    #[test]
    fn a_cursor_on_another_display_clamps_to_the_edge() {
        // Mirrors a real two-display guest: primary 1440x900 at the origin,
        // a second display to its right, and the pointer parked over there.
        let g = geometry(Point { x: 0, y: 0 }, (1440, 900));
        let p = g.to_primary(Point { x: 1544, y: 268 });
        assert!((0..1440).contains(&p.x), "{p:?}");
        assert!((0..900).contains(&p.y), "{p:?}");
        assert_eq!(p, Point { x: 1439, y: 268 });
    }

    #[test]
    fn named_keys_reach_the_contract() {
        let cases = [
            ("Return", pb::Key::Enter),
            ("Escape", pb::Key::Escape),
            ("BackSpace", pb::Key::Backspace),
            ("Page_Down", pb::Key::PageDown),
            ("F5", pb::Key::F5),
            ("slash", pb::Key::Slash),
            ("Meta_L", pb::Key::MetaLeft),
        ];
        for (name, want) in cases {
            let got = key_input(&Key::Named(name)).expect(name);
            assert_eq!(
                got.key,
                Some(pb::key_input::Key::Named(want as i32)),
                "{name}"
            );
        }
    }

    #[test]
    fn printable_characters_go_through_the_guest_layout() {
        let got = key_input(&Key::Char('A')).unwrap();
        assert_eq!(got.key, Some(pb::key_input::Key::Character("A".into())));
        let got = key_input(&Key::Char('/')).unwrap();
        assert_eq!(got.key, Some(pb::key_input::Key::Character("/".into())));
    }

    #[test]
    fn every_canonical_key_has_an_equivalent() {
        // The guard that matters: `keys.rs` grows its table, and a backend
        // that misses a new entry only fails once a model reaches for it.
        let mut missing = Vec::new();
        for name in crate::keys::canonical_keys() {
            if key_input(&Key::Named(name)).is_err() {
                missing.push(name);
            }
        }
        assert!(missing.is_empty(), "no equivalent for: {missing:?}");
    }

    #[test]
    fn plus_is_a_character_not_a_keysym() {
        // `keys.rs` canonicalises `plus` to a keysym, but it is Shift+Equal
        // on a US layout — sending the character gets it right anywhere.
        let got = key_input(&Key::Named("plus")).unwrap();
        assert_eq!(got.key, Some(pb::key_input::Key::Character("+".into())));
        assert_eq!(named_key("plus"), None);
        // And the lone-`+` chord, which arrives as a character.
        let got = key_input(&Key::Char('+')).unwrap();
        assert_eq!(got.key, Some(pb::key_input::Key::Character("+".into())));
    }

    #[test]
    fn an_unknown_multi_character_key_is_an_error() {
        let err = key_input(&Key::Named("Hyper_Something")).unwrap_err();
        assert!(matches!(err, ComputerError::Unsupported(_)), "{err}");
    }

    #[test]
    fn delivery_maps_onto_the_contract() {
        assert_eq!(Delivery::Auto.as_i32(), pb::Delivery::Auto as i32);
        assert_eq!(
            Delivery::Background.as_i32(),
            pb::Delivery::Background as i32
        );
        assert_eq!(
            Delivery::Foreground.as_i32(),
            pb::Delivery::Foreground as i32
        );
        assert_eq!(Delivery::default(), Delivery::Auto);
    }

    #[test]
    fn delivery_parses_from_a_cli_value() {
        for (text, want) in [
            ("auto", Delivery::Auto),
            ("Background", Delivery::Background),
            (" foreground ", Delivery::Foreground),
        ] {
            assert_eq!(text.parse::<Delivery>().unwrap(), want, "{text}");
            assert_eq!(want.to_string(), text.trim().to_lowercase());
        }
        let err = "backgroundish".parse::<Delivery>().unwrap_err();
        assert!(err.contains("auto, background or foreground"), "{err}");
    }

    #[test]
    fn a_token_becomes_a_bearer_header() {
        let opts = CuaOptions {
            token: Some("secret".into()),
            ..Default::default()
        };
        assert_eq!(
            opts.auth_header().unwrap().unwrap().to_str().unwrap(),
            "Bearer secret"
        );
        assert!(CuaOptions::default().auth_header().unwrap().is_none());
    }

    #[test]
    fn a_token_with_a_newline_is_rejected_before_it_reaches_a_header() {
        let opts = CuaOptions {
            token: Some("bad\ntoken".into()),
            ..Default::default()
        };
        assert!(matches!(
            opts.auth_header(),
            Err(ComputerError::InvalidArgs(_))
        ));
    }

    #[test]
    fn default_options_point_at_a_loopback_daemon() {
        let opts = CuaOptions::default();
        assert_eq!(opts.endpoint, "http://127.0.0.1:3211");
        assert!(opts.token.is_none());
        assert_eq!(opts.delivery, Delivery::Auto);
    }
    #[test]
    fn remote_transport_requires_tls_and_masks_credentials() {
        assert!(validate_endpoint("https://computer.example").is_ok());
        assert!(validate_endpoint("http://127.0.0.1:3211").is_ok());
        assert!(validate_endpoint("http://[::1]:3211").is_ok());
        for endpoint in [
            "http://192.168.1.20:3211",
            "https://user:secret@computer.example",
            "https://computer.example/m/machine/",
            "https://computer.example/?token=secret",
        ] {
            assert!(validate_endpoint(endpoint).is_err(), "{endpoint}");
        }
        let opts = CuaOptions {
            token: Some("private-token".into()),
            ..Default::default()
        };
        assert!(!format!("{opts:?}").contains("private-token"));
        assert!(opts.auth_header().unwrap().unwrap().is_sensitive());
    }
    #[test]
    fn window_epochs_survive_javascript_precision_limits() {
        let window = TargetWindow {
            id: "window".into(),
            epoch: u64::MAX,
            title: "App".into(),
            app: "App".into(),
        };
        let encoded = serde_json::to_value(&window).unwrap();
        assert_eq!(encoded["epoch"], u64::MAX.to_string());
        assert_eq!(
            serde_json::from_value::<TargetWindow>(encoded).unwrap(),
            window
        );
    }
}
