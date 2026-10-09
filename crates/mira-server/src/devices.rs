//! Device tools for agents: iOS Simulators and Android Emulators on this
//! machine. List them, boot one (it opens where you can watch it), take
//! screenshots, open URLs, install and launch apps, and drive input.
//!
//! Everything runs the platform's own tools with argument vectors, never a
//! shell: `xcrun simctl` for iOS, `adb` / `emulator` for Android. Device ids
//! are checked against what those tools list before use.
//!
//! Input differs by platform. Android takes taps, swipes, text and keys
//! through `adb shell input`. `simctl` has no input commands, so iOS taps and
//! typing go through Facebook's `idb` when it's installed; without it those
//! calls say how to install it rather than failing silently.

use std::path::PathBuf;
use std::time::Duration;

use base64::Engine as _;
use serde_json::{json, Value};

const CMD_TIMEOUT: Duration = Duration::from_secs(60);
const BOOT_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Platform {
    Ios,
    Android,
}

#[derive(Clone, Debug)]
struct Device {
    platform: Platform,
    /// iOS: UDID. Android: the AVD name until it runs, then its serial.
    id: String,
    name: String,
    /// iOS runtime ("iOS 26.2") or "Android".
    os: String,
    booted: bool,
}

/// Run a tool and return stdout, or its stderr as the error.
async fn run(program: &PathBuf, args: &[&str], timeout: Duration) -> Result<Vec<u8>, String> {
    let fut = tokio::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    let out = tokio::time::timeout(timeout, fut)
        .await
        .map_err(|_| format!("{} timed out", program.display()))?
        .map_err(|e| format!("{}: {e}", program.display()))?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if err.is_empty() {
            format!("{} failed", program.display())
        } else {
            err
        })
    }
}

/// Whether this machine has any device to drive: iOS Simulators (Xcode's
/// simulator runtime set up) or the Android SDK's tools. Checked once.
pub fn available() -> bool {
    static AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        let simulators = xcrun().is_some()
            && std::env::var_os("HOME")
                .map(PathBuf::from)
                .is_some_and(|h| h.join("Library/Developer/CoreSimulator/Devices").is_dir());
        simulators || adb().is_some() || emulator().is_some()
    })
}

fn xcrun() -> Option<PathBuf> {
    cfg!(target_os = "macos")
        .then(|| PathBuf::from("/usr/bin/xcrun"))
        .filter(|p| p.exists())
}

/// The Android SDK: `ANDROID_HOME`, `ANDROID_SDK_ROOT`, or the default
/// Android Studio location.
fn android_sdk() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    [
        std::env::var_os("ANDROID_HOME").map(PathBuf::from),
        std::env::var_os("ANDROID_SDK_ROOT").map(PathBuf::from),
        home.as_ref().map(|h| h.join("Library/Android/sdk")),
        home.as_ref().map(|h| h.join("Android/Sdk")),
    ]
    .into_iter()
    .flatten()
    .find(|p| p.is_dir())
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|d| d.join(name))
            .find(|p| p.is_file())
    })
}

fn adb() -> Option<PathBuf> {
    android_sdk()
        .map(|s| s.join("platform-tools/adb"))
        .filter(|p| p.is_file())
        .or_else(|| find_on_path("adb"))
}

fn emulator() -> Option<PathBuf> {
    android_sdk()
        .map(|s| s.join("emulator/emulator"))
        .filter(|p| p.is_file())
        .or_else(|| find_on_path("emulator"))
}

fn idb() -> Option<PathBuf> {
    find_on_path("idb")
}

async fn ios_devices() -> Result<Vec<Device>, String> {
    let Some(x) = xcrun() else {
        return Err("Xcode isn't installed".into());
    };
    let raw = run(
        &x,
        &["simctl", "list", "devices", "available", "-j"],
        CMD_TIMEOUT,
    )
    .await?;
    let v: Value = serde_json::from_slice(&raw).map_err(|e| format!("simctl list: {e}"))?;
    let mut out = Vec::new();
    for (runtime, list) in v
        .get("devices")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        // com.apple.CoreSimulator.SimRuntime.iOS-26-2 → iOS 26.2
        let os = runtime
            .rsplit('.')
            .next()
            .unwrap_or(runtime)
            .replacen('-', " ", 1)
            .replace('-', ".");
        if !os.starts_with("iOS") && !os.starts_with("iPadOS") {
            continue;
        }
        for d in list.as_array().into_iter().flatten() {
            let (Some(id), Some(name)) = (
                d.get("udid").and_then(Value::as_str),
                d.get("name").and_then(Value::as_str),
            ) else {
                continue;
            };
            out.push(Device {
                platform: Platform::Ios,
                id: id.to_string(),
                name: name.to_string(),
                os: os.clone(),
                booted: d.get("state").and_then(Value::as_str) == Some("Booted"),
            });
        }
    }
    Ok(out)
}

async fn android_devices() -> Result<Vec<Device>, String> {
    let (Some(adb), emu) = (adb(), emulator()) else {
        return Err("the Android SDK isn't installed (no adb)".into());
    };
    let mut out = Vec::new();
    let mut running_avds = Vec::new();
    let listed = String::from_utf8_lossy(&run(&adb, &["devices"], CMD_TIMEOUT).await?).to_string();
    for line in listed.lines().skip(1) {
        let mut parts = line.split_whitespace();
        let (Some(serial), Some("device")) = (parts.next(), parts.next()) else {
            continue;
        };
        let avd = run(
            &adb,
            &["-s", serial, "emu", "avd", "name"],
            Duration::from_secs(5),
        )
        .await
        .map(|o| {
            String::from_utf8_lossy(&o)
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_string()
        })
        .unwrap_or_default();
        if !avd.is_empty() {
            running_avds.push(avd.clone());
        }
        out.push(Device {
            platform: Platform::Android,
            id: serial.to_string(),
            name: if avd.is_empty() {
                serial.to_string()
            } else {
                avd
            },
            os: "Android".into(),
            booted: true,
        });
    }
    if let Some(emu) = emu {
        if let Ok(avds) = run(&emu, &["-list-avds"], CMD_TIMEOUT).await {
            for name in String::from_utf8_lossy(&avds)
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with("INFO"))
            {
                if !running_avds.iter().any(|r| r == name) {
                    out.push(Device {
                        platform: Platform::Android,
                        id: name.to_string(),
                        name: name.to_string(),
                        os: "Android".into(),
                        booted: false,
                    });
                }
            }
        }
    }
    Ok(out)
}

async fn all_devices() -> (Vec<Device>, Vec<String>) {
    let mut devices = Vec::new();
    let mut notes = Vec::new();
    match ios_devices().await {
        Ok(d) => devices.extend(d),
        Err(e) => notes.push(format!("iOS: {e}")),
    }
    match android_devices().await {
        Ok(d) => devices.extend(d),
        Err(e) => notes.push(format!("Android: {e}")),
    }
    (devices, notes)
}

/// The device `id` names, by UDID, serial, AVD name or exact device name.
async fn find(id: &str) -> Result<Device, String> {
    let (devices, _) = all_devices().await;
    devices
        .into_iter()
        .find(|d| d.id == id || d.name == id)
        .ok_or_else(|| format!("no device `{id}`; call device_list"))
}

fn arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| format!("`{key}` is required"))
}

fn num(args: &Value, key: &str) -> Result<f64, String> {
    args.get(key)
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("`{key}` is required"))
}

/// Boot (if needed) and show the device. Returns the id to use from now on
/// (an Android AVD gets a serial once it runs).
async fn open(d: &Device) -> Result<String, String> {
    match d.platform {
        Platform::Ios => {
            let x = xcrun().ok_or("Xcode isn't installed")?;
            if !d.booted {
                run(&x, &["simctl", "boot", &d.id], BOOT_TIMEOUT).await?;
            }
            // Bring up Simulator.app on this device so the user can watch.
            let _ = run(
                &PathBuf::from("/usr/bin/open"),
                &["-a", "Simulator", "--args", "-CurrentDeviceUDID", &d.id],
                CMD_TIMEOUT,
            )
            .await;
            run(&x, &["simctl", "bootstatus", &d.id, "-b"], BOOT_TIMEOUT).await?;
            Ok(d.id.clone())
        }
        Platform::Android => {
            if d.booted {
                return Ok(d.id.clone());
            }
            let emu = emulator().ok_or("the Android emulator isn't installed")?;
            let adb = adb().ok_or("adb isn't installed")?;
            // Detached: the emulator outlives this call (and its window is
            // where the user watches).
            std::process::Command::new(&emu)
                .args(["-avd", &d.id])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .map_err(|e| format!("emulator: {e}"))?;
            let deadline = tokio::time::Instant::now() + BOOT_TIMEOUT;
            loop {
                if let Ok(Device { id, .. }) = find(&d.name).await.filter_booted() {
                    let booted = run(
                        &adb,
                        &["-s", &id, "shell", "getprop", "sys.boot_completed"],
                        Duration::from_secs(5),
                    )
                    .await
                    .map(|o| String::from_utf8_lossy(&o).trim() == "1")
                    .unwrap_or(false);
                    if booted {
                        return Ok(id);
                    }
                }
                if tokio::time::Instant::now() >= deadline {
                    return Err(format!(
                        "{} didn't finish booting in {}s",
                        d.name,
                        BOOT_TIMEOUT.as_secs()
                    ));
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}

trait FilterBooted {
    fn filter_booted(self) -> Result<Device, String>;
}
impl FilterBooted for Result<Device, String> {
    fn filter_booted(self) -> Result<Device, String> {
        self.and_then(|d| {
            if d.booted {
                Ok(d)
            } else {
                Err("not booted".into())
            }
        })
    }
}

async fn screenshot(d: &Device) -> Result<Vec<u8>, String> {
    if !d.booted {
        return Err(format!("{} isn't running; call device_open first", d.name));
    }
    let png = match d.platform {
        Platform::Ios => {
            let x = xcrun().ok_or("Xcode isn't installed")?;
            let dir = ScratchDir::new()?;
            let file = dir.0.join("shot.png");
            let file_s = file.to_string_lossy().to_string();
            run(
                &x,
                &["simctl", "io", &d.id, "screenshot", "--type=png", &file_s],
                CMD_TIMEOUT,
            )
            .await?;
            downscale(&file).await;
            std::fs::read(&file).map_err(|e| e.to_string())?
        }
        Platform::Android => {
            let adb = adb().ok_or("adb isn't installed")?;
            let raw = run(
                &adb,
                &["-s", &d.id, "exec-out", "screencap", "-p"],
                CMD_TIMEOUT,
            )
            .await?;
            let dir = ScratchDir::new()?;
            let file = dir.0.join("shot.png");
            std::fs::write(&file, &raw).map_err(|e| e.to_string())?;
            downscale(&file).await;
            std::fs::read(&file).map_err(|e| e.to_string())?
        }
    };
    Ok(png)
}

/// A private temp folder, removed when dropped.
struct ScratchDir(PathBuf);
impl ScratchDir {
    fn new() -> Result<Self, String> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let dir = std::env::temp_dir().join(format!("mira-device-{}-{nanos}", std::process::id()));
        std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
        Ok(Self(dir))
    }
}
impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Phone screenshots are ~3x: shrink the long edge so one doesn't eat the
/// model's context. Best effort (macOS `sips`); the original is kept if it
/// isn't there.
async fn downscale(file: &std::path::Path) {
    let sips = PathBuf::from("/usr/bin/sips");
    if sips.exists() {
        let f = file.to_string_lossy().to_string();
        let _ = run(&sips, &["-Z", "1400", &f], CMD_TIMEOUT).await;
    }
}

async fn input(d: &Device, action: &str, args: &Value) -> Result<String, String> {
    if !d.booted {
        return Err(format!("{} isn't running; call device_open first", d.name));
    }
    match d.platform {
        Platform::Android => {
            let adb = adb().ok_or("adb isn't installed")?;
            let s = |v: f64| format!("{}", v.round() as i64);
            let cmd: Vec<String> = match action {
                "tap" => vec!["tap".into(), s(num(args, "x")?), s(num(args, "y")?)],
                "swipe" => vec![
                    "swipe".into(),
                    s(num(args, "x")?),
                    s(num(args, "y")?),
                    s(num(args, "to_x")?),
                    s(num(args, "to_y")?),
                    s(args
                        .get("duration_ms")
                        .and_then(Value::as_f64)
                        .unwrap_or(300.0)),
                ],
                // `input text` takes %s for spaces and no shell metacharacters.
                "type" => vec!["text".into(), arg(args, "text")?.replace(' ', "%s")],
                "key" => vec!["keyevent".into(), android_key(arg(args, "key")?)],
                _ => return Err(format!("unknown input `{action}`")),
            };
            let mut argv = vec!["-s", d.id.as_str(), "shell", "input"];
            argv.extend(cmd.iter().map(String::as_str));
            run(&adb, &argv, CMD_TIMEOUT).await?;
            Ok(format!("{action} sent to {}", d.name))
        }
        Platform::Ios => {
            let Some(idb) = idb() else {
                return Err("iOS Simulator input needs Facebook's idb: `brew install facebook/fb/idb-companion` \
                            and `pip3 install fb-idb`. Until then: open URLs, launch apps and take screenshots."
                    .into());
            };
            let s = |v: f64| format!("{}", v.round() as i64);
            let cmd: Vec<String> = match action {
                "tap" => vec![
                    "ui".into(),
                    "tap".into(),
                    s(num(args, "x")?),
                    s(num(args, "y")?),
                ],
                "swipe" => vec![
                    "ui".into(),
                    "swipe".into(),
                    s(num(args, "x")?),
                    s(num(args, "y")?),
                    s(num(args, "to_x")?),
                    s(num(args, "to_y")?),
                ],
                "type" => vec!["ui".into(), "text".into(), arg(args, "text")?.to_string()],
                "key" => vec!["ui".into(), "key".into(), arg(args, "key")?.to_string()],
                _ => return Err(format!("unknown input `{action}`")),
            };
            let mut argv: Vec<&str> = cmd.iter().map(String::as_str).collect();
            argv.extend(["--udid", d.id.as_str()]);
            run(&idb, &argv, CMD_TIMEOUT).await?;
            Ok(format!("{action} sent to {}", d.name))
        }
    }
}

fn android_key(key: &str) -> String {
    match key.to_ascii_lowercase().as_str() {
        "home" => "KEYCODE_HOME".into(),
        "back" => "KEYCODE_BACK".into(),
        "enter" | "return" => "KEYCODE_ENTER".into(),
        "delete" | "backspace" => "KEYCODE_DEL".into(),
        "tab" => "KEYCODE_TAB".into(),
        "menu" => "KEYCODE_MENU".into(),
        other if other.starts_with("keycode_") => other.to_ascii_uppercase(),
        other => other.to_string(),
    }
}

/// Run one `device_*` tool. `Ok` content is MCP tool-result content.
pub async fn call(name: &str, args: &Value) -> Result<Vec<Value>, String> {
    let text = |t: String| vec![json!({ "type": "text", "text": t })];
    match name {
        "device_list" => {
            let (devices, notes) = all_devices().await;
            let rows: Vec<Value> = devices
                .iter()
                .map(|d| json!({ "id": d.id, "name": d.name, "os": d.os, "running": d.booted }))
                .collect();
            let mut out = serde_json::to_string_pretty(&rows).unwrap_or_default();
            if rows.is_empty() {
                out = "No simulators or emulators found.".into();
            }
            if !notes.is_empty() {
                out.push_str(&format!("\n\nUnavailable: {}", notes.join("; ")));
            }
            Ok(text(out))
        }
        "device_open" => {
            let d = find(arg(args, "id")?).await?;
            let id = open(&d).await?;
            Ok(text(format!(
                "{} is running (id {id}) and open on screen where the user can watch. \
                 Use device_screenshot to see it.",
                d.name
            )))
        }
        "device_screenshot" => {
            let d = find(arg(args, "id")?).await?;
            let png = screenshot(&d).await?;
            Ok(vec![
                json!({ "type": "text", "text": format!("{} screenshot", d.name) }),
                json!({ "type": "image", "data": base64::engine::general_purpose::STANDARD.encode(png), "mimeType": "image/png" }),
            ])
        }
        "device_open_url" => {
            let d = find(arg(args, "id")?).await?;
            let url = arg(args, "url")?;
            match d.platform {
                Platform::Ios => {
                    run(
                        &xcrun().ok_or("Xcode isn't installed")?,
                        &["simctl", "openurl", &d.id, url],
                        CMD_TIMEOUT,
                    )
                    .await?;
                }
                Platform::Android => {
                    let adb = adb().ok_or("adb isn't installed")?;
                    run(
                        &adb,
                        &[
                            "-s",
                            &d.id,
                            "shell",
                            "am",
                            "start",
                            "-a",
                            "android.intent.action.VIEW",
                            "-d",
                            url,
                        ],
                        CMD_TIMEOUT,
                    )
                    .await?;
                }
            }
            Ok(text(format!("Opened {url} on {}.", d.name)))
        }
        "device_install" => {
            let d = find(arg(args, "id")?).await?;
            let path = arg(args, "path")?;
            if !std::path::Path::new(path).exists() {
                return Err(format!("{path} doesn't exist"));
            }
            match d.platform {
                Platform::Ios => {
                    run(
                        &xcrun().ok_or("Xcode isn't installed")?,
                        &["simctl", "install", &d.id, path],
                        BOOT_TIMEOUT,
                    )
                    .await?;
                }
                Platform::Android => {
                    run(
                        &adb().ok_or("adb isn't installed")?,
                        &["-s", &d.id, "install", "-r", path],
                        BOOT_TIMEOUT,
                    )
                    .await?;
                }
            }
            Ok(text(format!("Installed {path} on {}.", d.name)))
        }
        "device_launch" => {
            let d = find(arg(args, "id")?).await?;
            let app = arg(args, "app")?;
            match d.platform {
                Platform::Ios => {
                    run(
                        &xcrun().ok_or("Xcode isn't installed")?,
                        &["simctl", "launch", &d.id, app],
                        CMD_TIMEOUT,
                    )
                    .await?;
                }
                Platform::Android => {
                    let adb = adb().ok_or("adb isn't installed")?;
                    run(
                        &adb,
                        &[
                            "-s",
                            &d.id,
                            "shell",
                            "monkey",
                            "-p",
                            app,
                            "-c",
                            "android.intent.category.LAUNCHER",
                            "1",
                        ],
                        CMD_TIMEOUT,
                    )
                    .await?;
                }
            }
            Ok(text(format!("Launched {app} on {}.", d.name)))
        }
        "device_tap" | "device_swipe" | "device_type" | "device_key" => {
            let d = find(arg(args, "id")?).await?;
            Ok(text(
                input(&d, name.trim_start_matches("device_"), args).await?,
            ))
        }
        _ => Err(format!("unknown device tool `{name}`")),
    }
}

/// The device tools' MCP definitions.
pub fn tool_specs() -> Vec<Value> {
    let id = json!({ "type": "string", "description": "From device_list: an iOS UDID, an Android serial or AVD name, or the device's name" });
    let tool = |name: &str, description: &str, props: Value, required: &[&str]| {
        json!({
            "name": name,
            "description": description,
            "inputSchema": { "type": "object", "properties": props, "required": required, "additionalProperties": false }
        })
    };
    vec![
        tool("device_list", "List the iOS Simulators and Android Emulators on this machine, and which are running.", json!({}), &[]),
        tool(
            "device_open",
            "Boot a simulator or emulator (if it isn't running) and show it on screen so the user can watch.",
            json!({ "id": id }),
            &["id"],
        ),
        tool("device_screenshot", "A screenshot of a running device, to check your app's UI.", json!({ "id": id }), &["id"]),
        tool(
            "device_open_url",
            "Open a URL (a web page or a deep link into your app) on a running device.",
            json!({ "id": id, "url": { "type": "string" } }),
            &["id", "url"],
        ),
        tool(
            "device_install",
            "Install an app build on a running device: an iOS Simulator .app, or an Android .apk.",
            json!({ "id": id, "path": { "type": "string", "description": "Absolute path to the .app or .apk" } }),
            &["id", "path"],
        ),
        tool(
            "device_launch",
            "Launch an installed app: its iOS bundle id or Android package name.",
            json!({ "id": id, "app": { "type": "string" } }),
            &["id", "app"],
        ),
        tool(
            "device_tap",
            "Tap at a point, in the device's screen points (Android: pixels). iOS needs idb installed.",
            json!({ "id": id, "x": { "type": "number" }, "y": { "type": "number" } }),
            &["id", "x", "y"],
        ),
        tool(
            "device_swipe",
            "Swipe from one point to another. iOS needs idb installed.",
            json!({ "id": id, "x": { "type": "number" }, "y": { "type": "number" }, "to_x": { "type": "number" }, "to_y": { "type": "number" }, "duration_ms": { "type": "number" } }),
            &["id", "x", "y", "to_x", "to_y"],
        ),
        tool(
            "device_type",
            "Type text into the focused field. iOS needs idb installed.",
            json!({ "id": id, "text": { "type": "string" } }),
            &["id", "text"],
        ),
        tool(
            "device_key",
            "Press a key: home, back (Android), enter, delete, tab. iOS needs idb installed.",
            json!({ "id": id, "key": { "type": "string" } }),
            &["id", "key"],
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_has_a_schema_and_a_route() {
        for spec in tool_specs() {
            let name = spec["name"].as_str().unwrap();
            assert!(name.starts_with("device_"), "{name}");
            assert!(spec["inputSchema"]["properties"].is_object(), "{name}");
        }
        assert_eq!(android_key("Back"), "KEYCODE_BACK");
        assert_eq!(android_key("keycode_volume_up"), "KEYCODE_VOLUME_UP");
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn lists_ios_simulators_when_xcode_is_installed() {
        if xcrun().is_none() {
            return;
        }
        let out = call("device_list", &json!({})).await.unwrap();
        let text = out[0]["text"].as_str().unwrap();
        // Either a JSON list of devices, or a clear "none found".
        assert!(
            text.contains("\"id\"") || text.contains("No simulators"),
            "{text}"
        );
    }
}
