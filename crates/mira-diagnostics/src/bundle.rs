//! The problem-report bundle: everything a maintainer needs to reproduce
//! a bug, collected into one zip with secrets removed.
//!
//! Collection and packing are separate steps so a UI can show the exact
//! files before anything is written: [`Bundle::collect`] returns the
//! contents in memory, [`Bundle::to_zip`] packs those same bytes.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::redact::Redactor;

/// How much of each log to keep. The end of a log is what matters; a
/// day-long server log would otherwise make the bundle too big to attach.
const LOG_TAIL: u64 = 256 * 1024;
const MCP_LOG_TAIL: u64 = 64 * 1024;
const AGENT_EVENTS_TAIL: u64 = 1024 * 1024;
/// Messages kept from the session record. The failure is almost always
/// near the end, and older turns are mostly the user's own code.
const SESSION_MESSAGES: usize = 200;
const CRASH_REPORTS: usize = 5;

pub struct BundleOptions {
    /// `~/.mira`.
    pub mira_dir: PathBuf,
    /// The project folder, for its `.mira/config.yaml`.
    pub cwd: Option<PathBuf>,
    /// Include this session's record and agent events.
    pub session_id: Option<String>,
    /// Files the caller adds, e.g. the CLI's `doctor.txt`. Redacted too.
    pub extra: Vec<BundleFile>,
    pub redactor: Redactor,
    /// What built the bundle: `cli` or `web`.
    pub source: &'static str,
}

impl BundleOptions {
    pub fn new(redactor: Redactor, source: &'static str) -> Self {
        Self {
            mira_dir: crate::mira_dir(),
            cwd: None,
            session_id: None,
            extra: Vec::new(),
            redactor,
            source,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct BundleFile {
    /// Path inside the zip.
    pub name: String,
    pub contents: String,
    /// What was left out, e.g. "last 256 KB of 4.1 MB".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl BundleFile {
    pub fn new(name: impl Into<String>, contents: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            contents: contents.into(),
            note: None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Bundle {
    pub files: Vec<BundleFile>,
}

impl Bundle {
    pub fn collect(opts: BundleOptions) -> Self {
        let red = &opts.redactor;
        let mut files = vec![BundleFile::new("README.txt", README)];
        files.push(BundleFile::new(
            "about.json",
            red.text(&about(opts.source).to_string()),
        ));

        for mut f in opts.extra {
            f.contents = red.text(&f.contents);
            files.push(f);
        }

        // ---- config ----
        let configs = [
            ("config/mira.yaml", opts.mira_dir.join("mira.yaml")),
            ("config/state.yaml", opts.mira_dir.join("state.yaml")),
        ];
        for (name, path) in configs {
            if let Ok(raw) = std::fs::read_to_string(&path) {
                files.push(BundleFile::new(name, red.yaml(&raw)));
            }
        }
        if let Some(cwd) = &opts.cwd {
            if let Ok(raw) = std::fs::read_to_string(cwd.join(".mira").join("config.yaml")) {
                files.push(BundleFile::new("config/project.yaml", red.yaml(&raw)));
            }
        }

        // ---- logs ----
        let logs = opts.mira_dir.join("logs");
        for (dir, prefix, cap) in [
            (logs.clone(), "logs", LOG_TAIL),
            (logs.join("mcp"), "logs/mcp", MCP_LOG_TAIL),
        ] {
            for path in sorted_files(&dir) {
                if let Some((text, note)) = tail(&path, cap) {
                    files.push(BundleFile {
                        name: format!("{prefix}/{}", file_name(&path)),
                        contents: red.text(&text),
                        note,
                    });
                }
            }
        }

        // ---- crash reports ----
        let mut crashes = sorted_files(&crate::crash::crash_dir_in(&opts.mira_dir));
        crashes.reverse();
        for path in crashes.into_iter().take(CRASH_REPORTS) {
            if let Ok(text) = std::fs::read_to_string(&path) {
                files.push(BundleFile::new(
                    format!("crashes/{}", file_name(&path)),
                    red.text(&text),
                ));
            }
        }

        // ---- the failing session ----
        if let Some(id) = opts.session_id.as_deref().filter(|id| is_session_id(id)) {
            let dir = opts.mira_dir.join("sessions");
            if let Ok(raw) = std::fs::read_to_string(dir.join(format!("{id}.json"))) {
                let (text, note) = trim_session(&raw);
                files.push(BundleFile {
                    name: format!("session/{id}.json"),
                    contents: red.text(&text),
                    note,
                });
            }
            if let Some((text, note)) =
                tail(&dir.join(format!("{id}.agent.jsonl")), AGENT_EVENTS_TAIL)
            {
                files.push(BundleFile {
                    name: format!("session/{id}.agent.jsonl"),
                    contents: red.text(&text),
                    note,
                });
            }
        }

        Self { files }
    }

    pub fn total_bytes(&self) -> usize {
        self.files.iter().map(|f| f.contents.len()).sum()
    }

    pub fn to_zip(&self) -> std::io::Result<Vec<u8>> {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for f in &self.files {
            zip.start_file(f.name.as_str(), opts)
                .map_err(std::io::Error::other)?;
            zip.write_all(f.contents.as_bytes())?;
        }
        Ok(zip.finish().map_err(std::io::Error::other)?.into_inner())
    }

    /// `mira-diagnostics-20261008-224501.zip`.
    pub fn file_name() -> String {
        format!(
            "mira-diagnostics-{}.zip",
            chrono::Local::now().format("%Y%m%d-%H%M%S")
        )
    }
}

const README: &str = "\
Mira diagnostics bundle

Made by `mira doctor --bundle` or the web UI's \"Report a problem\".
Nothing in it has been sent anywhere. Look through it before you share it.

API keys, tokens, passwords and other secret-looking values were replaced
with «redacted», and your home folder with ~. If you spot anything private
that slipped through, delete it from the file before sharing, and please
mention it in your report so the redaction can be fixed.

  about.json     Mira version and platform
  doctor.txt     `mira doctor` checks (CLI bundles only)
  config/        mira.yaml, state.yaml and the project's .mira/config.yaml
  logs/          the end of each log in ~/.mira/logs
  crashes/       crash reports, if crash reports are on
  session/       the chat you chose to include, if any
";

fn about(source: &str) -> serde_json::Value {
    serde_json::json!({
        "mira_version": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
        "os_version": os_version(),
        "arch": std::env::consts::ARCH,
        "created_at": chrono::Local::now().to_rfc3339(),
        "source": source,
    })
}

pub(crate) fn os_version() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("sw_vers")
            .arg("-productVersion")
            .output()
            .ok()?;
        let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (!v.is_empty()).then(|| format!("macOS {v}"))
    }
    #[cfg(target_os = "linux")]
    {
        let raw = std::fs::read_to_string("/etc/os-release").ok()?;
        raw.lines()
            .find_map(|l| l.strip_prefix("PRETTY_NAME="))
            .map(|v| v.trim_matches('"').to_string())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

/// Session ids are generated (`sess_<hex>`), so anything else is refused
/// rather than joined onto a path.
pub fn is_session_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn sorted_files(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && !file_name(p).starts_with('.'))
        .collect();
    out.sort();
    out
}

/// The last `cap` bytes of a file, starting at a line boundary.
fn tail(path: &Path, cap: u64) -> Option<(String, Option<String>)> {
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let start = len.saturating_sub(cap);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    f.read_to_end(&mut bytes).ok()?;
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if start == 0 {
        return Some((text, None));
    }
    if let Some(nl) = text.find('\n') {
        text.drain(..=nl);
    }
    let note = format!("last {} of {}", human(cap), human(len));
    Some((text, Some(note)))
}

/// Keep the record's settings and the last [`SESSION_MESSAGES`] messages.
fn trim_session(raw: &str) -> (String, Option<String>) {
    let Ok(mut v) = serde_json::from_str::<serde_json::Value>(raw) else {
        return (raw.to_string(), None);
    };
    let mut note = None;
    if let Some(msgs) = v.get_mut("messages").and_then(|m| m.as_array_mut()) {
        let total = msgs.len();
        if total > SESSION_MESSAGES {
            msgs.drain(..total - SESSION_MESSAGES);
            note = Some(format!("last {SESSION_MESSAGES} of {total} messages"));
        }
    }
    let text = serde_json::to_string_pretty(&v).unwrap_or_else(|_| raw.to_string());
    (text, note)
}

pub fn human(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b >= KB * KB {
        format!("{:.1} MB", b / KB / KB)
    } else if b >= KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(dir: &Path) -> BundleOptions {
        BundleOptions {
            mira_dir: dir.to_path_buf(),
            cwd: None,
            session_id: None,
            extra: Vec::new(),
            redactor: Redactor::default(),
            source: "test",
        }
    }

    fn names(b: &Bundle) -> Vec<&str> {
        b.files.iter().map(|f| f.name.as_str()).collect()
    }

    #[test]
    fn collects_config_logs_and_the_chosen_session_without_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::write(
            d.join("mira.yaml"),
            "providers:\n  openai:\n    api_key: sk-abcdefghijklmnopqrstuv\n",
        )
        .unwrap();
        std::fs::create_dir_all(d.join("logs/mcp")).unwrap();
        std::fs::write(
            d.join("logs/desktop-server.log"),
            "INFO start\nERROR Authorization: Bearer abcdefghijklmnop\n",
        )
        .unwrap();
        std::fs::write(d.join("logs/mcp/github.log"), "boom\n").unwrap();
        std::fs::create_dir_all(d.join("sessions")).unwrap();
        std::fs::write(
            d.join("sessions/sess_1.json"),
            r#"{"id":"sess_1","messages":[{"role":"user","content":"my token=abc12345"}]}"#,
        )
        .unwrap();
        std::fs::write(d.join("sessions/sess_1.agent.jsonl"), "{\"t\":1}\n").unwrap();
        std::fs::write(d.join("sessions/sess_2.json"), "{}").unwrap();

        let mut o = opts(d);
        o.session_id = Some("sess_1".into());
        o.extra
            .push(BundleFile::new("doctor.txt", "[OK] api_key=xyzxyzxyz"));
        let b = Bundle::collect(o);

        assert_eq!(
            names(&b),
            [
                "README.txt",
                "about.json",
                "doctor.txt",
                "config/mira.yaml",
                "logs/desktop-server.log",
                "logs/mcp/github.log",
                "session/sess_1.json",
                "session/sess_1.agent.jsonl",
            ]
        );
        let all: String = b.files.iter().map(|f| f.contents.as_str()).collect();
        for secret in ["sk-abcdef", "abcdefghijklmnop", "abc12345", "xyzxyzxyz"] {
            assert!(!all.contains(secret), "{secret} leaked");
        }
        assert!(all.contains("ERROR Authorization"));

        // The zip holds the same files.
        let zip = b.to_zip().unwrap();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip)).unwrap();
        assert_eq!(archive.len(), b.files.len());
        let mut s = String::new();
        archive
            .by_name("config/mira.yaml")
            .unwrap()
            .read_to_string(&mut s)
            .unwrap();
        assert_eq!(s, b.files[3].contents);
    }

    #[test]
    fn refuses_session_ids_that_are_paths() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("secret.json"), "{}").unwrap();
        std::fs::create_dir_all(dir.path().join("sessions")).unwrap();
        let mut o = opts(dir.path());
        o.session_id = Some("../secret".into());
        let b = Bundle::collect(o);
        assert!(!names(&b).iter().any(|n| n.starts_with("session/")));
        assert!(is_session_id("sess_00ab"));
        assert!(!is_session_id("a/b"));
        assert!(!is_session_id(""));
    }

    #[test]
    fn long_logs_keep_their_end_from_a_line_start() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.log");
        let line = "0123456789\n".repeat(100);
        std::fs::write(&path, format!("{line}LAST\n")).unwrap();
        let (text, note) = tail(&path, 50).unwrap();
        assert!(text.ends_with("LAST\n"));
        assert!(text.starts_with("0123456789\n"));
        assert!(note.unwrap().starts_with("last 50 B of"));
    }

    #[test]
    fn long_sessions_keep_the_last_messages() {
        let msgs: Vec<_> = (0..250).map(|i| serde_json::json!({ "n": i })).collect();
        let raw = serde_json::json!({ "id": "s", "messages": msgs }).to_string();
        let (text, note) = trim_session(&raw);
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        let kept = v["messages"].as_array().unwrap();
        assert_eq!(kept.len(), SESSION_MESSAGES);
        assert_eq!(kept[0]["n"], 50);
        assert_eq!(note.as_deref(), Some("last 200 of 250 messages"));
    }
}
