//! User-editable TUI color palette — `~/.mira/theme.yaml`.
//!
//! Ships with a warm cream + coral default that mirrors the landing
//! mockup. Every key is optional; missing ones fall through to the
//! built-in defaults so a partial file doesn't break booting.
//!
//! Example:
//!
//! ```yaml
//! # ~/.mira/theme.yaml
//! salmon:   "#e89c68"    # coral accent — prompt, mode chips, headers
//! cream:    "#f0ebe2"    # prose / user text
//! muted:    "#8c8276"    # secondary text, hints
//! dim:      "#605a52"    # separators, gutter marks
//! hairline: "#a0603c"    # composer top border
//! prose:    "#f0ebe2"    # markdown paragraph text (defaults to cream)
//! ```
//!
//! Hex is the only accepted format — 6 hex chars, optional leading `#`.
//! Bad values log a warning and fall through to the default; a broken
//! file never wedges the TUI. Reads once at process start.

use std::path::PathBuf;

use ratatui::style::Color;
use serde::Deserialize;
use tracing::warn;

/// Built-in defaults. Each field's doc explains where it shows up.
#[derive(Clone, Copy, Debug)]
pub struct Theme {
    /// Coral accent — the `▸▸` prompt, mode chips, tool-call labels,
    /// section headers in the transcript. The Mira brand color.
    pub salmon: Color,
    /// Bright paper-white — user prompts, model IDs, tool-call
    /// arguments, first-run onboarding text.
    pub cream: Color,
    /// Secondary text — hints, footer, dim chips, metadata like
    /// `↑12.3k ↓4.1k`. Readable but recedes.
    pub muted: Color,
    /// Even quieter — separator glyphs (`└`, `│`), the code-block
    /// gutter, the plan-mode assistant gutter.
    pub dim: Color,
    /// The single-pixel top border above the composer.
    pub hairline: Color,
    /// Assistant markdown paragraphs. Split from `cream` only so a
    /// theme can tint prose distinctly (e.g. slightly warmer than
    /// user text). Defaults to `cream`.
    pub prose: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            salmon: Color::Rgb(232, 156, 104),
            cream: Color::Rgb(240, 235, 226),
            muted: Color::Rgb(140, 130, 118),
            dim: Color::Rgb(96, 90, 82),
            hairline: Color::Rgb(160, 96, 60),
            prose: Color::Rgb(240, 235, 226),
        }
    }
}

/// Load the user's theme, falling through per-key to the default.
/// Returns `Theme::default()` when the file doesn't exist or can't be
/// parsed. Logs a warning on parse failure so `mira doctor` can pick
/// it up, but never fails the TUI.
pub fn load() -> Theme {
    let path = theme_path();
    if !path.exists() {
        return Theme::default();
    }
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            warn!(%e, path = %path.display(), "theme: read failed, using defaults");
            return Theme::default();
        }
    };
    let file: ThemeFile = match serde_yaml::from_str(&raw) {
        Ok(f) => f,
        Err(e) => {
            warn!(%e, path = %path.display(), "theme: parse failed, using defaults");
            return Theme::default();
        }
    };
    file.merge_over_defaults()
}

/// Absolute path to `~/.mira/theme.yaml`.
pub fn theme_path() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(".mira")
        .join("theme.yaml")
}

/// On-disk shape. Every field is optional so a user can override just
/// one color without repeating the rest.
#[derive(Deserialize, Default)]
struct ThemeFile {
    #[serde(default)]
    salmon: Option<String>,
    #[serde(default)]
    cream: Option<String>,
    #[serde(default)]
    muted: Option<String>,
    #[serde(default)]
    dim: Option<String>,
    #[serde(default)]
    hairline: Option<String>,
    #[serde(default)]
    prose: Option<String>,
}

impl ThemeFile {
    fn merge_over_defaults(self) -> Theme {
        let mut t = Theme::default();
        // If `prose` isn't overridden and the user *did* override
        // `cream`, promote the new cream as the new prose default too.
        // The two colors are meant to move together unless the user
        // explicitly splits them.
        let cream_override = self.cream.as_deref().and_then(parse_hex);
        let prose_override = self.prose.as_deref().and_then(parse_hex);

        if let Some(c) = self.salmon.as_deref().and_then(parse_hex) {
            t.salmon = c;
        }
        if let Some(c) = cream_override {
            t.cream = c;
            if prose_override.is_none() {
                t.prose = c;
            }
        }
        if let Some(c) = self.muted.as_deref().and_then(parse_hex) {
            t.muted = c;
        }
        if let Some(c) = self.dim.as_deref().and_then(parse_hex) {
            t.dim = c;
        }
        if let Some(c) = self.hairline.as_deref().and_then(parse_hex) {
            t.hairline = c;
        }
        if let Some(c) = prose_override {
            t.prose = c;
        }
        t
    }
}

/// Parse `#RRGGBB` or `RRGGBB` into a truecolor `Color::Rgb`. Returns
/// `None` on any malformed input; the caller falls through to the
/// default so a typo doesn't break rendering.
fn parse_hex(s: &str) -> Option<Color> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        warn!(hex = s, "theme: hex must be 6 chars (RRGGBB)");
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some(Color::Rgb(r, g, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_accepts_with_and_without_hash() {
        assert!(matches!(parse_hex("#123456"), Some(Color::Rgb(0x12, 0x34, 0x56))));
        assert!(matches!(parse_hex("123456"), Some(Color::Rgb(0x12, 0x34, 0x56))));
    }

    #[test]
    fn parse_hex_rejects_wrong_length() {
        assert!(parse_hex("abc").is_none());
        assert!(parse_hex("#abcdefg").is_none());
    }

    #[test]
    fn missing_keys_fall_through_to_defaults() {
        let f: ThemeFile = serde_yaml::from_str("salmon: \"#ff0000\"\n").unwrap();
        let t = f.merge_over_defaults();
        let default = Theme::default();
        assert_eq!(t.salmon, Color::Rgb(0xff, 0, 0));
        assert_eq!(t.cream, default.cream);
        assert_eq!(t.muted, default.muted);
        assert_eq!(t.dim, default.dim);
    }

    #[test]
    fn cream_override_promotes_prose_when_prose_absent() {
        let f: ThemeFile = serde_yaml::from_str("cream: \"#123456\"\n").unwrap();
        let t = f.merge_over_defaults();
        assert_eq!(t.cream, Color::Rgb(0x12, 0x34, 0x56));
        assert_eq!(t.prose, Color::Rgb(0x12, 0x34, 0x56));
    }

    #[test]
    fn explicit_prose_wins_over_cream_promotion() {
        let f: ThemeFile =
            serde_yaml::from_str("cream: \"#123456\"\nprose: \"#654321\"\n").unwrap();
        let t = f.merge_over_defaults();
        assert_eq!(t.cream, Color::Rgb(0x12, 0x34, 0x56));
        assert_eq!(t.prose, Color::Rgb(0x65, 0x43, 0x21));
    }

    #[test]
    fn broken_hex_falls_through() {
        let f: ThemeFile = serde_yaml::from_str("salmon: \"not-hex\"\n").unwrap();
        let t = f.merge_over_defaults();
        assert_eq!(t.salmon, Theme::default().salmon);
    }
}
