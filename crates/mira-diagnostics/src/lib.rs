//! Problem reports without telemetry.
//!
//! - [`bundle`]: `mira doctor --bundle` and the web UI's "Report a
//!   problem" collect version, platform, config, recent logs and a chosen
//!   session into a zip the user reviews before sharing.
//! - [`crash`]: an off-by-default panic hook that saves crash reports
//!   locally; the user is shown each one and asked before it goes anywhere.
//! - [`redact`]: what both use to strip keys, tokens and the home path.

pub mod bundle;
pub mod crash;
pub mod redact;

use std::path::PathBuf;

pub use bundle::{Bundle, BundleFile, BundleOptions};
pub use redact::Redactor;

pub const ISSUES_URL: &str = "https://github.com/runmira/mira/issues/new";

/// GitHub truncates very long prefilled URLs; past this the body is cut.
const MAX_ISSUE_BODY: usize = 6000;

/// `~/.mira`, next to `mira.yaml`.
pub fn mira_dir() -> PathBuf {
    mira_config::global_path()
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".mira"))
}

/// A "new issue" link with the title and body filled in. Opening it sends
/// nothing: the user sees the text in GitHub's editor and submits it.
pub fn issue_url(title: &str, body: &str) -> String {
    let body = if body.len() > MAX_ISSUE_BODY {
        let mut cut = MAX_ISSUE_BODY;
        while !body.is_char_boundary(cut) {
            cut -= 1;
        }
        format!(
            "{}\n…(cut; the full text is in the file)\n```",
            &body[..cut]
        )
    } else {
        body.to_string()
    };
    format!(
        "{ISSUES_URL}?title={}&body={}",
        percent_encode(title),
        percent_encode(&body)
    )
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_urls_are_encoded_and_bounded() {
        let url = issue_url("a b&c", "line\n«x»");
        assert_eq!(
            url,
            format!("{ISSUES_URL}?title=a%20b%26c&body=line%0A%C2%ABx%C2%BB")
        );
        let long = issue_url("t", &"é".repeat(MAX_ISSUE_BODY));
        assert!(long.contains("cut%3B"));
    }
}
