//! Small text helpers.

/// Shorten `s` to at most `max` characters, adding `…` when cut.
pub fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_owned();
    }
    format!("{}…", &s[..max])
}

/// The last `n` lines of `s`.
pub fn tail(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    lines[lines.len() - n..].join("\n")
}
