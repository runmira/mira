//! A compact picture of a codebase: its source files, what each one
//! defines, and which definitions the rest of the code leans on.
//!
//! On a large repository an agent otherwise spends its first turns (and a
//! lot of tokens) grepping and listing directories just to find its way.
//! The map gives it that orientation up front, in the system prompt, and the
//! `repo_map` tool shows any part of it in more detail.
//!
//! In keeping with "everything is a file": definitions come from the same
//! outline as `file_outline`, ranked by how many *other* files mention each
//! name (a cheap stand-in for "how central is this"), and the result is
//! cached under `~/.mira/cache/repo-maps/` (outside the repository, so it
//! never shows up in `git status` or a commit). Only files whose size or
//! modification time changed are read again.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::builtin::file_outline::{is_outlined, outline_source};

/// The cached map of `root`: `~/.mira/cache/repo-maps/<hash of the path>.json`.
pub fn cache_path(root: &Path) -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let key = fnv1a(&root.to_string_lossy());
    Some(
        home.join(".mira/cache/repo-maps")
            .join(format!("{key:016x}.json")),
    )
}

/// Repositories with fewer source files than this don't get a map in the
/// system prompt: listing the directory is already cheap.
pub const MIN_FILES_FOR_PROMPT: usize = 25;

/// The map's share of the system prompt, in tokens (about 4 characters
/// each).
pub const PROMPT_TOKENS: usize = 2_000;

/// Files larger than this are skipped (generated code, vendored bundles).
const MAX_FILE_BYTES: u64 = 400 * 1024;

/// At most this many source files are mapped.
const MAX_FILES: usize = 20_000;

/// Identifiers shorter than this aren't counted as references: too many
/// accidental matches (`new`, `get`, `call`, `text`).
const MIN_REF_LEN: usize = 5;

/// Bumped when the cache format or what's extracted changes.
const CACHE_VERSION: u32 = 2;

/// Directories that are almost never worth mapping, when the repository
/// isn't a git checkout (git's own ignore rules cover them otherwise).
const SKIP_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    "__pycache__",
    ".venv",
    "venv",
    "vendor",
];

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Cache {
    version: u32,
    files: BTreeMap<String, FileEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct FileEntry {
    size: u64,
    modified_ns: u128,
    /// `(kind, name, line, container)`, as `file_outline` finds them.
    symbols: Vec<(String, String, u32, Option<String>)>,
    /// Hashes of the identifiers the file mentions, for counting references.
    idents: Vec<u64>,
}

/// The map of one repository.
#[derive(Clone, Debug)]
pub struct RepoMap {
    files: Vec<MappedFile>,
}

#[derive(Clone, Debug)]
struct MappedFile {
    path: String,
    /// Definitions, most referenced first: `(kind, name, references)`.
    symbols: Vec<(String, String, usize)>,
    score: usize,
}

impl RepoMap {
    /// Build (or refresh) the map of `root`, reading only files that
    /// changed since the cached map, and save the cache.
    pub fn build(root: &Path) -> std::io::Result<Self> {
        Self::build_cached(root, cache_path(root).as_deref())
    }

    /// [`RepoMap::build`] with the cache at `cache_path` (none: no cache).
    pub fn build_cached(root: &Path, cache_path: Option<&Path>) -> std::io::Result<Self> {
        let mut cache: Cache = cache_path
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .filter(|c: &Cache| c.version == CACHE_VERSION)
            .unwrap_or_default();
        let paths = source_files(root);
        let mut fresh = BTreeMap::new();
        let mut changed = false;
        for rel in paths {
            let full = root.join(&rel);
            let Ok(meta) = std::fs::metadata(&full) else {
                continue;
            };
            if meta.len() > MAX_FILE_BYTES {
                continue;
            }
            let modified_ns = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let key = rel.to_string_lossy().replace('\\', "/");
            match cache.files.remove(&key) {
                Some(entry) if entry.size == meta.len() && entry.modified_ns == modified_ns => {
                    fresh.insert(key, entry);
                }
                _ => {
                    let Ok(src) = std::fs::read_to_string(&full) else {
                        continue;
                    };
                    changed = true;
                    fresh.insert(
                        key,
                        FileEntry {
                            size: meta.len(),
                            modified_ns,
                            symbols: outline_source(&full, &src)
                                .into_iter()
                                .map(|(k, n, l, c)| (k.to_owned(), n, l, c))
                                .collect(),
                            idents: identifiers(&src),
                        },
                    );
                }
            }
        }
        // Files that disappeared count as a change too.
        changed |= !cache.files.is_empty();
        let cache = Cache {
            version: CACHE_VERSION,
            files: fresh,
        };
        if let Some(path) = cache_path.filter(|_| changed) {
            save(path, &cache);
        }
        Ok(Self::rank(&cache))
    }

    /// Number of mapped source files.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    fn rank(cache: &Cache) -> Self {
        // How many files mention each identifier.
        let mut mentions: HashMap<u64, usize> = HashMap::new();
        for entry in cache.files.values() {
            for id in &entry.idents {
                *mentions.entry(*id).or_default() += 1;
            }
        }
        // How many files define each name: a name defined all over
        // (`render`, `parse`) says little about where to look.
        let mut defined: HashMap<&str, usize> = HashMap::new();
        for entry in cache.files.values() {
            let names: HashSet<&str> = entry
                .symbols
                .iter()
                .filter(|s| mappable(s))
                .map(|s| s.1.as_str())
                .collect();
            for name in names {
                *defined.entry(name).or_default() += 1;
            }
        }
        let mut files: Vec<MappedFile> = cache
            .files
            .iter()
            .map(|(path, entry)| {
                let mut seen = HashSet::new();
                let mut symbols: Vec<(String, String, usize)> = entry
                    .symbols
                    .iter()
                    .filter(|s| mappable(s) && seen.insert(s.1.as_str()))
                    .map(|(kind, name, _, _)| {
                        // Mentions in *other* files (the defining file
                        // always mentions its own names), shared among the
                        // files defining the same name. Types count double:
                        // they're what code is organised around.
                        let refs = mentions
                            .get(&fnv1a(name))
                            .copied()
                            .unwrap_or(0)
                            .saturating_sub(1);
                        let weight = if is_type(kind) { 2 } else { 1 };
                        let shared = defined.get(name.as_str()).copied().unwrap_or(1).max(1);
                        (kind.clone(), name.clone(), refs * weight / shared)
                    })
                    .collect();
                symbols.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.1.cmp(&b.1)));
                let score = symbols.iter().take(5).map(|s| s.2).sum();
                MappedFile {
                    path: path.clone(),
                    symbols,
                    score,
                }
            })
            .collect();
        files.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.path.cmp(&b.path)));
        Self { files }
    }

    /// The map in about `max_tokens`, for files under `prefix` (all files
    /// when empty): the most-referenced files first in choosing what fits,
    /// shown grouped by directory, each with its most-referenced
    /// definitions.
    pub fn render(&self, prefix: &str, max_tokens: usize, symbols_per_file: usize) -> String {
        let prefix = prefix.trim_start_matches("./").trim_end_matches('/');
        let files: Vec<&MappedFile> = self
            .files
            .iter()
            .filter(|f| {
                prefix.is_empty() || f.path == prefix || f.path.starts_with(&format!("{prefix}/"))
            })
            .collect();
        if files.is_empty() {
            return format!("No source files under `{prefix}`.");
        }
        let budget = max_tokens * 4;
        let mut used = 0;
        let mut shown: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut shown_count = 0;
        for f in &files {
            let names: Vec<String> = f
                .symbols
                .iter()
                .take(symbols_per_file)
                .map(|(kind, name, _)| format!("{name} ({kind})"))
                .collect();
            let (dir, file) = match f.path.rsplit_once('/') {
                Some((d, n)) => (format!("{d}/"), n.to_owned()),
                None => (String::from("./"), f.path.clone()),
            };
            let line = if names.is_empty() {
                format!("  {file}")
            } else {
                format!("  {file}: {}", names.join(", "))
            };
            let cost = line.len()
                + if shown.contains_key(&dir) {
                    1
                } else {
                    dir.len() + 2
                };
            if used + cost > budget {
                continue;
            }
            used += cost;
            shown.entry(dir).or_default().push(line);
            shown_count += 1;
        }
        let mut out = String::new();
        for (dir, lines) in &shown {
            out.push_str(dir);
            out.push('\n');
            for line in lines {
                out.push_str(line);
                out.push('\n');
            }
        }
        let rest = files.len() - shown_count;
        if rest > 0 {
            let dirs: HashSet<&str> = files
                .iter()
                .map(|f| f.path.rsplit_once('/').map_or(".", |(d, _)| d))
                .collect();
            out.push_str(&format!(
                "(+{rest} more files; {} directories in all)\n",
                dirs.len()
            ));
        }
        out
    }

    /// The map for the system prompt, or `None` for a small repository.
    pub fn prompt_section(&self) -> Option<String> {
        if self.files.len() < MIN_FILES_FOR_PROMPT {
            return None;
        }
        Some(format!(
            "Repository map ({} source files). The files whose definitions the \
             rest of the code uses most, each with its most-used definitions. \
             Use it to go straight to the right file; `repo_map` shows any \
             directory in more detail and `file_outline` a single file.\n\n{}",
            self.files.len(),
            self.render("", PROMPT_TOKENS, 6)
        ))
    }
}

/// Whether a definition belongs on the map: top-level (members show with
/// `repo_map` on a directory), not an `impl` block, not test code.
fn mappable((kind, name, _, container): &(String, String, u32, Option<String>)) -> bool {
    container.is_none()
        && kind != "impl"
        && !(kind == "module" && matches!(name.as_str(), "tests" | "test"))
        && !name.starts_with("test_")
        && !name.starts_with("Test")
}

fn is_type(kind: &str) -> bool {
    matches!(
        kind,
        "struct" | "enum" | "trait" | "interface" | "class" | "type"
    )
}

/// The map's system-prompt section for `root`, if it's ready within
/// `wait`. A first build of a big repository can take longer: it carries on
/// in the background and saves its cache, so the next session has it.
///
/// Only for a git work tree, and never the home folder (where a session
/// with no project starts): mapping an arbitrary folder at every session
/// start would be slow and mostly noise. `MIRA_REPO_MAP=0` turns it off.
pub fn prompt_section_for(root: &Path, wait: Duration) -> Option<String> {
    if std::env::var("MIRA_REPO_MAP").is_ok_and(|v| matches!(v.as_str(), "0" | "off" | "false")) {
        return None;
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if home.is_some_and(|h| same_dir(&h, root)) || root.parent().is_none() {
        return None;
    }
    let root = root.to_path_buf();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("repo-map".into())
        .spawn(move || {
            if !in_git_work_tree(&root) {
                let _ = tx.send(None);
                return;
            }
            let _ = tx.send(RepoMap::build(&root).ok().and_then(|m| m.prompt_section()));
        })
        .ok()?;
    rx.recv_timeout(wait).ok().flatten()
}

fn in_git_work_tree(root: &Path) -> bool {
    std::process::Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(root)
        .output()
        .is_ok_and(|o| o.status.success() && o.stdout.starts_with(b"true"))
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Source files under `root`: what git tracks or would track (so its
/// ignore rules apply), else a walk that skips build directories.
fn source_files(root: &Path) -> Vec<PathBuf> {
    let listed = std::process::Command::new("git")
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .current_dir(root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            o.stdout
                .split(|b| *b == 0)
                .filter(|p| !p.is_empty())
                .map(|p| PathBuf::from(String::from_utf8_lossy(p).into_owned()))
                .collect::<Vec<_>>()
        });
    let mut files = match listed {
        Some(files) => files,
        None => {
            let mut out = Vec::new();
            walk(root, Path::new(""), &mut out);
            out
        }
    };
    files.retain(|p| is_outlined(p));
    files.sort();
    files.truncate(MAX_FILES);
    files
}

fn walk(root: &Path, rel: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root.join(rel)) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || SKIP_DIRS.contains(&name.as_ref()) {
            continue;
        }
        let path = rel.join(name.as_ref());
        match entry.file_type() {
            Ok(t) if t.is_dir() => walk(root, &path, out),
            Ok(t) if t.is_file() => out.push(path),
            _ => {}
        }
        if out.len() > MAX_FILES * 4 {
            return;
        }
    }
}

/// Hashes of the distinct identifiers in `src` long enough to count.
fn identifiers(src: &str) -> Vec<u64> {
    let mut seen = HashSet::new();
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            if i - start >= MIN_REF_LEN {
                seen.insert(fnv1a(&src[start..i]));
            }
        } else {
            i += 1;
        }
    }
    let mut out: Vec<u64> = seen.into_iter().collect();
    out.sort_unstable();
    out
}

/// FNV-1a: stable across runs and Rust versions, unlike `DefaultHasher`,
/// so cached hashes stay comparable.
fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn save(path: &Path, cache: &Cache) {
    let Some(dir) = path.parent() else { return };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let Ok(body) = serde_json::to_vec(cache) else {
        return;
    };
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, body).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    fn repo() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(root, "core/store.py", "class Store:\n    def save(self):\n        pass\n\ndef open_store():\n    return Store()\n");
        write(
            root,
            "core/util.py",
            "def rarely_used_helper():\n    pass\n",
        );
        for i in 0..4 {
            write(
                root,
                &format!("app/view{i}.py"),
                "from core.store import Store, open_store\n\ndef render():\n    return open_store()\n",
            );
        }
        write(root, "node_modules/lib/index.js", "function ignored() {}\n");
        write(root, "README.md", "# not code\n");
        tmp
    }

    #[test]
    fn the_most_used_definitions_come_first() {
        let tmp = repo();
        let map = RepoMap::build_cached(tmp.path(), Some(&tmp.path().join("cache.json"))).unwrap();
        // Six source files; build output and docs aren't mapped.
        assert_eq!(map.len(), 6);
        let text = map.render("", 2_000, 6);
        assert!(!text.contains("ignored"), "{text}");
        assert!(
            text.contains("core/\n  store.py: Store (class), open_store (function)"),
            "{text}"
        );
        // The file everything imports outranks the helper nobody uses.
        assert_eq!(map.files[0].path, "core/store.py");
        assert_eq!(map.files.last().unwrap().path, "core/util.py");
    }

    #[test]
    fn a_small_budget_keeps_the_central_files() {
        let tmp = repo();
        let map = RepoMap::build_cached(tmp.path(), Some(&tmp.path().join("cache.json"))).unwrap();
        let text = map.render("", 25, 6);
        assert!(text.contains("store.py"), "{text}");
        assert!(text.contains("more files"), "{text}");
        // And a directory can be shown on its own.
        let app = map.render("app", 2_000, 6);
        assert!(app.starts_with("app/\n"), "{app}");
        assert!(!app.contains("store.py"), "{app}");
    }

    #[test]
    fn only_changed_files_are_read_again() {
        let tmp = repo();
        RepoMap::build_cached(tmp.path(), Some(&tmp.path().join("cache.json"))).unwrap();
        let cached = std::fs::read_to_string(tmp.path().join("cache.json")).unwrap();
        assert!(cached.contains("open_store"));
        // An edit shows up on the next build.
        write(
            tmp.path(),
            "core/util.py",
            "def renamed_helper():\n    pass\n",
        );
        let map = RepoMap::build_cached(tmp.path(), Some(&tmp.path().join("cache.json"))).unwrap();
        let text = map.render("core", 2_000, 6);
        assert!(text.contains("renamed_helper"), "{text}");
        assert!(!text.contains("rarely_used_helper"), "{text}");
    }

    #[test]
    fn small_repos_get_no_prompt_section() {
        let tmp = repo();
        let map = RepoMap::build_cached(tmp.path(), Some(&tmp.path().join("cache.json"))).unwrap();
        assert!(map.prompt_section().is_none());
    }
}

#[cfg(test)]
mod try_it {
    /// `REPO_MAP_ROOT=/path cargo test -p mira-tools try_on_a_real_repo -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn try_on_a_real_repo() {
        let root = std::path::PathBuf::from(std::env::var("REPO_MAP_ROOT").unwrap());
        let cache = std::env::temp_dir().join("repo-map-try.json");
        for run in ["cold", "warm"] {
            let t = std::time::Instant::now();
            let map = super::RepoMap::build_cached(&root, Some(&cache)).unwrap();
            let section = map.prompt_section().unwrap_or_default();
            println!(
                "{run}: {} files in {:?}, prompt section {} chars",
                map.len(),
                t.elapsed(),
                section.len()
            );
            if run == "warm" {
                println!("{section}");
            }
        }
        let _ = std::fs::remove_file(cache);
    }
}
