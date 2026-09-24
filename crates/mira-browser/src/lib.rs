//! Browser automation for Mira's `browser` tool.
//!
//! Drives a real Chrome / Chromium / Edge / Brave over the DevTools
//! Protocol, in a **dedicated profile** (`~/.mira/browser/profile` by
//! default) so the agent never touches the user's everyday browser
//! sessions, yet logins it's walked through persist between runs.
//!
//! The model works mostly from text: [`BrowserAction::Snapshot`] lists
//! the page's interactive elements with short refs (`e12`) it can click
//! or type into, which is far cheaper than a screenshot per step.
//! Screenshots and coordinate clicks remain for visual pages (canvas,
//! maps, charts).

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use base64::Engine as _;
use serde::Deserialize;
use serde_json::{json, Value};
use thiserror::Error;
use tokio::sync::Mutex;

use mira_computer::keys::{Key, KeyChord, Modifier};
use mira_computer::{imaging, ImageLimits, Screenshot};

mod cdp;
pub mod launch;

use cdp::Cdp;

const SNAPSHOT_JS: &str = include_str!("snapshot.js");
/// Interactive elements listed per snapshot.
const MAX_ELEMENTS: usize = 250;
/// Visible-text characters per snapshot.
const MAX_TEXT: usize = 6_000;
/// `evaluate` result characters returned to the model.
const MAX_EVAL: usize = 8_000;
/// How long to wait for a page to finish loading.
const LOAD_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Error)]
pub enum BrowserError {
    #[error("invalid arguments: {0}")]
    InvalidArgs(String),
    #[error("browser not found: {0}")]
    NotFound(String),
    #[error("browser launch failed: {0}")]
    Launch(String),
    #[error("devtools: {0}")]
    Protocol(String),
    #[error("the browser was closed; the next call starts a fresh one")]
    Closed,
    #[error("{0}")]
    Page(String),
}

#[derive(Clone, Debug)]
pub struct BrowserOptions {
    /// Browser binary; `None` auto-detects (see [`launch::find_executable`]).
    pub executable: Option<PathBuf>,
    pub headless: bool,
    pub profile_dir: PathBuf,
    /// Initial window size (CSS pixels).
    pub window_size: (u32, u32),
    pub limits: ImageLimits,
}

impl Default for BrowserOptions {
    fn default() -> Self {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        // Headed when there's a display to show it on — the user can
        // watch (and take over, e.g. for a 2FA prompt). Headless on bare
        // servers, where headed would just fail to start.
        let has_display = cfg!(any(target_os = "macos", windows))
            || std::env::var_os("DISPLAY").is_some()
            || std::env::var_os("WAYLAND_DISPLAY").is_some();
        Self {
            executable: None,
            headless: !has_display,
            profile_dir: home.join(".mira").join("browser").join("profile"),
            window_size: (1280, 900),
            limits: ImageLimits::default(),
        }
    }
}

/// One parsed `browser` tool call.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrowserAction {
    Navigate {
        url: String,
    },
    Back,
    Forward,
    Reload,
    Snapshot,
    Screenshot,
    Click {
        #[serde(default, rename = "ref")]
        element: Option<String>,
        #[serde(default)]
        selector: Option<String>,
        #[serde(default)]
        coordinate: Option<[f64; 2]>,
        #[serde(default)]
        double: bool,
    },
    Type {
        text: String,
        #[serde(default, rename = "ref")]
        element: Option<String>,
        #[serde(default)]
        selector: Option<String>,
        /// Clear the field before typing.
        #[serde(default)]
        clear: bool,
        /// Press Enter afterwards.
        #[serde(default)]
        submit: bool,
    },
    Key {
        key: String,
    },
    Scroll {
        #[serde(default)]
        direction: Option<String>,
        #[serde(default)]
        amount: Option<u32>,
        #[serde(default, rename = "ref")]
        element: Option<String>,
    },
    Evaluate {
        expression: String,
    },
    ListTabs,
    NewTab {
        #[serde(default)]
        url: Option<String>,
    },
    SwitchTab {
        index: usize,
    },
    CloseTab {
        #[serde(default)]
        index: Option<usize>,
    },
    Wait {
        #[serde(default)]
        duration: Option<f64>,
    },
    Close,
}

impl BrowserAction {
    pub fn from_args(args: &Value) -> Result<Self, BrowserError> {
        serde_json::from_value(args.clone()).map_err(|e| BrowserError::InvalidArgs(e.to_string()))
    }

    pub fn verb(&self) -> &'static str {
        match self {
            Self::Navigate { .. } => "navigate",
            Self::Back => "back",
            Self::Forward => "forward",
            Self::Reload => "reload",
            Self::Snapshot => "snapshot",
            Self::Screenshot => "screenshot",
            Self::Click { .. } => "click",
            Self::Type { .. } => "type",
            Self::Key { .. } => "key",
            Self::Scroll { .. } => "scroll",
            Self::Evaluate { .. } => "evaluate",
            Self::ListTabs => "list_tabs",
            Self::NewTab { .. } => "new_tab",
            Self::SwitchTab { .. } => "switch_tab",
            Self::CloseTab { .. } => "close_tab",
            Self::Wait { .. } => "wait",
            Self::Close => "close",
        }
    }

    /// Policy target: `verb` or `verb:detail` (URL, typed text, key, JS,
    /// element). Navigation URLs are normalized first, so a rule like
    /// `Browser(navigate:https://github.com/*)` sees what will load.
    pub fn policy_target(&self) -> String {
        let detail = match self {
            Self::Navigate { url } => normalize_url(url),
            Self::NewTab { url: Some(u) } => normalize_url(u),
            Self::Click {
                element,
                selector,
                coordinate,
                ..
            } => element
                .clone()
                .or_else(|| selector.clone())
                .or_else(|| coordinate.map(|[x, y]| format!("{x},{y}")))
                .unwrap_or_default(),
            Self::Type { text, .. } => text.clone(),
            Self::Key { key } => key.clone(),
            Self::Evaluate { expression } => expression.clone(),
            Self::Scroll { direction, .. } => direction.clone().unwrap_or_else(|| "down".into()),
            _ => String::new(),
        };
        if detail.is_empty() {
            self.verb().to_owned()
        } else {
            format!("{}:{detail}", self.verb())
        }
    }
}

/// Add `https://` to scheme-less URLs (`example.com/x`). Leaves
/// `about:`, `file:`, `data:` and `chrome:` URLs alone.
pub fn normalize_url(url: &str) -> String {
    let url = url.trim();
    if url.contains("://") || url.starts_with("about:") || url.starts_with("data:") {
        url.to_owned()
    } else if url.starts_with("localhost") || url.starts_with("127.0.0.1") {
        format!("http://{url}")
    } else {
        format!("https://{url}")
    }
}

/// What one action produced.
#[derive(Clone, Debug)]
pub struct Outcome {
    pub text: String,
    pub screenshot: Option<Screenshot>,
}

struct Session {
    // Held for kill_on_drop: dropping the session kills the browser.
    _child: tokio::process::Child,
    cdp: Cdp,
    /// Page targets in the order Mira learned about them (tab indices).
    tabs: Vec<String>,
    /// target id → flat session id.
    attached: HashMap<String, String>,
    current: String,
    /// CSS pixels per model pixel, from the latest screenshot.
    css_per_px: Option<f64>,
}

/// A lazily-launched browser. The first action starts it; `close` (or
/// dropping the `Browser`) kills it.
pub struct Browser {
    opts: BrowserOptions,
    session: Mutex<Option<Session>>,
}

impl Browser {
    pub fn new(opts: BrowserOptions) -> Self {
        Self {
            opts,
            session: Mutex::new(None),
        }
    }

    pub fn options(&self) -> &BrowserOptions {
        &self.opts
    }

    pub async fn execute(&self, action: &BrowserAction) -> Result<Outcome, BrowserError> {
        let mut guard = self.session.lock().await;
        if matches!(action, BrowserAction::Close) {
            let was_open = guard.take().is_some();
            return Ok(Outcome {
                text: if was_open {
                    "Browser closed.".into()
                } else {
                    "Browser was not running.".into()
                },
                screenshot: None,
            });
        }
        if guard.is_none() {
            *guard = Some(start(&self.opts).await?);
        }
        let sess = guard.as_mut().expect("session just ensured");
        let res = run(sess, &self.opts, action).await;
        if matches!(res, Err(BrowserError::Closed)) {
            *guard = None;
        }
        res
    }
}

async fn start(opts: &BrowserOptions) -> Result<Session, BrowserError> {
    let launched = launch::launch(opts).await?;
    let cdp = Cdp::connect(&launched.ws_url).await?;
    let targets = page_targets(&cdp).await?;
    let current = match targets.first() {
        Some(t) => t.clone(),
        None => cdp
            .call(None, "Target.createTarget", json!({ "url": "about:blank" }))
            .await?["targetId"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
    };
    let mut sess = Session {
        _child: launched.child,
        cdp,
        tabs: vec![current.clone()],
        attached: HashMap::new(),
        current,
        css_per_px: None,
    };
    attach(&mut sess).await?;
    Ok(sess)
}

async fn page_targets(cdp: &Cdp) -> Result<Vec<String>, BrowserError> {
    let res = cdp.call(None, "Target.getTargets", json!({})).await?;
    Ok(res["targetInfos"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|t| t["type"] == "page")
        .filter_map(|t| t["targetId"].as_str().map(str::to_owned))
        .collect())
}

/// Flat session id for the current tab, attaching on first use.
async fn attach(sess: &mut Session) -> Result<String, BrowserError> {
    if let Some(s) = sess.attached.get(&sess.current) {
        return Ok(s.clone());
    }
    let res = sess
        .cdp
        .call(
            None,
            "Target.attachToTarget",
            json!({ "targetId": sess.current, "flatten": true }),
        )
        .await?;
    let sid = res["sessionId"]
        .as_str()
        .ok_or_else(|| BrowserError::Protocol("attachToTarget returned no sessionId".into()))?
        .to_owned();
    sess.attached.insert(sess.current.clone(), sid.clone());
    Ok(sid)
}

/// Reconcile `tabs` with the browser: drop closed tabs, append ones the
/// page opened itself (popups, `target=_blank`).
async fn refresh_tabs(sess: &mut Session) -> Result<(), BrowserError> {
    let live = page_targets(&sess.cdp).await?;
    sess.tabs.retain(|t| live.contains(t));
    sess.attached.retain(|t, _| live.contains(t));
    for t in live {
        if !sess.tabs.contains(&t) {
            sess.tabs.push(t);
        }
    }
    Ok(())
}

async fn eval(sess: &mut Session, expression: &str) -> Result<Value, BrowserError> {
    let sid = attach(sess).await?;
    let res = sess
        .cdp
        .call(
            Some(&sid),
            "Runtime.evaluate",
            json!({ "expression": expression, "returnByValue": true, "awaitPromise": true }),
        )
        .await?;
    if let Some(ex) = res.get("exceptionDetails") {
        let msg = ex["exception"]["description"]
            .as_str()
            .or_else(|| ex["text"].as_str())
            .unwrap_or("script threw");
        return Err(BrowserError::Page(format!("JavaScript error: {msg}")));
    }
    Ok(res["result"]["value"].clone())
}

async fn page_call(sess: &mut Session, method: &str, params: Value) -> Result<Value, BrowserError> {
    let sid = attach(sess).await?;
    sess.cdp.call(Some(&sid), method, params).await
}

/// Wait for the current page to finish loading. A short grace period
/// first lets a navigation that a click/key just triggered begin.
async fn settle(sess: &mut Session) -> Result<(), BrowserError> {
    tokio::time::sleep(Duration::from_millis(250)).await;
    let deadline = tokio::time::Instant::now() + LOAD_TIMEOUT;
    loop {
        match eval(sess, "document.readyState").await {
            Ok(v) if v == "complete" => break,
            // Mid-navigation the old context can vanish; just retry.
            Ok(_) | Err(BrowserError::Page(_)) | Err(BrowserError::Protocol(_)) => {}
            Err(e) => return Err(e),
        }
        if tokio::time::Instant::now() >= deadline {
            break; // slow page: report what's there rather than fail
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    Ok(())
}

async fn snapshot(sess: &mut Session) -> Result<String, BrowserError> {
    let raw = eval(
        sess,
        &format!("({SNAPSHOT_JS})({MAX_ELEMENTS}, {MAX_TEXT})"),
    )
    .await?;
    let v: Value = serde_json::from_str(raw.as_str().unwrap_or("{}"))
        .map_err(|e| BrowserError::Page(format!("bad snapshot: {e}")))?;
    let mut out = format!(
        "Page: {}\nURL: {}\nViewport: {}x{}, scrolled {}/{}px\n",
        v["title"].as_str().unwrap_or(""),
        v["url"].as_str().unwrap_or(""),
        v["viewport"][0],
        v["viewport"][1],
        v["scroll"][0],
        v["scroll"][1],
    );
    let elements: Vec<&str> = v["elements"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    out.push_str(&format!("\nInteractive elements ({}):\n", elements.len()));
    for e in &elements {
        out.push_str(e);
        out.push('\n');
    }
    if let Some(n) = v["omitted"].as_u64().filter(|n| *n > 0) {
        out.push_str(&format!("… {n} more not listed\n"));
    }
    out.push_str("\nText:\n");
    out.push_str(v["text"].as_str().unwrap_or(""));
    if v["textTruncated"].as_bool().unwrap_or(false) {
        out.push_str("\n… (truncated; scroll or use evaluate for more)");
    }
    Ok(out)
}

/// PNG width/height from the IHDR chunk.
fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    if png.len() < 24 || &png[1..4] != b"PNG" {
        return None;
    }
    let w = u32::from_be_bytes(png[16..20].try_into().ok()?);
    let h = u32::from_be_bytes(png[20..24].try_into().ok()?);
    Some((w, h))
}

async fn screenshot(sess: &mut Session, limits: &ImageLimits) -> Result<Screenshot, BrowserError> {
    let vp = eval(sess, "JSON.stringify([innerWidth, innerHeight])").await?;
    let [css_w, css_h]: [f64; 2] = serde_json::from_str(vp.as_str().unwrap_or("[0,0]"))
        .map_err(|e| BrowserError::Page(format!("viewport: {e}")))?;
    let res = page_call(sess, "Page.captureScreenshot", json!({ "format": "png" })).await?;
    let b64 = res["data"]
        .as_str()
        .ok_or_else(|| BrowserError::Protocol("captureScreenshot returned no data".into()))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| BrowserError::Protocol(format!("screenshot base64: {e}")))?;
    let (w, h) = png_size(&bytes).unwrap_or((css_w as u32, css_h as u32));
    // Fit the provider limits; this also folds HiDPI (2× captures) back
    // down toward CSS size.
    let (tw, th) = limits.fit(w, h);
    let (shot_b64, sw, sh) = if (tw, th) == (w, h) {
        (b64.to_owned(), w, h)
    } else {
        let img = imaging::resize(
            imaging::decode(&bytes).map_err(|e| BrowserError::Page(e.to_string()))?,
            tw,
            th,
        );
        (
            imaging::png_base64(&img).map_err(|e| BrowserError::Page(e.to_string()))?,
            tw,
            th,
        )
    };
    sess.css_per_px = Some(if sw > 0 { css_w / sw as f64 } else { 1.0 });
    Ok(Screenshot {
        png_base64: shot_b64,
        width: sw,
        height: sh,
    })
}

fn valid_ref(r: &str) -> Result<(), BrowserError> {
    let ok = r.len() > 1 && r.starts_with('e') && r[1..].chars().all(|c| c.is_ascii_digit());
    if ok {
        Ok(())
    } else {
        Err(BrowserError::InvalidArgs(format!(
            "`{r}` is not an element ref; use one like `e12` from a snapshot"
        )))
    }
}

/// Scroll the element into view and return its center (CSS px).
async fn element_center(
    sess: &mut Session,
    element: Option<&str>,
    selector: Option<&str>,
) -> Result<(f64, f64), BrowserError> {
    let css = match (element, selector) {
        (Some(r), _) => {
            valid_ref(r)?;
            format!("[data-mira-ref=\"{r}\"]")
        }
        (None, Some(s)) => s.to_owned(),
        (None, None) => {
            return Err(BrowserError::InvalidArgs(
                "give `ref` (from a snapshot), `selector`, or `coordinate`".into(),
            ))
        }
    };
    // The selector travels as a JSON string literal — no way to break
    // out of it into arbitrary script.
    let js = format!(
        "(function(sel){{ const el=document.querySelector(sel); if(!el) return null; \
         el.scrollIntoView({{block:'center',inline:'center',behavior:'instant'}}); \
         const r=el.getBoundingClientRect(); return JSON.stringify([r.left+r.width/2, r.top+r.height/2]); }})({})",
        serde_json::to_string(&css).expect("string encodes")
    );
    let v = eval(sess, &js).await?;
    let Some(s) = v.as_str() else {
        return Err(BrowserError::Page(format!(
            "no element matches {} (take a fresh snapshot; refs change when the page does)",
            element.unwrap_or(&css)
        )));
    };
    let [x, y]: [f64; 2] =
        serde_json::from_str(s).map_err(|e| BrowserError::Page(format!("element box: {e}")))?;
    Ok((x, y))
}

async fn mouse_click(sess: &mut Session, x: f64, y: f64, count: u32) -> Result<(), BrowserError> {
    page_call(
        sess,
        "Input.dispatchMouseEvent",
        json!({ "type": "mouseMoved", "x": x, "y": y }),
    )
    .await?;
    for i in 1..=count {
        for kind in ["mousePressed", "mouseReleased"] {
            page_call(
                sess,
                "Input.dispatchMouseEvent",
                json!({ "type": kind, "x": x, "y": y, "button": "left", "clickCount": i }),
            )
            .await?;
        }
    }
    Ok(())
}

/// DevTools key description: (key, code, windowsVirtualKeyCode, text).
fn cdp_key(key: &Key) -> Option<(String, String, u32, Option<String>)> {
    let named = |k: &str, code: &str, vk: u32, text: Option<&str>| {
        Some((k.to_owned(), code.to_owned(), vk, text.map(str::to_owned)))
    };
    match key {
        Key::Named(n) => match *n {
            "Return" => named("Enter", "Enter", 13, Some("\r")),
            "Tab" => named("Tab", "Tab", 9, None),
            "Escape" => named("Escape", "Escape", 27, None),
            "BackSpace" => named("Backspace", "Backspace", 8, None),
            "Delete" => named("Delete", "Delete", 46, None),
            "Insert" => named("Insert", "Insert", 45, None),
            "space" => named(" ", "Space", 32, Some(" ")),
            "Up" => named("ArrowUp", "ArrowUp", 38, None),
            "Down" => named("ArrowDown", "ArrowDown", 40, None),
            "Left" => named("ArrowLeft", "ArrowLeft", 37, None),
            "Right" => named("ArrowRight", "ArrowRight", 39, None),
            "Home" => named("Home", "Home", 36, None),
            "End" => named("End", "End", 35, None),
            "Page_Up" => named("PageUp", "PageUp", 33, None),
            "Page_Down" => named("PageDown", "PageDown", 34, None),
            "minus" => named("-", "Minus", 189, Some("-")),
            "equal" => named("=", "Equal", 187, Some("=")),
            "plus" => named("+", "Equal", 187, Some("+")),
            "comma" => named(",", "Comma", 188, Some(",")),
            "period" => named(".", "Period", 190, Some(".")),
            "slash" => named("/", "Slash", 191, Some("/")),
            f if f.starts_with('F') => {
                let n: u32 = f[1..].parse().ok()?;
                named(f, f, 111 + n, None)
            }
            _ => None,
        },
        Key::Char(c) => {
            let s = c.to_string();
            let (code, vk) = if c.is_ascii_alphabetic() {
                (
                    format!("Key{}", c.to_ascii_uppercase()),
                    c.to_ascii_uppercase() as u32,
                )
            } else if c.is_ascii_digit() {
                (format!("Digit{c}"), *c as u32)
            } else {
                (String::new(), 0)
            };
            Some((s.clone(), code, vk, Some(s)))
        }
    }
}

async fn press_key(sess: &mut Session, chord: &str) -> Result<(), BrowserError> {
    let chord = KeyChord::parse(chord).map_err(|e| BrowserError::InvalidArgs(e.to_string()))?;
    let Some(key) = &chord.key else {
        return Err(BrowserError::InvalidArgs(
            "key chord needs a non-modifier key".into(),
        ));
    };
    let (k, code, vk, text) = cdp_key(key)
        .ok_or_else(|| BrowserError::InvalidArgs(format!("unsupported key {key:?}")))?;
    let modifiers: u32 = chord
        .modifiers
        .iter()
        .map(|m| match m {
            Modifier::Alt => 1,
            Modifier::Ctrl => 2,
            Modifier::Meta => 4,
            Modifier::Shift => 8,
        })
        .sum();
    // With Ctrl/Cmd held the key is a shortcut, not text input.
    let text = text.filter(|_| modifiers & (2 | 4) == 0);
    let mut down = json!({
        "type": if text.is_some() { "keyDown" } else { "rawKeyDown" },
        "key": k, "code": code, "windowsVirtualKeyCode": vk, "modifiers": modifiers,
    });
    if let Some(t) = &text {
        down["text"] = json!(t);
    }
    page_call(sess, "Input.dispatchKeyEvent", down).await?;
    page_call(
        sess,
        "Input.dispatchKeyEvent",
        json!({ "type": "keyUp", "key": k, "code": code, "windowsVirtualKeyCode": vk, "modifiers": modifiers }),
    )
    .await?;
    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_owned();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}… ({} bytes truncated)", &s[..end], s.len() - end)
}

/// Settle, then report `prefix` plus a fresh snapshot. Actions that
/// change the page use this so the model sees the result (and the new
/// refs) without another round trip.
async fn with_snapshot(sess: &mut Session, prefix: String) -> Result<Outcome, BrowserError> {
    settle(sess).await?;
    let snap = snapshot(sess).await?;
    Ok(Outcome {
        text: format!("{prefix}\n\n{snap}"),
        screenshot: None,
    })
}

async fn run(
    sess: &mut Session,
    opts: &BrowserOptions,
    action: &BrowserAction,
) -> Result<Outcome, BrowserError> {
    use BrowserAction as A;
    let text_only = |text: String| Outcome {
        text,
        screenshot: None,
    };
    match action {
        A::Navigate { url } => {
            let url = normalize_url(url);
            let res = page_call(sess, "Page.navigate", json!({ "url": url })).await?;
            if let Some(err) = res["errorText"].as_str().filter(|e| !e.is_empty()) {
                return Err(BrowserError::Page(format!(
                    "navigation to {url} failed: {err}"
                )));
            }
            with_snapshot(sess, format!("Navigated to {url}.")).await
        }
        A::Back | A::Forward => {
            let dir = if matches!(action, A::Back) { -1 } else { 1 };
            eval(sess, &format!("history.go({dir})")).await?;
            with_snapshot(sess, format!("Went {}.", action.verb())).await
        }
        A::Reload => {
            page_call(sess, "Page.reload", json!({})).await?;
            with_snapshot(sess, "Reloaded.".into()).await
        }
        A::Snapshot => {
            let s = snapshot(sess).await?;
            Ok(text_only(s))
        }
        A::Screenshot => {
            let shot = screenshot(sess, &opts.limits).await?;
            Ok(Outcome {
                text: format!(
                    "Screenshot {}x{} of the viewport. `click` with `coordinate` uses this image's pixels.",
                    shot.width, shot.height
                ),
                screenshot: Some(shot),
            })
        }
        A::Click {
            element,
            selector,
            coordinate,
            double,
        } => {
            let (x, y) = match coordinate {
                Some([x, y]) if element.is_none() && selector.is_none() => {
                    let f = sess.css_per_px.ok_or_else(|| {
                        BrowserError::InvalidArgs(
                            "take a screenshot before clicking by coordinate".into(),
                        )
                    })?;
                    (x * f, y * f)
                }
                _ => element_center(sess, element.as_deref(), selector.as_deref()).await?,
            };
            mouse_click(sess, x, y, if *double { 2 } else { 1 }).await?;
            refresh_tabs(sess).await?;
            let what = element
                .clone()
                .or_else(|| selector.clone())
                .unwrap_or_else(|| format!("({x:.0}, {y:.0})"));
            with_snapshot(sess, format!("Clicked {what}.")).await
        }
        A::Type {
            text,
            element,
            selector,
            clear,
            submit,
        } => {
            if element.is_some() || selector.is_some() {
                let (x, y) = element_center(sess, element.as_deref(), selector.as_deref()).await?;
                mouse_click(sess, x, y, 1).await?;
            }
            if *clear {
                eval(
                    sess,
                    "(function(){ const el=document.activeElement; if(!el) return; \
                     if('value' in el){ el.select ? el.select() : null; el.value=''; \
                     el.dispatchEvent(new Event('input',{bubbles:true})); } \
                     else if(el.isContentEditable){ el.textContent=''; } })()",
                )
                .await?;
            }
            page_call(sess, "Input.insertText", json!({ "text": text })).await?;
            if *submit {
                press_key(sess, "Return").await?;
            }
            with_snapshot(
                sess,
                format!(
                    "Typed {} characters{}.",
                    text.chars().count(),
                    if *submit { " and pressed Enter" } else { "" }
                ),
            )
            .await
        }
        A::Key { key } => {
            press_key(sess, key).await?;
            with_snapshot(sess, format!("Pressed {key}.")).await
        }
        A::Scroll {
            direction,
            amount,
            element,
        } => {
            let amount = amount.unwrap_or(3).clamp(1, 50) as f64;
            let (dx, dy) = match direction.as_deref().unwrap_or("down") {
                "down" => (0.0, 120.0 * amount),
                "up" => (0.0, -120.0 * amount),
                "right" => (120.0 * amount, 0.0),
                "left" => (-120.0 * amount, 0.0),
                other => {
                    return Err(BrowserError::InvalidArgs(format!(
                        "unknown direction `{other}`"
                    )))
                }
            };
            let (x, y) = match element {
                Some(r) => element_center(sess, Some(r), None).await?,
                None => {
                    let v = eval(sess, "JSON.stringify([innerWidth/2, innerHeight/2])").await?;
                    serde_json::from_str::<[f64; 2]>(v.as_str().unwrap_or("[100,100]"))
                        .map(|[x, y]| (x, y))
                        .unwrap_or((100.0, 100.0))
                }
            };
            page_call(
                sess,
                "Input.dispatchMouseEvent",
                json!({ "type": "mouseWheel", "x": x, "y": y, "deltaX": dx, "deltaY": dy }),
            )
            .await?;
            with_snapshot(sess, "Scrolled.".into()).await
        }
        A::Evaluate { expression } => {
            let v = eval(sess, expression).await?;
            let rendered = match &v {
                Value::Null => "undefined / null".to_owned(),
                Value::String(s) => s.clone(),
                other => serde_json::to_string_pretty(other).unwrap_or_default(),
            };
            Ok(text_only(truncate(&rendered, MAX_EVAL)))
        }
        A::ListTabs => {
            refresh_tabs(sess).await?;
            let res = sess.cdp.call(None, "Target.getTargets", json!({})).await?;
            let infos: HashMap<String, (String, String)> = res["targetInfos"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|t| {
                    Some((
                        t["targetId"].as_str()?.to_owned(),
                        (
                            t["title"].as_str().unwrap_or("").to_owned(),
                            t["url"].as_str().unwrap_or("").to_owned(),
                        ),
                    ))
                })
                .collect();
            let mut out = String::from("Tabs:\n");
            for (i, t) in sess.tabs.iter().enumerate() {
                let (title, url) = infos.get(t).cloned().unwrap_or_default();
                let mark = if *t == sess.current { " (current)" } else { "" };
                out.push_str(&format!("[{i}] {title} — {url}{mark}\n"));
            }
            Ok(text_only(out))
        }
        A::NewTab { url } => {
            let url = url
                .as_deref()
                .map(normalize_url)
                .unwrap_or_else(|| "about:blank".into());
            let res = sess
                .cdp
                .call(None, "Target.createTarget", json!({ "url": url }))
                .await?;
            let id = res["targetId"]
                .as_str()
                .ok_or_else(|| BrowserError::Protocol("createTarget returned no id".into()))?
                .to_owned();
            refresh_tabs(sess).await?;
            if !sess.tabs.contains(&id) {
                sess.tabs.push(id.clone());
            }
            sess.current = id;
            let idx = sess.tabs.len() - 1;
            with_snapshot(sess, format!("Opened tab [{idx}] at {url}.")).await
        }
        A::SwitchTab { index } => {
            refresh_tabs(sess).await?;
            let id = sess.tabs.get(*index).cloned().ok_or_else(|| {
                BrowserError::InvalidArgs(format!("no tab [{index}]; see list_tabs"))
            })?;
            sess.current = id.clone();
            let _ = sess
                .cdp
                .call(None, "Target.activateTarget", json!({ "targetId": id }))
                .await;
            with_snapshot(sess, format!("Switched to tab [{index}].")).await
        }
        A::CloseTab { index } => {
            refresh_tabs(sess).await?;
            let idx = index.unwrap_or_else(|| {
                sess.tabs
                    .iter()
                    .position(|t| *t == sess.current)
                    .unwrap_or(0)
            });
            let id = sess.tabs.get(idx).cloned().ok_or_else(|| {
                BrowserError::InvalidArgs(format!("no tab [{idx}]; see list_tabs"))
            })?;
            if sess.tabs.len() == 1 {
                return Err(BrowserError::InvalidArgs(
                    "that's the last tab; use `close` to shut the browser".into(),
                ));
            }
            sess.cdp
                .call(None, "Target.closeTarget", json!({ "targetId": id }))
                .await?;
            sess.tabs.retain(|t| *t != id);
            sess.attached.remove(&id);
            if sess.current == id {
                sess.current = sess.tabs[0].clone();
            }
            Ok(text_only(format!(
                "Closed tab [{idx}]. Current tab is [0]."
            )))
        }
        A::Wait { duration } => {
            let secs = duration.unwrap_or(1.0).clamp(0.0, 30.0);
            tokio::time::sleep(Duration::from_secs_f64(secs)).await;
            with_snapshot(sess, format!("Waited {secs}s.")).await
        }
        A::Close => unreachable!("handled in Browser::execute"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_actions_and_targets() {
        let a = BrowserAction::from_args(&json!({"action": "navigate", "url": "github.com/x"}))
            .unwrap();
        assert_eq!(a.policy_target(), "navigate:https://github.com/x");
        let c = BrowserAction::from_args(&json!({"action": "click", "ref": "e4"})).unwrap();
        assert_eq!(c.policy_target(), "click:e4");
        let t = BrowserAction::from_args(
            &json!({"action": "type", "text": "hi", "ref": "e2", "submit": true}),
        )
        .unwrap();
        assert_eq!(t.policy_target(), "type:hi");
        assert_eq!(
            BrowserAction::from_args(&json!({"action": "snapshot"}))
                .unwrap()
                .policy_target(),
            "snapshot"
        );
        assert!(BrowserAction::from_args(&json!({"action": "fly"})).is_err());
        assert!(BrowserAction::from_args(&json!({"action": "click", "reff": "e1"})).is_err());
    }

    #[test]
    fn url_normalization() {
        assert_eq!(normalize_url("example.com"), "https://example.com");
        assert_eq!(normalize_url("localhost:3000/a"), "http://localhost:3000/a");
        assert_eq!(normalize_url("http://x.dev"), "http://x.dev");
        assert_eq!(normalize_url("about:blank"), "about:blank");
    }

    #[test]
    fn refs_are_validated() {
        assert!(valid_ref("e12").is_ok());
        assert!(valid_ref("e").is_err());
        assert!(valid_ref("\"]; alert(1)").is_err());
    }

    #[test]
    fn cdp_keys() {
        let enter = cdp_key(&Key::Named("Return")).unwrap();
        assert_eq!((enter.0.as_str(), enter.2), ("Enter", 13));
        let a = cdp_key(&Key::Char('a')).unwrap();
        assert_eq!((a.1.as_str(), a.2), ("KeyA", 65));
        assert_eq!(cdp_key(&Key::Named("F5")).unwrap().2, 116);
    }
}
