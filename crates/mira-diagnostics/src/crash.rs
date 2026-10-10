//! Opt-in crash reports.
//!
//! Mira has no telemetry, so "reporting" a crash only ever means saving a
//! file the user can read. With `diagnostics.crash_reports: true`, a panic
//! writes `~/.mira/crashes/crash-<time>.txt` with secrets removed. The CLI
//! then shows each report in full and asks before opening a GitHub issue
//! with it; a report is marked reviewed whatever the answer, so it's asked
//! about once.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::redact::Redactor;

/// Reports kept on disk. A crash loop shouldn't fill the disk.
const KEEP: usize = 20;
const REVIEWED: &str = ".reviewed.txt";

pub fn crash_dir() -> PathBuf {
    crash_dir_in(&crate::mira_dir())
}

pub fn crash_dir_in(mira_dir: &Path) -> PathBuf {
    mira_dir.join("crashes")
}

static REDACTOR: OnceLock<Redactor> = OnceLock::new();

/// Save a report on every panic, then run the previous hook (which prints
/// the usual message). Does nothing unless the user turned reports on.
pub fn install_panic_hook(cfg: &mira_config::MiraConfig) {
    if !cfg.diagnostics.crash_reports() {
        return;
    }
    // Collected now: reading config and the environment inside a panic
    // hook is more work than a crashing process should do.
    let _ = REDACTOR.set(Redactor::from_env_and_config(cfg));
    let dir = crash_dir();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "(non-string panic payload)".into());
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_default();
        let thread = std::thread::current()
            .name()
            .unwrap_or("unnamed")
            .to_string();
        let report = render(&message, &location, &thread, &backtrace.to_string());
        let report = REDACTOR.get().map(|r| r.text(&report)).unwrap_or(report);
        let _ = save(&dir, &report);
        previous(info);
    }));
}

fn render(message: &str, location: &str, thread: &str, backtrace: &str) -> String {
    format!(
        "Mira crash report\n\nversion: {}\nos: {} {}\nos_version: {}\ntime: {}\nthread: {thread}\nlocation: {location}\n\npanic: {message}\n\nbacktrace:\n{backtrace}\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        crate::bundle::os_version().unwrap_or_default(),
        chrono::Local::now().to_rfc3339(),
    )
}

fn save(dir: &Path, report: &str) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    // Nanoseconds keep two panics in the same second apart.
    let path = dir.join(format!(
        "crash-{}.txt",
        chrono::Local::now().format("%Y%m%d-%H%M%S-%f")
    ));
    mira_config::write_private(&path, report.as_bytes())?;
    prune(dir);
    Ok(path)
}

fn prune(dir: &Path) {
    let mut all = list(dir);
    if all.len() > KEEP {
        all.sort();
        for p in &all[..all.len() - KEEP] {
            let _ = std::fs::remove_file(p);
        }
    }
}

fn list(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("crash-") && n.ends_with(".txt"))
        })
        .collect()
}

#[derive(Clone, Debug)]
pub struct CrashReport {
    pub path: PathBuf,
    pub contents: String,
}

impl CrashReport {
    /// The first line of the panic, for an issue title.
    pub fn summary(&self) -> String {
        self.contents
            .lines()
            .find_map(|l| l.strip_prefix("panic: "))
            .unwrap_or("crash")
            .chars()
            .take(80)
            .collect()
    }

    pub fn mark_reviewed(&self) -> std::io::Result<()> {
        let name = self
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        let reviewed = name.trim_end_matches(".txt").to_string() + REVIEWED;
        std::fs::rename(&self.path, self.path.with_file_name(reviewed))
    }

    pub fn issue_url(&self) -> String {
        crate::issue_url(
            &format!("Crash: {}", self.summary()),
            &format!(
                "<!-- Mira saved this crash report on your machine. Check it before submitting. -->\n\n**What were you doing when it crashed?**\n\n\n```\n{}\n```\n",
                self.contents.trim()
            ),
        )
    }
}

/// Reports the user hasn't been asked about yet, oldest first.
pub fn pending() -> Vec<CrashReport> {
    pending_in(&crash_dir())
}

pub fn pending_in(dir: &Path) -> Vec<CrashReport> {
    let mut paths: Vec<PathBuf> = list(dir)
        .into_iter()
        .filter(|p| !p.to_string_lossy().ends_with(REVIEWED))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .filter_map(|path| {
            let contents = std::fs::read_to_string(&path).ok()?;
            Some(CrashReport { path, contents })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_reports_are_pending_until_reviewed() {
        let dir = tempfile::tempdir().unwrap();
        let report = render("index out of bounds", "src/a.rs:1:2", "main", "bt");
        save(dir.path(), &report).unwrap();
        let pending = pending_in(dir.path());
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].summary(), "index out of bounds");
        assert!(pending[0].issue_url().contains("index%20out%20of%20bounds"));
        pending[0].mark_reviewed().unwrap();
        assert!(pending_in(dir.path()).is_empty());
        // Reviewed reports stay on disk, for the bundle.
        assert_eq!(list(dir.path()).len(), 1);
    }

    #[test]
    fn keeps_only_the_newest_reports() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..KEEP + 5 {
            std::fs::write(dir.path().join(format!("crash-{i:04}.txt")), "x").unwrap();
        }
        prune(dir.path());
        let mut left = list(dir.path());
        left.sort();
        assert_eq!(left.len(), KEEP);
        assert!(left[0].ends_with("crash-0005.txt"));
    }
}
