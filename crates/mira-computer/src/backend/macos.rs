//! macOS backend: `screencapture` for capture, CoreGraphics events posted
//! from JavaScript for Automation (`osascript -l JavaScript`) for input.
//!
//! Needs two privacy grants for the app Mira runs inside (Terminal,
//! iTerm, …): **Screen Recording** — without it `screencapture` returns
//! only the wallpaper — and **Accessibility**, without which posted
//! events are silently dropped. `mira doctor` checks both.
//!
//! Input space is logical points (what CGEvent takes); captures come
//! back at backing-pixel resolution (2× on Retina). The scaling layer
//! reconciles the two.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use async_trait::async_trait;
use image::RgbaImage;

use super::{run, ButtonAction, ComputerBackend, KeyAction};
use crate::action::{MouseButton, Point, ScrollDirection};
use crate::keys::{Key, KeyChord, Modifier};
use crate::{imaging, ComputerError};

// CGEventType values. Numeric, because JXA's bridging of the enum
// constants is unreliable across macOS releases.
const LEFT_DOWN: u32 = 1;
const LEFT_UP: u32 = 2;
const RIGHT_DOWN: u32 = 3;
const RIGHT_UP: u32 = 4;
const MOVED: u32 = 5;
const LEFT_DRAGGED: u32 = 6;
const OTHER_DOWN: u32 = 25;
const OTHER_UP: u32 = 26;

// CGEventFlags masks.
const FLAG_SHIFT: u64 = 0x0002_0000;
const FLAG_CTRL: u64 = 0x0004_0000;
const FLAG_ALT: u64 = 0x0008_0000;
const FLAG_CMD: u64 = 0x0010_0000;

/// JXA prelude shared by every input script.
const PRELUDE: &str = "ObjC.import('CoreGraphics');\n\
function post(e){ $.CGEventPost(0, e); }\n\
function mouse(t,x,y,b,n){ var e=$.CGEventCreateMouseEvent(null,t,{x:x,y:y},b); if(n){ $.CGEventSetIntegerValueField(e,1,n); } post(e); }\n\
function key(c,down,flags){ var e=$.CGEventCreateKeyboardEvent(null,c,down); $.CGEventSetFlags(e,flags); post(e); }\n\
function pause(ms){ delay(ms/1000); }\n";

pub struct MacOs {
    /// Whether the left button is held (so moves post drag events).
    left_down: AtomicBool,
    shot_seq: AtomicU64,
}

impl MacOs {
    pub fn new() -> Self {
        Self {
            left_down: AtomicBool::new(false),
            shot_seq: AtomicU64::new(0),
        }
    }

    async fn jxa(&self, body: &str) -> Result<String, ComputerError> {
        let script = format!("{PRELUDE}{body}");
        let out = run("osascript", &["-l", "JavaScript", "-e", &script]).await?;
        Ok(String::from_utf8_lossy(&out).trim().to_owned())
    }

    async fn current_point(&self) -> Result<Point, ComputerError> {
        self.cursor_position().await
    }
}

impl Default for MacOs {
    fn default() -> Self {
        Self::new()
    }
}

fn flags(mods: &[Modifier]) -> u64 {
    mods.iter()
        .map(|m| match m {
            Modifier::Shift => FLAG_SHIFT,
            Modifier::Ctrl => FLAG_CTRL,
            Modifier::Alt => FLAG_ALT,
            Modifier::Meta => FLAG_CMD,
        })
        .fold(0, |a, b| a | b)
}

fn modifier_keycode(m: Modifier) -> u16 {
    match m {
        Modifier::Meta => 55,
        Modifier::Shift => 56,
        Modifier::Alt => 58,
        Modifier::Ctrl => 59,
    }
}

/// ANSI (US) virtual keycode for a key, plus whether shift is implied
/// (uppercase letters, shifted symbols).
pub(crate) fn keycode(key: &Key) -> Option<(u16, bool)> {
    let named = |n: &str| -> Option<u16> {
        Some(match n {
            "Return" => 36,
            "Tab" => 48,
            "space" => 49,
            "BackSpace" => 51,
            "Escape" => 53,
            "Caps_Lock" => 57,
            "Delete" => 117,
            "Insert" => 114, // Help — closest thing Mac keyboards have
            "Home" => 115,
            "End" => 119,
            "Page_Up" => 116,
            "Page_Down" => 121,
            "Left" => 123,
            "Right" => 124,
            "Down" => 125,
            "Up" => 126,
            "F1" => 122,
            "F2" => 120,
            "F3" => 99,
            "F4" => 118,
            "F5" => 96,
            "F6" => 97,
            "F7" => 98,
            "F8" => 100,
            "F9" => 101,
            "F10" => 109,
            "F11" => 103,
            "F12" => 111,
            "minus" => 27,
            "equal" => 24,
            "comma" => 43,
            "period" => 47,
            "slash" => 44,
            "plus" => return None, // handled as shifted '='
            _ => return None,
        })
    };
    let ch = |c: char| -> Option<(u16, bool)> {
        let base = |c: char| -> Option<u16> {
            Some(match c {
                'a' => 0,
                's' => 1,
                'd' => 2,
                'f' => 3,
                'h' => 4,
                'g' => 5,
                'z' => 6,
                'x' => 7,
                'c' => 8,
                'v' => 9,
                'b' => 11,
                'q' => 12,
                'w' => 13,
                'e' => 14,
                'r' => 15,
                'y' => 16,
                't' => 17,
                '1' => 18,
                '2' => 19,
                '3' => 20,
                '4' => 21,
                '6' => 22,
                '5' => 23,
                '=' => 24,
                '9' => 25,
                '7' => 26,
                '-' => 27,
                '8' => 28,
                '0' => 29,
                ']' => 30,
                'o' => 31,
                'u' => 32,
                '[' => 33,
                'i' => 34,
                'p' => 35,
                'l' => 37,
                'j' => 38,
                '\'' => 39,
                'k' => 40,
                ';' => 41,
                '\\' => 42,
                ',' => 43,
                '/' => 44,
                'n' => 45,
                'm' => 46,
                '.' => 47,
                '`' => 50,
                ' ' => 49,
                _ => return None,
            })
        };
        if let Some(k) = base(c) {
            return Some((k, false));
        }
        if c.is_ascii_uppercase() {
            return base(c.to_ascii_lowercase()).map(|k| (k, true));
        }
        let unshifted = match c {
            '!' => '1',
            '@' => '2',
            '#' => '3',
            '$' => '4',
            '%' => '5',
            '^' => '6',
            '&' => '7',
            '*' => '8',
            '(' => '9',
            ')' => '0',
            '_' => '-',
            '+' => '=',
            '{' => '[',
            '}' => ']',
            '|' => '\\',
            ':' => ';',
            '"' => '\'',
            '<' => ',',
            '>' => '.',
            '?' => '/',
            '~' => '`',
            _ => return None,
        };
        base(unshifted).map(|k| (k, true))
    };
    match key {
        Key::Named("plus") => ch('+'),
        Key::Named(n) => named(n).map(|k| (k, false)),
        Key::Char(c) => ch(*c),
    }
}

#[async_trait]
impl ComputerBackend for MacOs {
    fn name(&self) -> &'static str {
        "macos"
    }

    async fn screen_size(&self) -> Result<(u32, u32), ComputerError> {
        let out = self
            .jxa("ObjC.import('AppKit'); var f=$.NSScreen.mainScreen.frame; JSON.stringify([f.size.width, f.size.height]);")
            .await?;
        let v: Vec<f64> = serde_json::from_str(&out).map_err(|_| {
            ComputerError::Command(format!("unexpected screen size output: {out:?}"))
        })?;
        match v.as_slice() {
            [w, h] => Ok((*w as u32, *h as u32)),
            _ => Err(ComputerError::Command(format!(
                "unexpected screen size output: {out:?}"
            ))),
        }
    }

    async fn capture(&self) -> Result<RgbaImage, ComputerError> {
        let n = self.shot_seq.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("mira-shot-{}-{n}.png", std::process::id()));
        let path_str = path.to_string_lossy().into_owned();
        // -x: no shutter sound; -m: main display only; -C: no cursor.
        let res = run("screencapture", &["-x", "-m", "-t", "png", &path_str]).await;
        let bytes = tokio::fs::read(&path).await;
        let _ = tokio::fs::remove_file(&path).await;
        res?;
        let bytes = bytes
            .map_err(|e| ComputerError::Command(format!("screencapture wrote no file: {e}")))?;
        imaging::decode(&bytes)
    }

    async fn mouse_move(&self, p: Point) -> Result<(), ComputerError> {
        let (t, b) = if self.left_down.load(Ordering::Relaxed) {
            (LEFT_DRAGGED, 0)
        } else {
            (MOVED, 0)
        };
        self.jxa(&format!("mouse({t},{},{},{b},0);", p.x, p.y))
            .await
            .map(drop)
    }

    async fn button(&self, button: MouseButton, action: ButtonAction) -> Result<(), ComputerError> {
        let p = self.current_point().await?;
        let (down, up, b) = match button {
            MouseButton::Left => (LEFT_DOWN, LEFT_UP, 0),
            MouseButton::Right => (RIGHT_DOWN, RIGHT_UP, 1),
            MouseButton::Middle => (OTHER_DOWN, OTHER_UP, 2),
        };
        let script = match action {
            ButtonAction::Click(n) => (1..=n.max(1))
                .map(|i| {
                    format!(
                        "mouse({down},{x},{y},{b},{i}); mouse({up},{x},{y},{b},{i}); pause(40);",
                        x = p.x,
                        y = p.y
                    )
                })
                .collect::<String>(),
            ButtonAction::Down => format!("mouse({down},{},{},{b},1);", p.x, p.y),
            ButtonAction::Up => format!("mouse({up},{},{},{b},1);", p.x, p.y),
        };
        self.jxa(&script).await?;
        if button == MouseButton::Left {
            match action {
                ButtonAction::Down => self.left_down.store(true, Ordering::Relaxed),
                ButtonAction::Up | ButtonAction::Click(_) => {
                    self.left_down.store(false, Ordering::Relaxed)
                }
            }
        }
        Ok(())
    }

    async fn scroll(&self, direction: ScrollDirection, amount: u32) -> Result<(), ComputerError> {
        let n = amount.max(1) as i32;
        // wheel1 = vertical (positive = up), wheel2 = horizontal (positive = left).
        let (dy, dx) = match direction {
            ScrollDirection::Up => (n, 0),
            ScrollDirection::Down => (-n, 0),
            ScrollDirection::Left => (0, n),
            ScrollDirection::Right => (0, -n),
        };
        // CGEventCreateScrollWheelEvent is variadic, which JXA can't call;
        // the `2` variant (macOS 10.13+) takes all three wheels explicitly.
        self.jxa(&format!(
            "post($.CGEventCreateScrollWheelEvent2(null, 1, 2, {dy}, {dx}, 0));"
        ))
        .await
        .map(drop)
    }

    async fn type_text(&self, text: &str) -> Result<(), ComputerError> {
        // Text travels as argv, never spliced into the script, so quotes
        // and backslashes in it can't break (or inject into) AppleScript.
        run(
            "osascript",
            &[
                "-e",
                "on run argv",
                "-e",
                "tell application \"System Events\" to keystroke (item 1 of argv)",
                "-e",
                "end run",
                "--",
                text,
            ],
        )
        .await
        .map(drop)
    }

    async fn key(&self, chord: &KeyChord, action: KeyAction) -> Result<(), ComputerError> {
        let mut mods = chord.modifiers.clone();
        let main = match &chord.key {
            Some(k) => {
                let (code, shift) = keycode(k).ok_or_else(|| {
                    ComputerError::InvalidArgs(format!("no macOS keycode for {k:?}"))
                })?;
                if shift && !mods.contains(&Modifier::Shift) {
                    mods.push(Modifier::Shift);
                }
                Some(code)
            }
            None => None,
        };
        let f = flags(&mods);
        let mod_events = |down: bool| -> String {
            mods.iter()
                .map(|m| format!("key({},{down},{f});", modifier_keycode(*m)))
                .collect()
        };
        let script = match (action, main) {
            (KeyAction::Press, Some(code)) => format!(
                "{}key({code},true,{f}); key({code},false,{f}); {}",
                mod_events(true),
                mod_events(false)
            ),
            (KeyAction::Down, Some(code)) => format!("{}key({code},true,{f});", mod_events(true)),
            (KeyAction::Up, Some(code)) => format!("key({code},false,{f}); {}", mod_events(false)),
            (KeyAction::Press, None) => format!("{}{}", mod_events(true), mod_events(false)),
            (KeyAction::Down, None) => mod_events(true),
            (KeyAction::Up, None) => mod_events(false),
        };
        self.jxa(&script).await.map(drop)
    }

    async fn cursor_position(&self) -> Result<Point, ComputerError> {
        let out = self
            .jxa("var p=$.CGEventGetLocation($.CGEventCreate(null)); JSON.stringify([p.x, p.y]);")
            .await?;
        let v: Vec<f64> = serde_json::from_str(&out)
            .map_err(|_| ComputerError::Command(format!("unexpected cursor output: {out:?}")))?;
        match v.as_slice() {
            [x, y] => Ok(Point {
                x: x.round() as i32,
                y: y.round() as i32,
            }),
            _ => Err(ComputerError::Command(format!(
                "unexpected cursor output: {out:?}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keycodes_cover_common_chords() {
        assert_eq!(keycode(&Key::Char('c')), Some((8, false)));
        assert_eq!(keycode(&Key::Char('C')), Some((8, true)));
        assert_eq!(keycode(&Key::Char('!')), Some((18, true)));
        assert_eq!(keycode(&Key::Named("Return")), Some((36, false)));
        assert_eq!(keycode(&Key::Named("plus")), Some((24, true)));
        assert_eq!(keycode(&Key::Char('é')), None);
        assert_eq!(
            flags(&[Modifier::Meta, Modifier::Shift]),
            FLAG_CMD | FLAG_SHIFT
        );
    }
}
