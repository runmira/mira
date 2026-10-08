//! Device pairing: who besides this machine may use the server.
//!
//! Until now the server trusted every request and relied on binding to
//! loopback. That stops working the moment it's reachable from elsewhere
//! (`tailscale serve`, a LAN bind), and even on loopback it let any website
//! open `ws://127.0.0.1:8787/ws` — WebSockets aren't covered by CORS.
//!
//! The rule now:
//! - **Local** requests are trusted: from a loopback peer, to a loopback
//!   `Host`, not forwarded by a proxy, and not sent by another site (by
//!   `Origin` / `Sec-Fetch-Site`). That's a browser on this machine, the
//!   desktop app, the Vite dev server, and local tools.
//! - Everything else needs a **device token**, as the `mira_device` cookie
//!   (so WebSockets and `<img>` carry it) or an `Authorization: Bearer`.
//!
//! A device gets its token by pairing: the local UI opens a one-time code
//! (10 minutes, single use, attempt-limited), the new device enters it.
//! Only a SHA-256 of each token is stored, in `~/.mira/devices.json`.
//! Opening codes and revoking devices are local-only.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, Path as AxumPath, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const COOKIE: &str = "mira_device";
const CODE_TTL: Duration = Duration::from_secs(10 * 60);
/// Wrong guesses before an open code is thrown away.
const CODE_ATTEMPTS: u32 = 5;
/// Wrong guesses across all codes before pairing pauses.
const FAILURE_LIMIT: usize = 10;
const FAILURE_WINDOW: Duration = Duration::from_secs(10 * 60);
/// `last_seen_at` is written at most this often per device.
const SEEN_GRANULARITY: i64 = 60;
/// No 0/O, 1/I/L: codes get read aloud and typed on phones.
const CODE_ALPHABET: &[u8] = b"23456789ABCDEFGHJKMNPQRSTUVWXYZ";
const CODE_LEN: usize = 8;

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Device {
    id: String,
    name: String,
    token_sha256: String,
    created_at: i64,
    #[serde(default)]
    last_seen_at: Option<i64>,
}

/// A device as the UI sees it — never the token hash.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct DeviceView {
    pub id: String,
    pub name: String,
    pub created_at: i64,
    pub last_seen_at: Option<i64>,
}

struct Pairing {
    code: String,
    expires: Instant,
    expires_at: i64,
    attempts: u32,
}

#[derive(Default)]
struct Inner {
    devices: Vec<Device>,
    pairing: Option<Pairing>,
    failures: VecDeque<Instant>,
}

pub struct Devices {
    path: Option<PathBuf>,
    inner: Mutex<Inner>,
}

#[derive(Debug, PartialEq)]
pub enum PairError {
    /// No open code, or it expired / was used / was discarded.
    NoCode,
    Wrong,
    /// Too many wrong guesses recently.
    Locked,
}

fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

fn sha256_hex(s: &str) -> String {
    Sha256::digest(s.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Compare without an early exit, so timing doesn't leak a prefix match.
fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn random_bytes() -> [u8; 16] {
    *uuid::Uuid::new_v4().as_bytes()
}

fn new_code() -> String {
    // 16 random bytes, 8 picks: modulo bias over 31 symbols is negligible
    // next to the attempt limit.
    random_bytes()
        .iter()
        .take(CODE_LEN)
        .map(|b| CODE_ALPHABET[*b as usize % CODE_ALPHABET.len()] as char)
        .collect()
}

fn new_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// What people type: case-insensitive, separators ignored.
fn normalize_code(input: &str) -> String {
    input
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

impl Devices {
    /// Devices persisted at `path` (missing file = none paired).
    pub fn load(path: PathBuf) -> Self {
        let devices = match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                tracing::warn!(error = %e, path = %path.display(), "devices.json unreadable; no devices paired");
                Vec::new()
            }),
            Err(_) => Vec::new(),
        };
        Self {
            path: Some(path),
            inner: Mutex::new(Inner {
                devices,
                ..Default::default()
            }),
        }
    }

    /// In-memory only, for tests.
    pub fn ephemeral() -> Self {
        Self {
            path: None,
            inner: Mutex::new(Inner::default()),
        }
    }

    pub fn default_path() -> PathBuf {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default()
            .join(".mira")
            .join("devices.json")
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn save(&self, inner: &Inner) {
        let Some(path) = &self.path else { return };
        let bytes = match serde_json::to_vec_pretty(&inner.devices) {
            Ok(b) => b,
            Err(e) => return tracing::warn!(error = %e, "encode devices.json"),
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = mira_config::write_private(path, &bytes) {
            tracing::warn!(error = %e, "write devices.json");
        }
    }

    /// Open a new pairing code, replacing any open one.
    pub fn open_code(&self) -> (String, i64) {
        let code = new_code();
        let expires_at = now_unix() + CODE_TTL.as_secs() as i64;
        self.lock().pairing = Some(Pairing {
            code: code.clone(),
            expires: Instant::now() + CODE_TTL,
            expires_at,
            attempts: 0,
        });
        (code, expires_at)
    }

    /// Trade a code for a new device. Returns the device and its token
    /// (shown to the device once; only its hash is kept).
    pub fn pair(&self, code: &str, name: &str) -> Result<(DeviceView, String), PairError> {
        let mut inner = self.lock();
        let now = Instant::now();
        while inner
            .failures
            .front()
            .is_some_and(|t| now.duration_since(*t) > FAILURE_WINDOW)
        {
            inner.failures.pop_front();
        }
        if inner.failures.len() >= FAILURE_LIMIT {
            return Err(PairError::Locked);
        }
        let Some(pairing) = inner.pairing.as_mut() else {
            return Err(PairError::NoCode);
        };
        if now >= pairing.expires {
            inner.pairing = None;
            return Err(PairError::NoCode);
        }
        if !constant_eq(normalize_code(code).as_bytes(), pairing.code.as_bytes()) {
            pairing.attempts += 1;
            if pairing.attempts >= CODE_ATTEMPTS {
                inner.pairing = None;
            }
            inner.failures.push_back(now);
            return Err(PairError::Wrong);
        }
        inner.pairing = None;

        let token = new_token();
        let name = name.trim();
        let device = Device {
            id: uuid::Uuid::new_v4().simple().to_string()[..12].to_string(),
            name: if name.is_empty() {
                "Device".into()
            } else {
                name.chars().take(60).collect()
            },
            token_sha256: sha256_hex(&token),
            created_at: now_unix(),
            last_seen_at: Some(now_unix()),
        };
        inner.devices.push(device.clone());
        self.save(&inner);
        Ok((view(&device), token))
    }

    /// The device a token belongs to, noting it was just seen.
    pub fn authenticate(&self, token: &str) -> Option<DeviceView> {
        let hash = sha256_hex(token);
        let mut inner = self.lock();
        let now = now_unix();
        let device = inner
            .devices
            .iter_mut()
            .find(|d| constant_eq(d.token_sha256.as_bytes(), hash.as_bytes()))?;
        let stale = device
            .last_seen_at
            .is_none_or(|t| now - t >= SEEN_GRANULARITY);
        if stale {
            device.last_seen_at = Some(now);
        }
        let out = view(device);
        if stale {
            self.save(&inner);
        }
        Some(out)
    }

    pub fn list(&self) -> Vec<DeviceView> {
        self.lock().devices.iter().map(view).collect()
    }

    pub fn revoke(&self, id: &str) -> bool {
        let mut inner = self.lock();
        let before = inner.devices.len();
        inner.devices.retain(|d| d.id != id);
        let removed = inner.devices.len() != before;
        if removed {
            self.save(&inner);
        }
        removed
    }

    /// The open code's expiry, if one is open.
    fn open_code_expiry(&self) -> Option<i64> {
        let inner = self.lock();
        inner
            .pairing
            .as_ref()
            .filter(|p| Instant::now() < p.expires)
            .map(|p| p.expires_at)
    }
}

fn view(d: &Device) -> DeviceView {
    DeviceView {
        id: d.id.clone(),
        name: d.name.clone(),
        created_at: d.created_at,
        last_seen_at: d.last_seen_at,
    }
}

// ---- who is asking ----------------------------------------------------------

/// Host part of a `host[:port]` / `[v6]:port` authority or URL origin.
fn host_of(authority: &str) -> &str {
    let rest = authority
        .split_once("://")
        .map_or(authority, |(_, r)| r)
        .split('/')
        .next()
        .unwrap_or("");
    if let Some(v6) = rest.strip_prefix('[') {
        return v6.split(']').next().unwrap_or("");
    }
    rest.rsplit_once(':').map_or(rest, |(h, port)| {
        if port.chars().all(|c| c.is_ascii_digit()) {
            h
        } else {
            rest
        }
    })
}

fn is_loopback_host(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host == "localhost"
        || host.ends_with(".localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

/// Is this request from this machine, on its own behalf? See the module
/// docs; every condition has to hold.
pub fn is_local(peer: Option<SocketAddr>, method: &Method, headers: &HeaderMap) -> bool {
    if !peer.is_some_and(|p| p.ip().is_loopback()) {
        return false;
    }
    // Proxies on this machine (tailscale serve, ngrok, …) connect from
    // loopback on someone else's behalf.
    for forwarded in [
        "forwarded",
        "x-forwarded-for",
        "x-forwarded-host",
        "x-real-ip",
        "cf-connecting-ip",
        "tailscale-user-login",
    ] {
        if headers.contains_key(forwarded) {
            return false;
        }
    }
    // DNS rebinding arrives with the attacker's name as Host.
    if !header(headers, "host").is_some_and(|h| is_loopback_host(host_of(h))) {
        return false;
    }
    if let Some(origin) = header(headers, "origin") {
        let ok = origin == "tauri://localhost" || {
            (origin.starts_with("http://") || origin.starts_with("https://"))
                && is_loopback_host(host_of(origin))
        };
        if !ok {
            return false;
        }
    }
    // Requests another site triggers (img, form, fetch) are cross-site.
    // Top-level GET navigations are allowed: OAuth sign-ins land back here
    // that way.
    if header(headers, "sec-fetch-site") == Some("cross-site") {
        let navigation = header(headers, "sec-fetch-mode") == Some("navigate");
        if !(navigation && method == Method::GET) {
            return false;
        }
    }
    true
}

/// Served to anyone: what an unpaired device needs to load the app shell
/// and pair. Static files carry no data; everything under `/api` and `/ws`
/// does.
fn is_public(path: &str) -> bool {
    matches!(
        path,
        "/api/health" | "/api/version" | "/api/pairing/me" | "/api/pairing/pair"
    ) || !(path.starts_with("/api/") || path == "/ws" || path.starts_with("/ws/"))
}

fn device_token(headers: &HeaderMap) -> Option<String> {
    if let Some(bearer) = header(headers, "authorization").and_then(|v| v.strip_prefix("Bearer ")) {
        return Some(bearer.trim().to_string());
    }
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|kv| {
            let (k, v) = kv.trim().split_once('=')?;
            (k == COOKIE).then(|| v.to_string())
        })
}

/// How a request was let in.
#[derive(Clone, Debug)]
pub enum Caller {
    Local,
    Device(DeviceView),
    /// Public path, no credentials.
    Anonymous,
}

fn peer(req: &Request) -> Option<SocketAddr> {
    req.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0)
}

/// Gate for every route.
pub async fn guard(State(devices): State<Arc<Devices>>, mut req: Request, next: Next) -> Response {
    let caller = if is_local(peer(&req), req.method(), req.headers()) {
        Caller::Local
    } else if let Some(device) = device_token(req.headers()).and_then(|t| devices.authenticate(&t))
    {
        Caller::Device(device)
    } else if is_public(req.uri().path()) {
        Caller::Anonymous
    } else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "error": "pairing_required",
                "message": "Pair this device from Settings → Devices on the computer running Mira.",
            })),
        )
            .into_response();
    };
    req.extensions_mut().insert(caller);
    next.run(req).await
}

fn caller(req_ext: &axum::http::Extensions) -> Caller {
    req_ext
        .get::<Caller>()
        .cloned()
        .unwrap_or(Caller::Anonymous)
}

fn local_only(c: &Caller) -> Option<Response> {
    (!matches!(c, Caller::Local)).then(|| {
        (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "only from the computer running Mira" })),
        )
            .into_response()
    })
}

// ---- endpoints ----------------------------------------------------------------

/// `GET /api/pairing/me` — whether this browser may use the server.
pub async fn me(req: Request) -> Response {
    let c = caller(req.extensions());
    Json(match c {
        Caller::Local => serde_json::json!({ "local": true, "paired": true }),
        Caller::Device(d) => serde_json::json!({ "local": false, "paired": true, "device": d }),
        Caller::Anonymous => serde_json::json!({ "local": false, "paired": false }),
    })
    .into_response()
}

/// `GET /api/pairing/devices` — paired devices (local or paired callers).
pub async fn list(State(devices): State<Arc<Devices>>) -> Response {
    Json(serde_json::json!({
        "devices": devices.list(),
        "pairing_expires_at": devices.open_code_expiry(),
        "addresses": addresses().await,
    }))
    .into_response()
}

/// `POST /api/pairing/code` — open a code (local only).
pub async fn open_pairing(State(devices): State<Arc<Devices>>, req: Request) -> Response {
    if let Some(r) = local_only(&caller(req.extensions())) {
        return r;
    }
    let (code, expires_at) = devices.open_code();
    Json(serde_json::json!({
        "code": format!("{}-{}", &code[..4], &code[4..]),
        "expires_at": expires_at,
    }))
    .into_response()
}

#[derive(Deserialize)]
pub struct PairBody {
    code: String,
    #[serde(default)]
    name: String,
}

/// `POST /api/pairing/pair` — trade a code for this device's token.
pub async fn pair(
    State(devices): State<Arc<Devices>>,
    headers: HeaderMap,
    Json(body): Json<PairBody>,
) -> Response {
    match devices.pair(&body.code, &body.name) {
        Ok((device, token)) => {
            // Secure when the page came over HTTPS (tailscale serve sets
            // X-Forwarded-Proto); HttpOnly so page scripts can't read it.
            let secure = header(&headers, "x-forwarded-proto") == Some("https");
            let cookie = format!(
                "{COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age=34560000{}",
                if secure { "; Secure" } else { "" }
            );
            let mut res = Json(serde_json::json!({ "device": device })).into_response();
            if let Ok(v) = HeaderValue::from_str(&cookie) {
                res.headers_mut().insert(header::SET_COOKIE, v);
            }
            res
        }
        Err(e) => {
            let (status, msg) = match e {
                PairError::NoCode => (
                    StatusCode::GONE,
                    "That code has expired or was already used. Open a new one in Settings → Devices.",
                ),
                PairError::Wrong => (StatusCode::UNAUTHORIZED, "That code isn't right."),
                PairError::Locked => (
                    StatusCode::TOO_MANY_REQUESTS,
                    "Too many wrong codes. Try again in a few minutes.",
                ),
            };
            (status, Json(serde_json::json!({ "error": msg }))).into_response()
        }
    }
}

/// `DELETE /api/pairing/devices/:id` — revoke (local only).
pub async fn revoke(
    State(devices): State<Arc<Devices>>,
    AxumPath(id): AxumPath<String>,
    req: Request,
) -> Response {
    if let Some(r) = local_only(&caller(req.extensions())) {
        return r;
    }
    if devices.revoke(&id) {
        StatusCode::NO_CONTENT.into_response()
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}

/// Where other devices might reach this machine: its Tailscale name and
/// addresses, when the Tailscale CLI is available. Best effort.
pub async fn addresses() -> serde_json::Value {
    let candidates = [
        "tailscale",
        "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
    ];
    for bin in candidates {
        let out = tokio::process::Command::new(bin)
            .args(["status", "--json"])
            .output();
        let Ok(Ok(out)) = tokio::time::timeout(Duration::from_secs(3), out).await else {
            continue;
        };
        if !out.status.success() {
            continue;
        }
        let Ok(status) = serde_json::from_slice::<serde_json::Value>(&out.stdout) else {
            continue;
        };
        let me = &status["Self"];
        return serde_json::json!({
            "tailscale": {
                "dns_name": me["DNSName"].as_str().map(|s| s.trim_end_matches('.')),
                "ips": me["TailscaleIPs"],
                "online": me["Online"],
            }
        });
    }
    serde_json::json!({ "tailscale": null })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                HeaderValue::from_str(v).unwrap(),
            );
        }
        h
    }

    fn lo() -> Option<SocketAddr> {
        Some("127.0.0.1:50000".parse().unwrap())
    }

    #[test]
    fn local_browser_and_tools_are_local() {
        let get = Method::GET;
        assert!(is_local(
            lo(),
            &get,
            &headers(&[("host", "127.0.0.1:8787")])
        ));
        assert!(is_local(
            lo(),
            &get,
            &headers(&[
                ("host", "localhost:8787"),
                ("origin", "http://localhost:5173")
            ])
        ));
        assert!(is_local(
            lo(),
            &get,
            &headers(&[("host", "127.0.0.1:8787"), ("origin", "tauri://localhost")])
        ));
        assert!(is_local(
            Some("[::1]:5000".parse().unwrap()),
            &get,
            &headers(&[("host", "[::1]:8787")])
        ));
        assert!(is_local(
            lo(),
            &get,
            &headers(&[("host", "127.0.0.1:8787"), ("sec-fetch-site", "same-site")])
        ));
    }

    #[test]
    fn remote_proxied_or_cross_site_requests_are_not_local() {
        let get = Method::GET;
        let post = Method::POST;
        let h = headers(&[("host", "127.0.0.1:8787")]);
        // Another machine.
        assert!(!is_local(
            Some("100.64.0.2:5000".parse().unwrap()),
            &get,
            &h
        ));
        assert!(!is_local(None, &get, &h));
        // tailscale serve: loopback peer, tailnet Host.
        assert!(!is_local(
            lo(),
            &get,
            &headers(&[("host", "box.tail1234.ts.net")])
        ));
        // Cloudflare's connector (remote access) marks what it forwards.
        assert!(!is_local(
            lo(),
            &get,
            &headers(&[
                ("host", "127.0.0.1:8787"),
                ("cf-connecting-ip", "203.0.113.5")
            ])
        ));
        // A proxy that rewrites Host but says it forwarded.
        assert!(!is_local(
            lo(),
            &get,
            &headers(&[
                ("host", "127.0.0.1:8787"),
                ("x-forwarded-for", "100.64.0.2")
            ])
        ));
        // DNS rebinding.
        assert!(!is_local(
            lo(),
            &get,
            &headers(&[("host", "evil.example:8787")])
        ));
        // A website opening the socket or posting a form.
        assert!(!is_local(
            lo(),
            &get,
            &headers(&[
                ("host", "127.0.0.1:8787"),
                ("origin", "https://evil.example")
            ])
        ));
        assert!(!is_local(
            lo(),
            &get,
            &headers(&[("host", "127.0.0.1:8787"), ("origin", "null")])
        ));
        assert!(!is_local(
            lo(),
            &post,
            &headers(&[
                ("host", "127.0.0.1:8787"),
                ("sec-fetch-site", "cross-site"),
                ("sec-fetch-mode", "navigate")
            ])
        ));
        assert!(!is_local(
            lo(),
            &get,
            &headers(&[
                ("host", "127.0.0.1:8787"),
                ("sec-fetch-site", "cross-site"),
                ("sec-fetch-mode", "no-cors")
            ])
        ));
        // ...but a top-level GET navigation (OAuth callback) is fine.
        assert!(is_local(
            lo(),
            &get,
            &headers(&[
                ("host", "127.0.0.1:8787"),
                ("sec-fetch-site", "cross-site"),
                ("sec-fetch-mode", "navigate")
            ])
        ));
    }

    #[test]
    fn host_parsing() {
        assert_eq!(host_of("127.0.0.1:8787"), "127.0.0.1");
        assert_eq!(host_of("[::1]:8787"), "::1");
        assert_eq!(host_of("http://localhost:5173"), "localhost");
        assert_eq!(host_of("https://box.ts.net"), "box.ts.net");
        assert!(is_loopback_host("app.localhost"));
        assert!(!is_loopback_host("localhost.evil.example"));
    }

    #[test]
    fn public_paths() {
        assert!(is_public("/"));
        assert!(is_public("/assets/index-abc.js"));
        assert!(is_public("/api/pairing/pair"));
        assert!(!is_public("/api/sessions"));
        assert!(!is_public("/api/pairing/devices"));
        assert!(!is_public("/ws"));
        assert!(!is_public("/ws/terminal"));
    }

    #[test]
    fn pairing_flow() {
        let d = Devices::ephemeral();
        assert_eq!(d.pair("ABCD-EFGH", "x").unwrap_err(), PairError::NoCode);
        let (code, _) = d.open_code();
        assert_eq!(code.len(), CODE_LEN);
        assert_eq!(d.pair("WRONG-CODE", "x").unwrap_err(), PairError::Wrong);
        // Typed loosely: lowercase, with a dash.
        let typed = format!("{}-{}", &code[..4], &code[4..]).to_lowercase();
        let (device, token) = d.pair(&typed, "  Phone ").unwrap();
        assert_eq!(device.name, "Phone");
        assert_eq!(token.len(), 64);
        // Single use.
        assert_eq!(d.pair(&code, "x").unwrap_err(), PairError::NoCode);
        // The token works; the hash is all that's kept.
        assert_eq!(d.authenticate(&token).unwrap().id, device.id);
        assert!(d.authenticate("nope").is_none());
        assert!(!d.lock().devices[0].token_sha256.contains(&token));
        // Revoked tokens stop working.
        assert!(d.revoke(&device.id));
        assert!(d.authenticate(&token).is_none());
    }

    #[test]
    fn wrong_guesses_burn_the_code_then_lock_pairing() {
        let d = Devices::ephemeral();
        let (code, _) = d.open_code();
        for _ in 0..CODE_ATTEMPTS {
            assert_eq!(d.pair("2222-2222", "x").unwrap_err(), PairError::Wrong);
        }
        assert_eq!(d.pair(&code, "x").unwrap_err(), PairError::NoCode);
        d.open_code();
        for _ in CODE_ATTEMPTS as usize..FAILURE_LIMIT {
            let _ = d.pair("2222-2222", "x");
        }
        let (code, _) = d.open_code();
        assert_eq!(d.pair(&code, "x").unwrap_err(), PairError::Locked);
    }

    #[test]
    fn tokens_come_from_cookie_or_bearer() {
        assert_eq!(
            device_token(&headers(&[("cookie", "a=1; mira_device=tok; b=2")])).as_deref(),
            Some("tok")
        );
        assert_eq!(
            device_token(&headers(&[("authorization", "Bearer tok2")])).as_deref(),
            Some("tok2")
        );
        assert_eq!(device_token(&headers(&[("cookie", "other=1")])), None);
    }

    async fn call(
        app: &axum::Router,
        peer: &str,
        path: &str,
        extra: &[(&str, &str)],
    ) -> StatusCode {
        use tower::ServiceExt;
        let mut req = axum::http::Request::get(path).header("host", "127.0.0.1:8787");
        for (k, v) in extra {
            req = req.header(*k, *v);
        }
        let mut req = req.body(axum::body::Body::empty()).unwrap();
        req.extensions_mut()
            .insert(ConnectInfo::<SocketAddr>(peer.parse().unwrap()));
        app.clone().oneshot(req).await.unwrap().status()
    }

    #[tokio::test]
    async fn guard_lets_in_this_machine_and_paired_devices_only() {
        let devices = Arc::new(Devices::ephemeral());
        let app = axum::Router::new()
            .route("/api/sessions", axum::routing::get(|| async { "data" }))
            .route("/ws", axum::routing::get(|| async { "socket" }))
            .route("/", axum::routing::get(|| async { "shell" }))
            .route("/api/pairing/me", axum::routing::get(me))
            .layer(axum::middleware::from_fn_with_state(devices.clone(), guard));
        let remote = "100.64.0.2:5000";

        assert_eq!(
            call(&app, "127.0.0.1:5000", "/api/sessions", &[]).await,
            StatusCode::OK
        );
        assert_eq!(
            call(&app, remote, "/api/sessions", &[]).await,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            call(&app, remote, "/ws", &[]).await,
            StatusCode::UNAUTHORIZED
        );
        // The app shell and pairing status load, so the pair screen can.
        assert_eq!(call(&app, remote, "/", &[]).await, StatusCode::OK);
        assert_eq!(
            call(&app, remote, "/api/pairing/me", &[]).await,
            StatusCode::OK
        );
        // A website poking the local server gets nothing.
        assert_eq!(
            call(
                &app,
                "127.0.0.1:5000",
                "/ws",
                &[("origin", "https://evil.example")]
            )
            .await,
            StatusCode::UNAUTHORIZED
        );

        let (code, _) = devices.open_code();
        let (_, token) = devices.pair(&code, "Phone").unwrap();
        let cookie = format!("{COOKIE}={token}");
        assert_eq!(
            call(&app, remote, "/api/sessions", &[("cookie", &cookie)]).await,
            StatusCode::OK
        );
        assert_eq!(
            call(&app, remote, "/ws", &[("cookie", &cookie)]).await,
            StatusCode::OK
        );
        assert_eq!(
            call(
                &app,
                remote,
                "/api/sessions",
                &[("cookie", "mira_device=forged")]
            )
            .await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[test]
    fn devices_persist_without_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("devices.json");
        let d = Devices::load(path.clone());
        let (code, _) = d.open_code();
        let (device, token) = d.pair(&code, "Laptop").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains(&token));
        let reloaded = Devices::load(path);
        assert_eq!(reloaded.authenticate(&token).unwrap().id, device.id);
    }
}
