//! Editor detection + "open in editor".
//!
//! `GET  /api/editors`      → `{ editors: [{ id, name, kind }], default_id }`
//! `POST /api/editors/open` → body: `{ path, editor_id? }` — opens the path
//!                              in the chosen app (or the first detected
//!                              editor). Always detached; never blocks.
//!
//! Detection is best-effort and local-machine only: macOS scans
//! /Applications (+ ~/Applications) for known bundles and falls back to
//! PATH binaries; Linux checks PATH binaries (+ a few .desktop entries);
//! Windows checks PATH plus well-known install dirs. A file-manager
//! reveal entry and a Terminal entry are always appended. Editors the
//! scan can't see are still openable — the UI offers the curated list
//! regardless, and `open` reports a clear error when the binary/app
//! is missing.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::state::AppState;

#[derive(Debug, Clone)]
struct KnownEditor {
    id: &'static str,
    name: &'static str,
    /// macOS bundle name, e.g. `Visual Studio Code.app`.
    mac_app: Option<&'static str>,
    /// CLI binaries probed via PATH (first hit wins).
    bins: &'static [&'static str],
    /// Windows executable names probed via PATH.
    win_exes: &'static [&'static str],
    /// Well-known Windows install dirs (joined with the exe name).
    win_dirs: &'static [&'static str],
}

const KNOWN_EDITORS: &[KnownEditor] = &[
    KnownEditor { id: "vscode", name: "VS Code", mac_app: Some("Visual Studio Code.app"), bins: &["code"], win_exes: &["code.cmd", "code.exe"], win_dirs: &["Microsoft VS Code\\bin"] },
    KnownEditor { id: "cursor", name: "Cursor", mac_app: Some("Cursor.app"), bins: &["cursor"], win_exes: &["cursor.exe"], win_dirs: &["Cursor"] },
    KnownEditor { id: "zed", name: "Zed", mac_app: Some("Zed.app"), bins: &["zed"], win_exes: &["zed.exe"], win_dirs: &["Zed"] },
    KnownEditor { id: "windsurf", name: "Windsurf", mac_app: Some("Windsurf.app"), bins: &["windsurf"], win_exes: &["windsurf.exe"], win_dirs: &["Windsurf"] },
    KnownEditor { id: "antigravity", name: "Antigravity", mac_app: Some("Antigravity.app"), bins: &["antigravity"], win_exes: &["antigravity.exe"], win_dirs: &["Antigravity"] },
    KnownEditor { id: "sublime", name: "Sublime Text", mac_app: Some("Sublime Text.app"), bins: &["subl", "sublime_text"], win_exes: &["subl.exe"], win_dirs: &["Sublime Text"] },
    KnownEditor { id: "nova", name: "Nova", mac_app: Some("Nova.app"), bins: &[], win_exes: &[], win_dirs: &[] },
    KnownEditor { id: "textmate", name: "TextMate", mac_app: Some("TextMate.app"), bins: &["mate"], win_exes: &[], win_dirs: &[] },
    KnownEditor { id: "fleet", name: "Fleet", mac_app: Some("Fleet.app"), bins: &["fleet"], win_exes: &["fleet.exe"], win_dirs: &["Fleet"] },
    KnownEditor { id: "idea", name: "IntelliJ IDEA", mac_app: Some("IntelliJ IDEA.app"), bins: &["idea"], win_exes: &["idea64.exe"], win_dirs: &["JetBrains\\IntelliJ IDEA"] },
];

#[derive(Debug, Clone, Serialize)]
pub struct EditorEntry {
    pub id: String,
    pub name: String,
    /// `app` (mac bundle), `binary` (PATH exe), or `system` (file
    /// manager / terminal — always present).
    pub kind: String,
    /// The server can serve this editor's real app icon
    /// (`GET /api/editors/icon/:id`).
    pub has_icon: bool,
}

fn path_dirs() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|v| std::env::split_paths(&v).collect())
        .unwrap_or_default()
}

/// `true` when `name` resolves to an executable file on PATH.
fn in_path(name: &str) -> bool {
    #[cfg(windows)]
    let names: Vec<String> = {
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".exe") || lower.ends_with(".cmd") || lower.ends_with(".bat") {
            vec![name.to_owned()]
        } else {
            vec![format!("{name}.exe"), format!("{name}.cmd"), format!("{name}.bat")]
        }
    };
    #[cfg(not(windows))]
    let names = vec![name.to_owned()];

    path_dirs().iter().any(|dir| {
        names.iter().any(|n| {
            let p = dir.join(n);
            p.is_file()
        })
    })
}

fn mac_app_path(bundle: &str) -> Option<PathBuf> {
    let roots = [
        PathBuf::from("/Applications"),
        dirs_home_join("Applications"),
    ];
    roots.into_iter().map(|r| r.join(bundle)).find(|p| p.is_dir())
}

fn dirs_home_join(child: &str) -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
        .join(child)
}

fn windows_known_path(dir: &str, exe: &str) -> Option<PathBuf> {
    let bases = [
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
        std::env::var_os("ProgramFiles").map(PathBuf::from),
        std::env::var_os("ProgramFiles(x86)").map(PathBuf::from),
    ];
    bases.into_iter().flatten().map(|b| b.join(dir).join(exe)).find(|p| p.is_file())
}

fn linux_desktop_present(desktop: &str) -> bool {
    let mut roots = vec![PathBuf::from("/usr/share/applications")];
    roots.push(dirs_home_join(".local/share/applications"));
    roots.into_iter().any(|r| r.join(desktop).is_file())
}

/// Probe every known editor; returns (detected entries, all-by-id map input).
fn detect() -> Vec<EditorEntry> {
    let mut out = Vec::new();
    let os = std::env::consts::OS;

    for ed in KNOWN_EDITORS {
        let mut present = false;
        let mut kind = "binary";
        if os == "macos" {
            if let Some(bundle) = ed.mac_app {
                if mac_app_path(bundle).is_some() {
                    present = true;
                    kind = "app";
                }
            }
        }        if !present && os == "windows" {
            for exe in ed.win_exes {
                if in_path(exe) {
                    present = true;
                    break;
                }
            }
            if !present {
                'outer: for dir in ed.win_dirs {
                    for exe in ed.win_exes {
                        if windows_known_path(dir, exe).is_some() {
                            present = true;
                            break 'outer;
                        }
                    }
                }
            }
        }
        if !present {
            for bin in ed.bins {
                if in_path(bin) {
                    present = true;
                    break;
                }
            }
        }
        if !present && os == "linux" {
            for desktop in [
                "code.desktop",
                "cursor.desktop",
                "zed.desktop",
                "sublime_text.desktop",
                "windsurf.desktop",
            ] {
                if desktop.starts_with(&ed.id.replace("sublime", "sublime_text"))
                    && linux_desktop_present(desktop)
                {
                    present = true;
                    break;
                }
            }
        }
        if present {
            out.push(EditorEntry {
                id: ed.id.to_owned(),
                name: ed.name.to_owned(),
                kind: kind.to_owned(),
                has_icon: os == "macos" && icon_icns_for(ed.id).is_some(),
            });
        }
    }

    // Always-present system entries.
    let (fm_id, fm_name) = match os {
        "macos" => ("finder", "Finder"),
        "windows" => ("explorer", "File Explorer"),
        _ => ("files", "Files"),
    };
    out.push(EditorEntry { id: fm_id.to_owned(), name: fm_name.to_owned(), kind: "system".to_owned(), has_icon: os == "macos" && icon_icns_for(fm_id).is_some() });
    out.push(EditorEntry { id: "terminal".to_owned(), name: "Terminal".to_owned(), kind: "system".to_owned(), has_icon: os == "macos" && icon_icns_for("terminal").is_some() });
    out
}

/// .app bundle backing an editor id (known mac bundles + system apps).
fn app_bundle_for(id: &str) -> Option<PathBuf> {
    if id == "finder" {
        let p = PathBuf::from("/System/Library/CoreServices/Finder.app");
        if p.is_dir() {
            return Some(p);
        }
    }
    if id == "terminal" {
        for p in [
            PathBuf::from("/System/Applications/Utilities/Terminal.app"),
            PathBuf::from("/Applications/Utilities/Terminal.app"),
        ] {
            if p.is_dir() {
                return Some(p);
            }
        }
        return None;
    }
    let ed = KNOWN_EDITORS.iter().find(|e| e.id == id)?;
    let bundle = ed.mac_app?;
    mac_app_path(bundle)
}

/// Best `.icns` inside a bundle's Resources: prefer an AppIcon match,
/// then the largest file, then whatever comes first.
fn icon_icns_for(id: &str) -> Option<PathBuf> {
    let resources = app_bundle_for(id)?.join("Contents").join("Resources");
    let entries: Vec<PathBuf> = std::fs::read_dir(&resources)
        .ok()?
        .filter_map(|e| e.ok().map(|x| x.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "icns"))
        .collect();
    if entries.is_empty() {
        return None;
    }
    if let Some(icon) = entries.iter().find(|p| {
        p.file_stem()
            .is_some_and(|s| s.to_string_lossy().to_lowercase().contains("appicon"))
    }) {
        return Some(icon.clone());
    }
    entries
        .into_iter()
        .max_by_key(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0))
}

/// Rendered icon cache (id → PNG bytes).
fn icon_cache() -> &'static std::sync::Mutex<std::collections::HashMap<String, Vec<u8>>> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, Vec<u8>>>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// GET /api/editors/icon/:id — the editor's real app icon as PNG
/// (converted from its bundle `.icns` via `sips`). macOS only.
pub async fn editor_icon(axum::extract::Path(id): axum::extract::Path<String>) -> Response {
    if std::env::consts::OS != "macos" {
        return err(StatusCode::NOT_FOUND, "app icons are only available on macOS".to_owned());
    }
    if let Some(bytes) = icon_cache().lock().ok().and_then(|c| c.get(&id).cloned()) {
        return (
            [(axum::http::header::CONTENT_TYPE, "image/png")],
            bytes,
        )
            .into_response();
    }
    let Some(icns) = icon_icns_for(&id) else {
        return err(StatusCode::NOT_FOUND, format!("no icon for editor: {id}"));
    };
    let tmp = std::env::temp_dir().join(format!("mira-editor-icon-{id}.png"));
    let status = Command::new("sips")
        .args(["-s", "format", "png", "-z", "64", "64"])
        .arg(&icns)
        .arg("--out")
        .arg(&tmp)
        .output();
    let bytes = match status {
        Ok(out) if out.status.success() => std::fs::read(&tmp).ok(),
        _ => None,
    };
    let _ = std::fs::remove_file(&tmp);
    match bytes {
        Some(b) => {
            if let Ok(mut c) = icon_cache().lock() {
                c.insert(id, b.clone());
            }
            ([(axum::http::header::CONTENT_TYPE, "image/png")], b).into_response()
        }
        None => err(StatusCode::INTERNAL_SERVER_ERROR, "icon conversion failed".to_owned()),
    }
}

/// GET /api/editors — detected editors plus the curated full list's ids
/// the client can offer regardless (`all`).
#[derive(Debug, Serialize)]
pub struct EditorsListView {
    pub editors: Vec<EditorEntry>,
    pub default_id: Option<String>,
    pub all: Vec<EditorEntry>,
}

pub async fn list_editors(State(_state): State<AppState>) -> Response {
    let detected = detect();
    let seen: std::collections::HashSet<&str> = detected.iter().map(|e| e.id.as_str()).collect();
    let mut all: Vec<EditorEntry> = detected.clone();
    for ed in KNOWN_EDITORS {
        if !seen.contains(ed.id) {
            all.push(EditorEntry {
                id: ed.id.to_owned(),
                name: ed.name.to_owned(),
                kind: "binary".to_owned(),
                has_icon: false,
            });
        }
    }
    let default_id = detected
        .iter()
        .find(|e| e.kind == "app" || e.kind == "binary")
        .map(|e| e.id.clone());
    Json(EditorsListView { editors: detected, default_id, all }).into_response()
}

#[derive(Deserialize)]
pub struct OpenEditorRequest {
    pub path: String,
    pub editor_id: Option<String>,
}

fn err(status: StatusCode, msg: String) -> Response {
    (status, Json(serde_json::json!({ "error": msg }))).into_response()
}

fn spawn_detached(cmd: &mut Command) -> std::io::Result<()> {
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    cmd.spawn().map(|_| ())
}

/// POST /api/editors/open { path, editor_id? } — open a file or folder in
/// an editor. Relative paths resolve against the session cwd.
pub async fn open_in_editor(
    State(state): State<AppState>,
    Json(req): Json<OpenEditorRequest>,
) -> Response {
    let cwd = state.current_cwd().await;
    let raw = Path::new(&req.path);
    let path = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        cwd.join(raw)
    };
    if !path.exists() {
        return err(StatusCode::NOT_FOUND, format!("no such file: {}", path.display()));
    }
    let os = std::env::consts::OS;
    let id = req.editor_id.as_deref().unwrap_or("");

    // System entries.
    if id == "finder" || id == "explorer" || id == "files" {
        let mut cmd = match os {
            "macos" => {
                let mut c = Command::new("open");
                if path.is_file() {
                    c.arg("-R");
                }
                c
            }
            "windows" => {
                let mut c = Command::new("explorer");
                if path.is_file() {
                    c.arg(format!("/select,{}", path.display()));
                    return match spawn_detached(&mut c) {
                        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
                        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
                    };
                }
                c
            }
            _ => {
                let c = Command::new("xdg-open");
                c
            }
        };
        // Directories open as-is; on mac an explicit dir arg is fine too.
        if !(os == "windows" && path.is_file()) {
            cmd.arg(&path);
        }
        return match spawn_detached(&mut cmd) {
            Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
            Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        };
    }
    if id == "terminal" {
        let dir = if path.is_file() {
            path.parent().map(Path::to_path_buf).unwrap_or_else(|| cwd.clone())
        } else {
            path.clone()
        };
        let mut cmd = match os {
            "macos" => {
                let mut c = Command::new("open");
                c.arg("-a").arg("Terminal").arg(&dir);
                c
            }
            "windows" => {
                let mut c = Command::new("cmd");
                c.args(["/C", "start", "", &dir.to_string_lossy()]);
                c
            }
            _ => {
                // First terminal emulator found on PATH wins.
                let term = ["x-terminal-emulator", "gnome-terminal", "konsole", "alacritty", "kitty", "wezterm", "foot"]
                    .into_iter()
                    .find(|t| in_path(t));
                match term {
                    Some(t) => {
                        let mut c = Command::new(t);
                        if t == "gnome-terminal" || t == "konsole" {
                            c.arg("--working-directory").arg(&dir);
                        } else {
                            c.current_dir(&dir);
                        }
                        c
                    }
                    None => {
                        return err(
                            StatusCode::NOT_FOUND,
                            "no terminal emulator found on PATH".to_owned(),
                        )
                    }
                }
            }
        };
        let _ = &mut cmd;
        return match spawn_detached(&mut cmd) {
            Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
            Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        };
    }

    // Known editors.
    let Some(ed) = KNOWN_EDITORS.iter().find(|e| e.id == id) else {
        return err(StatusCode::BAD_REQUEST, format!("unknown editor: {id}"));
    };
    if os == "macos" {
        if let Some(bundle) = ed.mac_app {
            if let Some(app_path) = mac_app_path(bundle) {
                let mut cmd = Command::new("open");
                cmd.arg("-a").arg(&app_path).arg(&path);
                return match spawn_detached(&mut cmd) {
                    Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
                    Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
                };
            }
        }
    }
    if os == "windows" {
        for exe in ed.win_exes {
            if in_path(exe) {
                let mut cmd = Command::new(exe);
                cmd.arg(&path);
                return match spawn_detached(&mut cmd) {
                    Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
                    Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
                };
            }
            for dir in ed.win_dirs {
                if let Some(full) = windows_known_path(dir, exe) {
                    let mut cmd = Command::new(full);
                    cmd.arg(&path);
                    return match spawn_detached(&mut cmd) {
                        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
                        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
                    };
                }
            }
        }
        return err(StatusCode::NOT_FOUND, format!("{} is not installed", ed.name));
    }
    for bin in ed.bins {
        if in_path(bin) {
            let mut cmd = Command::new(bin);
            cmd.arg(&path);
            return match spawn_detached(&mut cmd) {
                Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
                Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
            };
        }
    }
    err(StatusCode::NOT_FOUND, format!("{} is not installed", ed.name))
}
