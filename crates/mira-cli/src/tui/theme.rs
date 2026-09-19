//! User-editable TUI color palette — `~/.mira/theme.yaml` + `/theme`.
//!
//! Ships with a warm cream + coral default that mirrors the landing
//! mockup. Users can either:
//!
//! * Edit `~/.mira/theme.yaml` directly — every key is optional; missing
//!   ones fall through to the built-in defaults so a partial file doesn't
//!   break booting.
//! * `/theme <name>` in the TUI — apply one of the bundled presets
//!   in-memory (see [`presets`]). Combine with `/theme save` to persist.
//! * `/theme reload` — re-read the yaml without restarting.
//!
//! Every read of a palette color reads from a [`RwLock`]-guarded global,
//! so `/theme <name>` re-paints the transcript on the very next render
//! tick without touching the source.
//!
//! Example yaml:
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
//! file never wedges the TUI.

use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};

use anyhow::{Context, Result};
use ratatui::style::Color;
use serde::{Deserialize, Serialize};
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
        DEFAULT
    }
}

/// Compile-time default palette. Also the `default` preset returned by
/// [`preset`]. Kept as a `const` so it can be sliced into `PRESETS`.
pub const DEFAULT: Theme = Theme {
    salmon: Color::Rgb(232, 156, 104),
    cream: Color::Rgb(240, 235, 226),
    muted: Color::Rgb(140, 130, 118),
    dim: Color::Rgb(96, 90, 82),
    hairline: Color::Rgb(160, 96, 60),
    prose: Color::Rgb(240, 235, 226),
};

/// Grayscale + white accents. For when you want the coral to shut up.
pub const MONO: Theme = Theme {
    salmon: Color::Rgb(220, 220, 220),
    cream: Color::Rgb(240, 240, 240),
    muted: Color::Rgb(140, 140, 140),
    dim: Color::Rgb(90, 90, 90),
    hairline: Color::Rgb(170, 170, 170),
    prose: Color::Rgb(240, 240, 240),
};

/// Indigo/violet accents on cool white — the Codex-CLI feel.
pub const MIDNIGHT: Theme = Theme {
    salmon: Color::Rgb(140, 130, 240),
    cream: Color::Rgb(230, 232, 245),
    muted: Color::Rgb(140, 145, 175),
    dim: Color::Rgb(85, 90, 120),
    hairline: Color::Rgb(120, 110, 220),
    prose: Color::Rgb(230, 232, 245),
};

/// Bundled preset roster. `name → (theme, one-line description)`.
/// Keep the first entry the default so `/theme` always lists it first.
pub const PRESETS: &[(&str, Theme, &str)] = &[
    ("default", DEFAULT, "warm cream + coral (landing mockup)"),
    ("mono", MONO, "grayscale + white accents"),
    ("midnight", MIDNIGHT, "indigo + cool white (Codex-CLI feel)"),
];

/// Look up a preset by name, case-insensitive. Returns the theme + its
/// description so `/theme <name>` can echo what just switched.
pub fn preset(name: &str) -> Option<(Theme, &'static str)> {
    let n = name.to_ascii_lowercase();
    PRESETS
        .iter()
        .find(|(pn, _, _)| pn.eq_ignore_ascii_case(&n))
        .map(|(_, t, d)| (*t, *d))
}

// ---- Live palette store ----------------------------------------------
//
// The palette is a global read-mostly value: written by `/theme <name>`
// or `/theme reload`, read by every `SALMON()` / `CREAM()` / … call on
// every render tick. `RwLock` from std is fine here — read locks are
// nearly-free on the uncontended path and the write path fires at most
// once per keystroke.

fn store() -> &'static RwLock<Theme> {
    static STORE: OnceLock<RwLock<Theme>> = OnceLock::new();
    STORE.get_or_init(|| RwLock::new(load()))
}

/// Snapshot of the currently-active theme. Cheap enough to call per
/// render — `Theme` is `Copy` so the read lock releases immediately.
pub fn current() -> Theme {
    *store().read().expect("theme lock poisoned")
}

/// Swap the live theme. The next render tick reads the new colors.
pub fn set(t: Theme) {
    *store().write().expect("theme lock poisoned") = t;
}

/// Re-read `~/.mira/theme.yaml` and swap the live theme to whatever it
/// says. `Ok(theme_path)` when the load succeeded (whether or not the
/// file existed — a missing file means defaults, which is a valid
/// outcome). `Err` when the file was present but malformed.
pub fn reload() -> Result<PathBuf> {
    let path = theme_path();
    if !path.exists() {
        set(Theme::default());
        return Ok(path);
    }
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("read {}", path.display()))?;
    let file: ThemeFile = serde_yaml::from_str(&raw)
        .with_context(|| format!("parse {}", path.display()))?;
    set(file.merge_over_defaults());
    Ok(path)
}

/// Serialize the currently-active theme back into `~/.mira/theme.yaml`
/// (creates the file + parent dir if missing). Every field is written
/// as `#RRGGBB` hex so the file round-trips cleanly through `reload`.
pub fn save_current_to_disk() -> Result<PathBuf> {
    let t = current();
    let path = theme_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("mkdir {}", parent.display()))?;
    }
    let out = ThemeFile::from(&t);
    let yaml = serde_yaml::to_string(&out).context("serialize theme")?;
    let banner = "# ~/.mira/theme.yaml — written by `/theme save`\n\
                  # Edit any key; delete a key to fall back to the default.\n\n";
    std::fs::write(&path, format!("{banner}{yaml}"))
        .with_context(|| format!("write {}", path.display()))?;
    Ok(path)
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

// ---- On-disk YAML shape ----------------------------------------------

/// On-disk shape. Every field is optional so a user can override just
/// one color without repeating the rest.
#[derive(Deserialize, Serialize, Default)]
struct ThemeFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    salmon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cream: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    muted: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dim: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hairline: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
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

impl From<&Theme> for ThemeFile {
    fn from(t: &Theme) -> Self {
        Self {
            salmon: Some(color_to_hex(t.salmon)),
            cream: Some(color_to_hex(t.cream)),
            muted: Some(color_to_hex(t.muted)),
            dim: Some(color_to_hex(t.dim)),
            hairline: Some(color_to_hex(t.hairline)),
            prose: Some(color_to_hex(t.prose)),
        }
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

/// Inverse of [`parse_hex`]. Anything not `Color::Rgb` (e.g. an ANSI
/// enum variant) falls through to `#000000` — this codebase only ever
/// stores truecolor palettes, but the fallback keeps the fn total.
fn color_to_hex(c: Color) -> String {
    match c {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        _ => "#000000".to_owned(),
    }
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

    #[test]
    fn preset_lookup_is_case_insensitive() {
        assert!(preset("default").is_some());
        assert!(preset("Default").is_some());
        assert!(preset("MONO").is_some());
        assert!(preset("nope").is_none());
    }

    #[test]
    fn color_to_hex_round_trips() {
        let c = Color::Rgb(0xab, 0xcd, 0xef);
        let s = color_to_hex(c);
        assert_eq!(s, "#abcdef");
        assert_eq!(parse_hex(&s).unwrap(), c);
    }

    #[test]
    fn set_then_current_returns_new_theme() {
        // Snapshot original, mutate, restore. Test order shouldn't leak.
        let before = current();
        set(MONO);
        assert_eq!(current().salmon, MONO.salmon);
        set(before);
    }
}
