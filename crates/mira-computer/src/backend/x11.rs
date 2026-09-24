//! Linux / X11 backend: `xdotool` for input, the first available of
//! `maim`, `import` (ImageMagick) or `xwd` for capture.
//!
//! Wayland sessions are refused up front: xdotool there only reaches
//! XWayland windows, and screenshots of a Wayland desktop need the
//! portal. Better an honest "unsupported" than clicks that land nowhere.

use async_trait::async_trait;
use image::RgbaImage;

use super::{run, which, ButtonAction, ComputerBackend, KeyAction};
use crate::action::{MouseButton, Point, ScrollDirection};
use crate::keys::{Key, KeyChord, Modifier};
use crate::{imaging, xwd, ComputerError};

/// Milliseconds between typed characters. xdotool's default (12) drops
/// characters in some Electron apps; 8–15 is the usual safe band.
const TYPE_DELAY_MS: &str = "12";

pub struct X11 {
    _private: (),
}

impl X11 {
    pub fn from_env() -> Result<Self, ComputerError> {
        let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
        if session.eq_ignore_ascii_case("wayland") {
            return Err(ComputerError::Unsupported(
                "Wayland sessions aren't supported yet; log into an X11 session (or run Mira \
                 against an Xvfb display via DISPLAY=:N)"
                    .into(),
            ));
        }
        if std::env::var_os("DISPLAY").is_none() {
            return Err(ComputerError::Unsupported(
                "no X display: DISPLAY is not set".into(),
            ));
        }
        Ok(Self { _private: () })
    }

    async fn xdotool(&self, args: &[&str]) -> Result<Vec<u8>, ComputerError> {
        run("xdotool", args).await
    }
}

fn button_number(b: MouseButton) -> &'static str {
    match b {
        MouseButton::Left => "1",
        MouseButton::Middle => "2",
        MouseButton::Right => "3",
    }
}

/// Render a chord in xdotool syntax (`ctrl+shift+t`).
pub(crate) fn xdotool_chord(chord: &KeyChord) -> String {
    let mut parts: Vec<String> = chord
        .modifiers
        .iter()
        .map(|m| {
            match m {
                Modifier::Ctrl => "ctrl",
                Modifier::Shift => "shift",
                Modifier::Alt => "alt",
                Modifier::Meta => "super",
            }
            .to_owned()
        })
        .collect();
    match &chord.key {
        Some(Key::Named(n)) => parts.push((*n).to_owned()),
        Some(Key::Char(c)) => parts.push(match c {
            ' ' => "space".to_owned(),
            '+' => "plus".to_owned(),
            '-' => "minus".to_owned(),
            c => c.to_string(),
        }),
        None => {}
    }
    parts.join("+")
}

#[async_trait]
impl ComputerBackend for X11 {
    fn name(&self) -> &'static str {
        "x11"
    }

    async fn screen_size(&self) -> Result<(u32, u32), ComputerError> {
        let out = self.xdotool(&["getdisplaygeometry"]).await?;
        let text = String::from_utf8_lossy(&out);
        let mut it = text.split_whitespace().map(str::parse::<u32>);
        match (it.next(), it.next()) {
            (Some(Ok(w)), Some(Ok(h))) => Ok((w, h)),
            _ => Err(ComputerError::Command(format!(
                "unexpected `xdotool getdisplaygeometry` output: {text:?}"
            ))),
        }
    }

    async fn capture(&self) -> Result<RgbaImage, ComputerError> {
        if which("maim").is_some() {
            return imaging::decode(&run("maim", &["--hidecursor"]).await?);
        }
        if which("import").is_some() {
            return imaging::decode(
                &run("import", &["-silent", "-window", "root", "png:-"]).await?,
            );
        }
        match run("xwd", &["-root", "-silent"]).await {
            Ok(bytes) => xwd::decode(&bytes),
            Err(ComputerError::MissingDependency(_)) => Err(ComputerError::MissingDependency(
                "a screenshot tool (install one of: maim, imagemagick, x11-apps)".into(),
            )),
            Err(e) => Err(e),
        }
    }

    async fn mouse_move(&self, p: Point) -> Result<(), ComputerError> {
        self.xdotool(&["mousemove", &p.x.to_string(), &p.y.to_string()])
            .await
            .map(drop)
    }

    async fn button(&self, button: MouseButton, action: ButtonAction) -> Result<(), ComputerError> {
        let b = button_number(button);
        match action {
            ButtonAction::Click(n) => {
                let n = n.max(1).to_string();
                self.xdotool(&["click", "--repeat", &n, "--delay", "80", b])
                    .await
            }
            ButtonAction::Down => self.xdotool(&["mousedown", b]).await,
            ButtonAction::Up => self.xdotool(&["mouseup", b]).await,
        }
        .map(drop)
    }

    async fn scroll(&self, direction: ScrollDirection, amount: u32) -> Result<(), ComputerError> {
        let b = match direction {
            ScrollDirection::Up => "4",
            ScrollDirection::Down => "5",
            ScrollDirection::Left => "6",
            ScrollDirection::Right => "7",
        };
        let n = amount.max(1).to_string();
        self.xdotool(&["click", "--repeat", &n, "--delay", "30", b])
            .await
            .map(drop)
    }

    async fn type_text(&self, text: &str) -> Result<(), ComputerError> {
        self.xdotool(&["type", "--delay", TYPE_DELAY_MS, "--", text])
            .await
            .map(drop)
    }

    async fn key(&self, chord: &KeyChord, action: KeyAction) -> Result<(), ComputerError> {
        let rendered = xdotool_chord(chord);
        let verb = match action {
            KeyAction::Press => "key",
            KeyAction::Down => "keydown",
            KeyAction::Up => "keyup",
        };
        self.xdotool(&[verb, "--", &rendered]).await.map(drop)
    }

    async fn cursor_position(&self) -> Result<Point, ComputerError> {
        let out = self.xdotool(&["getmouselocation", "--shell"]).await?;
        let text = String::from_utf8_lossy(&out);
        let get = |k: &str| {
            text.lines()
                .find_map(|l| l.strip_prefix(k))
                .and_then(|v| v.trim().parse::<i32>().ok())
        };
        match (get("X="), get("Y=")) {
            (Some(x), Some(y)) => Ok(Point { x, y }),
            _ => Err(ComputerError::Command(format!(
                "unexpected `xdotool getmouselocation` output: {text:?}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_xdotool_chords() {
        let c = |s| xdotool_chord(&KeyChord::parse(s).unwrap());
        assert_eq!(c("ctrl+shift+t"), "ctrl+shift+t");
        assert_eq!(c("cmd+space"), "super+space");
        assert_eq!(c("Enter"), "Return");
        assert_eq!(c("ctrl+-"), "ctrl+minus");
    }
}
