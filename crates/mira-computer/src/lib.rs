//! Desktop control for Mira's `computer` tool.
//!
//! Layers, bottom up:
//!
//! - [`backend::ComputerBackend`] — one per platform, speaks the OS's
//!   input coordinates. [`backend::detect`] picks the right one.
//! - [`imaging`] — screenshot resizing and the [`imaging::Scale`] that
//!   maps model-space coordinates (pixels of the screenshot the model
//!   saw) to input space. HiDPI lives entirely here.
//! - [`Computer`] — executes one parsed [`ComputerAction`] end to end:
//!   bounds-checks and rescales coordinates, drives the backend, and
//!   takes the follow-up screenshot.
//!
//! The policy layer never sees this crate directly: the tool wrapper in
//! `mira-tools` gates on [`ComputerAction::policy_target`] before
//! [`Computer::execute`] runs.

use std::sync::Arc;
use std::time::Duration;

use thiserror::Error;
use tokio::sync::Mutex;

pub mod action;
pub mod backend;
pub mod imaging;
pub mod keys;
mod xwd;

pub use action::{ComputerAction, MouseButton, Point, ScrollDirection};
pub use backend::{detect, preflight, ButtonAction, Check, ComputerBackend, KeyAction};
pub use imaging::{ImageLimits, Scale};
pub use keys::KeyChord;

#[derive(Debug, Error)]
pub enum ComputerError {
    #[error("invalid arguments: {0}")]
    InvalidArgs(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("missing dependency: {0} is not installed or not on PATH")]
    MissingDependency(String),
    #[error("{0}")]
    Command(String),
    #[error("image error: {0}")]
    Image(String),
}

/// Knobs for [`Computer`].
#[derive(Clone, Debug)]
pub struct ComputerOptions {
    pub limits: ImageLimits,
    /// Take a screenshot after every action that changes something, so the
    /// model sees the result without a second round trip. On by default:
    /// it's what the model is trained to expect and saves a turn per step.
    pub screenshot_after_action: bool,
    /// Pause before that follow-up screenshot so animations and page
    /// loads settle.
    pub settle: Duration,
}

impl Default for ComputerOptions {
    fn default() -> Self {
        Self {
            limits: ImageLimits::default(),
            screenshot_after_action: true,
            settle: Duration::from_millis(600),
        }
    }
}

/// A screenshot ready for the model.
#[derive(Clone, Debug)]
pub struct Screenshot {
    pub png_base64: String,
    pub width: u32,
    pub height: u32,
}

/// What one action produced.
#[derive(Clone, Debug)]
pub struct Outcome {
    pub text: String,
    pub screenshot: Option<Screenshot>,
}

/// Executes computer actions against one backend. Serializes actions:
/// two clicks racing each other on the same screen is never what anyone
/// wants.
pub struct Computer {
    backend: Arc<dyn ComputerBackend>,
    opts: ComputerOptions,
    /// Last known model↔input mapping; refreshed by every screenshot.
    scale: Mutex<Option<Scale>>,
}

impl Computer {
    pub fn new(backend: Arc<dyn ComputerBackend>, opts: ComputerOptions) -> Self {
        Self {
            backend,
            opts,
            scale: Mutex::new(None),
        }
    }

    pub fn backend_name(&self) -> &'static str {
        self.backend.name()
    }

    /// Current model-space display size (what screenshots will measure).
    pub async fn display_size(&self) -> Result<(u32, u32), ComputerError> {
        Ok(self.refresh_scale().await?.model)
    }

    async fn refresh_scale(&self) -> Result<Scale, ComputerError> {
        let size = self.backend.screen_size().await?;
        let scale = Scale::new(size, &self.opts.limits);
        *self.scale.lock().await = Some(scale);
        Ok(scale)
    }

    async fn scale(&self) -> Result<Scale, ComputerError> {
        let cached = *self.scale.lock().await;
        match cached {
            Some(s) => Ok(s),
            None => self.refresh_scale().await,
        }
    }

    /// Model-space point → input-space point, rejecting off-screen ones.
    async fn to_input(&self, p: Point) -> Result<Point, ComputerError> {
        let scale = self.scale().await?;
        scale.check_bounds(p)?;
        Ok(scale.to_input(p))
    }

    pub async fn screenshot(&self) -> Result<Screenshot, ComputerError> {
        let scale = self.refresh_scale().await?;
        let raw = self.backend.capture().await?;
        let img = imaging::resize(raw, scale.model.0, scale.model.1);
        Ok(Screenshot {
            png_base64: imaging::png_base64(&img)?,
            width: img.width(),
            height: img.height(),
        })
    }

    async fn zoom(&self, region: [i32; 4]) -> Result<Screenshot, ComputerError> {
        let scale = self.scale().await?;
        let [x0, y0, x1, y1] = region;
        if x1 <= x0 || y1 <= y0 {
            return Err(ComputerError::InvalidArgs(
                "zoom region must be [x0, y0, x1, y1] with x1 > x0 and y1 > y0".into(),
            ));
        }
        scale.check_bounds(Point { x: x0, y: y0 })?;
        let raw = self.backend.capture().await?;
        // Model space → capture pixels (capture may be 2× on HiDPI).
        let fx = raw.width() as f64 / scale.model.0 as f64;
        let fy = raw.height() as f64 / scale.model.1 as f64;
        let px = |v: i32, f: f64| (v.max(0) as f64 * f).round() as u32;
        let crop = imaging::crop(&raw, [px(x0, fx), px(y0, fy), px(x1, fx), px(y1, fy)]);
        let (w, h) = self.opts.limits.fit(crop.width(), crop.height());
        let img = imaging::resize(crop, w, h);
        Ok(Screenshot {
            png_base64: imaging::png_base64(&img)?,
            width: img.width(),
            height: img.height(),
        })
    }

    /// Hold `modifiers` (a chord like `shift` or `ctrl+alt`) around `f`.
    async fn with_modifiers<F, Fut>(
        &self,
        modifiers: Option<&str>,
        f: F,
    ) -> Result<(), ComputerError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<(), ComputerError>>,
    {
        let Some(m) = modifiers else {
            return f().await;
        };
        let chord = KeyChord::parse(m)?;
        self.backend.key(&chord, KeyAction::Down).await?;
        let res = f().await;
        // Always release, even when the action failed — a stuck modifier
        // poisons every later keystroke on the user's machine.
        let up = self.backend.key(&chord, KeyAction::Up).await;
        res.and(up)
    }

    /// Run one action. Coordinates in `action` are model space.
    pub async fn execute(&self, action: &ComputerAction) -> Result<Outcome, ComputerError> {
        use ComputerAction as A;
        let b = &self.backend;
        let text = match action {
            A::Screenshot => {
                let shot = self.screenshot().await?;
                return Ok(Outcome {
                    text: format!(
                        "Screenshot {}x{}. Coordinates you send are in this image's pixel space.",
                        shot.width, shot.height
                    ),
                    screenshot: Some(shot),
                });
            }
            A::Zoom { region } => {
                let shot = self.zoom(*region).await?;
                return Ok(Outcome {
                    text: format!(
                        "Zoomed view of {region:?} at {}x{}. Keep using full-screenshot coordinates.",
                        shot.width, shot.height
                    ),
                    screenshot: Some(shot),
                });
            }
            A::CursorPosition => {
                let scale = self.scale().await?;
                let p = scale.to_model(b.cursor_position().await?);
                return Ok(Outcome {
                    text: format!("Cursor at ({}, {}).", p.x, p.y),
                    screenshot: None,
                });
            }
            A::Wait { seconds } => {
                tokio::time::sleep(Duration::from_secs_f64(*seconds)).await;
                let shot = self.screenshot().await?;
                return Ok(Outcome {
                    text: format!("Waited {seconds}s."),
                    screenshot: Some(shot),
                });
            }
            A::MouseMove(p) => {
                b.mouse_move(self.to_input(*p).await?).await?;
                format!("Moved mouse to ({}, {}).", p.x, p.y)
            }
            A::Click {
                button,
                at,
                count,
                modifiers,
            } => {
                if let Some(p) = at {
                    b.mouse_move(self.to_input(*p).await?).await?;
                }
                self.with_modifiers(modifiers.as_deref(), || {
                    b.button(*button, ButtonAction::Click(*count))
                })
                .await?;
                match at {
                    Some(p) => format!("{} at ({}, {}).", action.verb(), p.x, p.y),
                    None => format!("{} at the current position.", action.verb()),
                }
            }
            A::Drag { from, to } => {
                let (f, t) = (self.to_input(*from).await?, self.to_input(*to).await?);
                b.mouse_move(f).await?;
                b.button(MouseButton::Left, ButtonAction::Down).await?;
                // A midpoint makes drag-and-drop targets register the
                // hover; many UIs ignore a single-jump drag.
                let mid = Point {
                    x: (f.x + t.x) / 2,
                    y: (f.y + t.y) / 2,
                };
                let res = async {
                    b.mouse_move(mid).await?;
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    b.mouse_move(t).await
                }
                .await;
                b.button(MouseButton::Left, ButtonAction::Up).await?;
                res?;
                format!(
                    "Dragged from ({}, {}) to ({}, {}).",
                    from.x, from.y, to.x, to.y
                )
            }
            A::MouseDown => {
                b.button(MouseButton::Left, ButtonAction::Down).await?;
                "Left button down.".to_owned()
            }
            A::MouseUp => {
                b.button(MouseButton::Left, ButtonAction::Up).await?;
                "Left button up.".to_owned()
            }
            A::Type(t) => {
                b.type_text(t).await?;
                format!("Typed {} characters.", t.chars().count())
            }
            A::Key(k) => {
                b.key(&KeyChord::parse(k)?, KeyAction::Press).await?;
                format!("Pressed {k}.")
            }
            A::HoldKey { keys, seconds } => {
                let chord = KeyChord::parse(keys)?;
                b.key(&chord, KeyAction::Down).await?;
                tokio::time::sleep(Duration::from_secs_f64(*seconds)).await;
                b.key(&chord, KeyAction::Up).await?;
                format!("Held {keys} for {seconds}s.")
            }
            A::Scroll {
                at,
                direction,
                amount,
                modifiers,
            } => {
                if let Some(p) = at {
                    b.mouse_move(self.to_input(*p).await?).await?;
                }
                self.with_modifiers(modifiers.as_deref(), || b.scroll(*direction, *amount))
                    .await?;
                format!("Scrolled {direction:?} by {amount}.").to_lowercase()
            }
        };

        let screenshot = if self.opts.screenshot_after_action {
            tokio::time::sleep(self.opts.settle).await;
            Some(self.screenshot().await?)
        } else {
            None
        };
        Ok(Outcome { text, screenshot })
    }
}

#[cfg(test)]
mod tests {
    use super::backend::mock::{Event, MockBackend};
    use super::*;
    use serde_json::json;

    fn computer(mock: Arc<MockBackend>) -> Computer {
        Computer::new(
            mock,
            ComputerOptions {
                settle: Duration::ZERO,
                ..Default::default()
            },
        )
    }

    #[tokio::test]
    async fn hidpi_click_lands_in_input_space() {
        // 2560x1600 logical (too big for the limits) captured at 2×.
        let mock = Arc::new(MockBackend::new((2560, 1600), (5120, 3200)));
        let c = computer(mock.clone());
        let shot = c.screenshot().await.unwrap();
        assert!(shot.width < 2560 && shot.height < 1600);
        let (mw, mh) = (shot.width as i32, shot.height as i32);

        let act = ComputerAction::from_args(&json!({
            "action": "left_click", "coordinate": [mw / 2, mh / 2]
        }))
        .unwrap();
        let out = c.execute(&act).await.unwrap();
        assert!(out.screenshot.is_some(), "follow-up screenshot");
        let ev = mock.events();
        let Event::Move(p) = ev[0] else {
            panic!("{ev:?}")
        };
        assert!((p.x - 1280).abs() <= 2 && (p.y - 800).abs() <= 2, "{p:?}");
        assert_eq!(
            ev[1],
            Event::Button(MouseButton::Left, ButtonAction::Click(1))
        );
    }

    #[tokio::test]
    async fn out_of_bounds_clicks_are_refused_without_input() {
        let mock = Arc::new(MockBackend::new((1280, 800), (1280, 800)));
        let c = computer(mock.clone());
        let act =
            ComputerAction::from_args(&json!({"action": "left_click", "coordinate": [5000, 10]}))
                .unwrap();
        assert!(c.execute(&act).await.is_err());
        assert!(mock.events().is_empty());
    }

    #[tokio::test]
    async fn modifiers_are_released_after_click() {
        let mock = Arc::new(MockBackend::new((1280, 800), (1280, 800)));
        let c = computer(mock.clone());
        let act = ComputerAction::from_args(&json!({
            "action": "left_click", "coordinate": [10, 10], "text": "shift"
        }))
        .unwrap();
        c.execute(&act).await.unwrap();
        let ev = mock.events();
        assert!(matches!(ev[1], Event::Key(_, KeyAction::Down)));
        assert!(matches!(ev[3], Event::Key(_, KeyAction::Up)));
    }

    #[tokio::test]
    async fn zoom_crops_from_full_resolution() {
        let mock = Arc::new(MockBackend::new((1000, 500), (2000, 1000)));
        let c = computer(mock);
        c.screenshot().await.unwrap();
        let out = c
            .execute(&ComputerAction::Zoom {
                region: [0, 0, 100, 50],
            })
            .await
            .unwrap();
        let s = out.screenshot.unwrap();
        // 100x50 model px at 2× capture density = 200x100 real pixels.
        assert_eq!((s.width, s.height), (200, 100));
    }
}
