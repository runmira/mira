//! Key-chord parsing.
//!
//! Models write chords xdotool-style (`ctrl+shift+t`, `Return`, `cmd+c`,
//! `Page_Down`) with plenty of aliasing. [`KeyChord::parse`] normalizes
//! that into modifiers plus one canonical key name, which each backend
//! renders into its own vocabulary.

use crate::ComputerError;

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Modifier {
    Ctrl,
    Shift,
    Alt,
    /// Command on macOS, Super/Windows elsewhere.
    Meta,
}

/// A parsed chord: zero or more modifiers plus an optional main key.
/// `key` is `None` for modifier-only chords (e.g. `hold_key shift`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyChord {
    pub modifiers: Vec<Modifier>,
    pub key: Option<Key>,
}

/// Canonical main key. Named keys use xdotool keysym spelling; single
/// printable characters are kept as-is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key {
    Named(&'static str),
    Char(char),
}

/// (aliases, canonical keysym). Canonical names are X11 keysyms because
/// the X11 backend passes them through verbatim; other backends map from
/// these.
const NAMED: &[(&[&str], &str)] = &[
    (&["return", "enter", "kp_enter"], "Return"),
    (&["escape", "esc"], "Escape"),
    (&["tab"], "Tab"),
    (&["backspace", "back_space"], "BackSpace"),
    (&["delete", "del"], "Delete"),
    (&["insert", "ins"], "Insert"),
    (&["space", "spacebar"], "space"),
    (&["up", "arrowup", "up_arrow"], "Up"),
    (&["down", "arrowdown", "down_arrow"], "Down"),
    (&["left", "arrowleft", "left_arrow"], "Left"),
    (&["right", "arrowright", "right_arrow"], "Right"),
    (&["home"], "Home"),
    (&["end"], "End"),
    (&["pageup", "page_up", "prior"], "Page_Up"),
    (&["pagedown", "page_down", "next"], "Page_Down"),
    (&["capslock", "caps_lock"], "Caps_Lock"),
    (&["f1"], "F1"),
    (&["f2"], "F2"),
    (&["f3"], "F3"),
    (&["f4"], "F4"),
    (&["f5"], "F5"),
    (&["f6"], "F6"),
    (&["f7"], "F7"),
    (&["f8"], "F8"),
    (&["f9"], "F9"),
    (&["f10"], "F10"),
    (&["f11"], "F11"),
    (&["f12"], "F12"),
    (&["minus"], "minus"),
    (&["plus"], "plus"),
    (&["equal", "equals"], "equal"),
    (&["comma"], "comma"),
    (&["period"], "period"),
    (&["slash"], "slash"),
];

fn modifier(token: &str) -> Option<Modifier> {
    Some(match token {
        "ctrl" | "control" | "control_l" | "control_r" | "ctl" => Modifier::Ctrl,
        "shift" | "shift_l" | "shift_r" => Modifier::Shift,
        "alt" | "alt_l" | "alt_r" | "option" | "opt" => Modifier::Alt,
        "cmd" | "command" | "super" | "super_l" | "super_r" | "meta" | "win" | "windows" => {
            Modifier::Meta
        }
        _ => return None,
    })
}

impl KeyChord {
    pub fn parse(chord: &str) -> Result<Self, ComputerError> {
        let chord = chord.trim();
        if chord.is_empty() {
            return Err(ComputerError::InvalidArgs("empty key chord".into()));
        }
        // A lone "+" is the plus key, not a separator.
        if chord == "+" {
            return Ok(Self {
                modifiers: vec![],
                key: Some(Key::Char('+')),
            });
        }
        let mut modifiers = Vec::new();
        let mut key = None;
        for token in chord.split('+').map(str::trim).filter(|t| !t.is_empty()) {
            let lower = token.to_ascii_lowercase();
            if let Some(m) = modifier(&lower) {
                if !modifiers.contains(&m) {
                    modifiers.push(m);
                }
                continue;
            }
            if key.is_some() {
                return Err(ComputerError::InvalidArgs(format!(
                    "`{chord}` names more than one non-modifier key; send one chord per `key` call"
                )));
            }
            key = Some(
                if let Some((_, canon)) = NAMED.iter().find(|(a, _)| a.contains(&lower.as_str())) {
                    Key::Named(canon)
                } else {
                    let mut chars = token.chars();
                    match (chars.next(), chars.next()) {
                        (Some(c), None) => Key::Char(c),
                        _ => {
                            return Err(ComputerError::InvalidArgs(format!(
                                "unknown key `{token}` in `{chord}`"
                            )))
                        }
                    }
                },
            );
        }
        modifiers.sort();
        Ok(Self { modifiers, key })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_aliases() {
        let c = KeyChord::parse("Control+Shift+t").unwrap();
        assert_eq!(c.modifiers, vec![Modifier::Ctrl, Modifier::Shift]);
        assert_eq!(c.key, Some(Key::Char('t')));

        assert_eq!(
            KeyChord::parse("enter").unwrap().key,
            Some(Key::Named("Return"))
        );
        assert_eq!(
            KeyChord::parse("cmd+c").unwrap().modifiers,
            vec![Modifier::Meta]
        );
        assert_eq!(
            KeyChord::parse("Page_Down").unwrap().key,
            Some(Key::Named("Page_Down"))
        );
        assert_eq!(KeyChord::parse("+").unwrap().key, Some(Key::Char('+')));
        assert_eq!(KeyChord::parse("shift").unwrap().key, None);
    }

    #[test]
    fn rejects_multi_key_and_unknown() {
        assert!(KeyChord::parse("a+b").is_err());
        assert!(KeyChord::parse("hyperdrive").is_err());
        assert!(KeyChord::parse("").is_err());
    }
}
