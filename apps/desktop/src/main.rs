//! Mira desktop: the web UI in a native window.
//!
//! The app bundles the `mira` binary as a sidecar and runs `mira serve` on
//! a loopback port, then points its window at it. Everything the web UI
//! does (sessions, worktrees, environments, settings) works unchanged; the
//! shell adds what a browser tab can't: a native window, sign-in through
//! the system browser (`mira://` deep link), links that open outside the
//! app, and your login shell's PATH so tools like `git` and `cargo` resolve
//! when launched from the Dock.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::io::{Read, Seek, SeekFrom};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{
    AppHandle, Emitter, Manager, RunEvent, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_opener::OpenerExt;

/// Preferred port. Stable on purpose: the UI keeps its sign-in session and
/// preferences in the page's storage, which is scoped to the origin, port
/// included. Differs from `mira serve`'s 8787 so both can run at once.
const PREFERRED_PORT: u16 = 8797;
/// How many ports after the preferred one to try when it's taken.
const PORT_ATTEMPTS: u16 = 20;
/// How long `mira serve` gets to start listening.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(60);
/// Event the page listens for with the `mira://auth-callback?...` URL.
const AUTH_EVENT: &str = "mira-auth-callback";

/// Runs in every page before its own scripts, the loading page and the
/// server's UI alike. Marks the page as running in the desktop app and
/// sends new-window links to the system browser, since the webview has no
/// tabs to open them in.
const INIT_SCRIPT: &str = r#"
(function () {
  window.__MIRA_DESKTOP__ = true;
  function openExternal(url) {
    try {
      var abs = new URL(url, window.location.href).toString();
      window.__TAURI__.core.invoke('open_external', { url: abs });
    } catch (e) {
      console.error('mira: could not open', url, e);
    }
  }
  window.open = function (url) {
    if (url) openExternal(String(url));
    return null;
  };
  document.addEventListener('click', function (e) {
    var a = e.target && e.target.closest ? e.target.closest('a[href]') : null;
    if (!a) return;
    var target = (a.getAttribute('target') || '').toLowerCase();
    var href = a.href;
    var external = /^https?:/i.test(href) && new URL(href).origin !== window.location.origin;
    if (target === '_blank' || external) {
      e.preventDefault();
      openExternal(href);
    }
  }, true);
})();
"#;

/// The running `mira serve`, killed when the app exits.
struct Server(Mutex<Option<Child>>);

fn main() {
    tauri::Builder::default()
        // First: a second launch (or, on Windows/Linux, a `mira://` link)
        // focuses the running app instead of starting another server.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            focus_main(app);
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_opener::init())
        .manage(Server(Mutex::new(None)))
        .invoke_handler(tauri::generate_handler![open_external])
        .setup(|app| {
            #[cfg(any(windows, target_os = "linux"))]
            {
                // Installed bundles register `mira://` at install time; this
                // covers dev builds and AppImages.
                let _ = app.deep_link().register_all();
            }
            let handle = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                for url in event.urls() {
                    if url.scheme() == "mira" {
                        focus_main(&handle);
                        let _ = handle.emit_to("main", AUTH_EVENT, url.to_string());
                    }
                }
            });

            let window = build_window(app.handle())?;
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                match start_server(&handle) {
                    Ok(url) => {
                        if let Err(e) = window.navigate(url) {
                            show_error(&window, &format!("could not open the UI: {e}"));
                        }
                    }
                    Err(e) => show_error(&window, &e),
                }
                // On Linux the server's parent-death signal follows the
                // thread that spawned it, so this thread lives as long as
                // the app.
                loop {
                    std::thread::park();
                }
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build the Mira app")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                stop_server(app);
            }
        });
}

fn build_window(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    let handle = app.clone();
    WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("Mira")
        .inner_size(1280.0, 840.0)
        .min_inner_size(760.0, 520.0)
        .initialization_script(INIT_SCRIPT)
        .on_navigation(move |url| {
            // The window only ever shows the bundled loading page and the
            // local server; anything else goes to the system browser.
            let local = matches!(
                url.host_str(),
                Some("127.0.0.1" | "localhost" | "tauri.localhost")
            );
            if local || !matches!(url.scheme(), "http" | "https") {
                return true;
            }
            let _ = handle.opener().open_url(url.as_str(), None::<&str>);
            false
        })
        .build()
}

#[tauri::command]
fn open_external(app: AppHandle, url: String) -> Result<(), String> {
    let ok = ["http://", "https://", "mailto:"]
        .iter()
        .any(|p| url.starts_with(p));
    if !ok {
        return Err(format!("refusing to open `{url}`"));
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| e.to_string())
}

fn focus_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn show_error(window: &WebviewWindow, message: &str) {
    let js = format!(
        "window.miraStartupFailed && window.miraStartupFailed({})",
        serde_json::Value::String(message.to_owned())
    );
    let _ = window.eval(&js);
}

fn stop_server(app: &AppHandle) {
    let state = app.state::<Server>();
    let child = state.0.lock().ok().and_then(|mut c| c.take());
    if let Some(mut child) = child {
        let _ = child.kill();
        let _ = child.wait();
    }
    let _ = std::fs::remove_file(pid_path());
}

fn pid_path() -> PathBuf {
    home_dir().join(".mira").join("desktop-server.pid")
}

/// Stop a server left behind by an app that didn't exit cleanly (crash,
/// force quit), so it doesn't hold the preferred port. Only kills the
/// recorded process if it still looks like the server this app starts.
#[cfg(unix)]
fn stop_stale_server() {
    let path = pid_path();
    let Ok(pid) = std::fs::read_to_string(&path) else {
        return;
    };
    let _ = std::fs::remove_file(&path);
    let pid = pid.trim();
    if pid.is_empty() || !pid.bytes().all(|b| b.is_ascii_digit()) {
        return;
    }
    let args = Command::new("ps")
        .args(["-p", pid, "-o", "args="])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    if !args.contains("mira") || !args.contains("serve --host 127.0.0.1 --port") {
        return;
    }
    let _ = Command::new("kill").arg(pid).status();
    // Give it a moment to release the port.
    for _ in 0..20 {
        let alive = Command::new("kill")
            .args(["-0", pid])
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !alive {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(not(unix))]
fn stop_stale_server() {}

/// Start `mira serve` and wait until it accepts connections. Returns the
/// URL to load.
fn start_server(app: &AppHandle) -> Result<tauri::Url, String> {
    stop_stale_server();
    let port = pick_port().ok_or("no free local port for the Mira server")?;
    let bin = mira_binary();
    let log_path = log_path();
    if let Some(dir) = log_path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| format!("{}: {e}", log_path.display()))?;
    let log_start = log.metadata().map(|m| m.len()).unwrap_or(0);
    let log_err = log.try_clone().map_err(|e| e.to_string())?;

    let mut cmd = Command::new(&bin);
    cmd.args(["serve", "--host", "127.0.0.1", "--port", &port.to_string()])
        .current_dir(home_dir())
        .envs(login_shell_env())
        .stdin(Stdio::null())
        .stdout(log)
        .stderr(log_err);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(target_os = "linux")]
    {
        // Stop the server if the app dies without cleaning up.
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }
    }
    let child = cmd
        .spawn()
        .map_err(|e| format!("could not start `{}`: {e}", bin.display()))?;
    let _ = std::fs::write(pid_path(), child.id().to_string());
    app.state::<Server>()
        .0
        .lock()
        .map_err(|e| e.to_string())?
        .replace(child);

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    loop {
        if TcpStream::connect_timeout(&addr, Duration::from_millis(250)).is_ok() {
            break;
        }
        let exited = app
            .state::<Server>()
            .0
            .lock()
            .ok()
            .and_then(|mut c| c.as_mut().and_then(|c| c.try_wait().ok().flatten()));
        if let Some(status) = exited {
            return Err(format!(
                "the Mira server stopped ({status}).\n\n{}\n\nLog: {}",
                log_tail(&log_path, log_start),
                log_path.display()
            ));
        }
        if Instant::now() > deadline {
            return Err(format!(
                "the Mira server didn't start within {}s.\n\n{}\n\nLog: {}",
                STARTUP_TIMEOUT.as_secs(),
                log_tail(&log_path, log_start),
                log_path.display()
            ));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    format!("http://127.0.0.1:{port}/")
        .parse()
        .map_err(|e| format!("{e}"))
}

fn pick_port() -> Option<u16> {
    (PREFERRED_PORT..PREFERRED_PORT + PORT_ATTEMPTS)
        .find(|p| TcpListener::bind(("127.0.0.1", *p)).is_ok())
}

/// `MIRA_BIN` if set, else the sidecar next to the app's executable, else
/// `mira` on PATH.
fn mira_binary() -> PathBuf {
    if let Some(p) = std::env::var_os("MIRA_BIN") {
        return PathBuf::from(p);
    }
    let name = if cfg!(windows) { "mira.exe" } else { "mira" };
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.join(name)))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from(name))
}

fn home_dir() -> PathBuf {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn log_path() -> PathBuf {
    home_dir()
        .join(".mira")
        .join("logs")
        .join("desktop-server.log")
}

fn log_tail(path: &std::path::Path, from: u64) -> String {
    let mut text = String::new();
    if let Ok(mut f) = std::fs::File::open(path) {
        let _ = f.seek(SeekFrom::Start(from));
        let _ = f.read_to_string(&mut text);
    }
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(20)..].join("\n")
}

/// The environment of a login shell. Apps started from the Dock or a
/// launcher get a minimal PATH and none of the keys exported in the
/// user's shell profile; the agent needs both.
#[cfg(unix)]
fn login_shell_env() -> Vec<(String, String)> {
    use std::sync::mpsc;

    let shell = std::env::var("SHELL").unwrap_or_else(|_| {
        if cfg!(target_os = "macos") {
            "/bin/zsh".into()
        } else {
            "/bin/sh".into()
        }
    });
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let out = Command::new(shell)
            .args(["-l", "-c", "env -0"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output();
        let _ = tx.send(out);
    });
    // A slow or interactive profile shouldn't block startup for long.
    let Ok(Ok(out)) = rx.recv_timeout(Duration::from_secs(5)) else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&out.stdout)
        .split('\0')
        .filter_map(|kv| kv.split_once('='))
        .filter(|(k, _)| !matches!(*k, "PWD" | "OLDPWD" | "SHLVL" | "_" | "TERM"))
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
}

#[cfg(not(unix))]
fn login_shell_env() -> Vec<(String, String)> {
    Vec::new()
}
