//! Platform backends.
//!
//! A backend speaks *input space*: the coordinates the OS takes for mouse
//! events. Scaling to and from the model's screenshot space happens one
//! level up in [`crate::Computer`], so backends stay dumb.
//!
//! Backends drive the OS through its stock command-line tools (xdotool,
//! screencapture, osascript) rather than native bindings. That keeps
//! Mira's build free of X11 / CoreGraphics link dependencies, and the
//! trait is the seam where a native or remote (VM) backend slots in later.

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use image::RgbaImage;

use crate::action::{MouseButton, Point, ScrollDirection};
use crate::keys::KeyChord;
use crate::ComputerError;

pub mod macos;
pub mod mock;
pub mod x11;

/// What to do with a mouse button.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ButtonAction {
    /// Press and release `n` times in quick succession.
    Click(u8),
    Down,
    Up,
}

/// What to do with a key chord.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum KeyAction {
    Press,
    Down,
    Up,
}

#[async_trait]
pub trait ComputerBackend: Send + Sync {
    /// Short identifier for logs and `mira doctor` (`x11`, `macos`, …).
    fn name(&self) -> &'static str;

    /// Size of the input coordinate space (logical points).
    async fn screen_size(&self) -> Result<(u32, u32), ComputerError>;

    /// Capture the whole primary display at native resolution. May be
    /// larger than [`Self::screen_size`] on HiDPI displays.
    async fn capture(&self) -> Result<RgbaImage, ComputerError>;

    /// Move the pointer to `p` in input-space coordinates.
    async fn mouse_move(&self, p: Point) -> Result<(), ComputerError>;
    async fn button(&self, button: MouseButton, action: ButtonAction) -> Result<(), ComputerError>;
    /// Scroll at the current pointer position by `amount` wheel clicks.
    async fn scroll(&self, direction: ScrollDirection, amount: u32) -> Result<(), ComputerError>;
    async fn type_text(&self, text: &str) -> Result<(), ComputerError>;
    async fn key(&self, chord: &KeyChord, action: KeyAction) -> Result<(), ComputerError>;
    async fn cursor_position(&self) -> Result<Point, ComputerError>;
}

/// Pick the backend for the current platform, or explain why there is
/// none. Cheap: only inspects the environment, no subprocesses.
pub fn detect() -> Result<Box<dyn ComputerBackend>, ComputerError> {
    if cfg!(target_os = "macos") {
        return Ok(Box::new(macos::MacOs::new()));
    }
    if cfg!(target_os = "linux") || cfg!(target_os = "freebsd") {
        return x11::X11::from_env().map(|b| Box::new(b) as Box<dyn ComputerBackend>);
    }
    Err(ComputerError::Unsupported(format!(
        "computer use is not supported on {} yet (macOS and Linux/X11 only)",
        std::env::consts::OS
    )))
}

/// Default ceiling for a single helper-command invocation.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(20);

/// Run `program args…`, returning stdout. Non-zero exit becomes a
/// [`ComputerError::Command`] carrying stderr so the model (and user) see
/// *why* — e.g. macOS's "not allowed to send keystrokes".
pub(crate) async fn run(program: &str, args: &[&str]) -> Result<Vec<u8>, ComputerError> {
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let out = tokio::time::timeout(COMMAND_TIMEOUT, cmd.output())
        .await
        .map_err(|_| ComputerError::Command(format!("`{program}` timed out")))?
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ComputerError::MissingDependency(program.to_owned())
            } else {
                ComputerError::Command(format!("`{program}`: {e}"))
            }
        })?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(ComputerError::Command(format!(
            "`{program}` failed ({}): {}",
            out.status,
            stderr.trim()
        )));
    }
    Ok(out.stdout)
}

/// First directory on `PATH` containing an executable `name`.
pub fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

/// One readiness check, for `mira doctor`.
#[derive(Clone, Debug)]
pub struct Check {
    pub label: &'static str,
    pub ok: bool,
    pub detail: String,
}

/// Probe whether computer use can work here: backend, helper tools, and
/// on macOS the Screen Recording / Accessibility grants. Runs real
/// commands (cheap ones), so call it from diagnostics, not hot paths.
pub async fn preflight() -> Vec<Check> {
    let mut out = Vec::new();
    let backend = match detect() {
        Ok(b) => b,
        Err(e) => {
            out.push(Check {
                label: "backend",
                ok: false,
                detail: e.to_string(),
            });
            return out;
        }
    };
    out.push(Check {
        label: "backend",
        ok: true,
        detail: backend.name().to_owned(),
    });
    if backend.name() == "x11" {
        let xdotool = which("xdotool");
        out.push(Check {
            label: "xdotool",
            ok: xdotool.is_some(),
            detail: xdotool
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "not found — install xdotool".into()),
        });
        let shot = ["maim", "import", "xwd"]
            .into_iter()
            .find(|t| which(t).is_some());
        out.push(Check {
            label: "screenshot tool",
            ok: shot.is_some(),
            detail: shot
                .map(str::to_owned)
                .unwrap_or_else(|| "none found — install maim, imagemagick or x11-apps".into()),
        });
    }
    if backend.name() == "macos" {
        for (label, script, hint) in [
            (
                "screen recording",
                "ObjC.import('CoreGraphics'); $.CGPreflightScreenCaptureAccess();",
                "grant Screen Recording to your terminal app in System Settings → Privacy & Security",
            ),
            (
                "accessibility",
                "ObjC.import('ApplicationServices'); $.AXIsProcessTrusted();",
                "grant Accessibility to your terminal app in System Settings → Privacy & Security",
            ),
        ] {
            let res = run("osascript", &["-l", "JavaScript", "-e", script]).await;
            let granted = matches!(&res, Ok(o) if String::from_utf8_lossy(o).trim() == "true");
            out.push(Check {
                label,
                ok: granted,
                detail: if granted {
                    "granted".into()
                } else {
                    hint.into()
                },
            });
        }
    }
    match backend.screen_size().await {
        Ok((w, h)) => out.push(Check {
            label: "display",
            ok: true,
            detail: format!("{w}x{h}"),
        }),
        Err(e) => out.push(Check {
            label: "display",
            ok: false,
            detail: e.to_string(),
        }),
    }
    out
}
