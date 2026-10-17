//! Device tools for agents: list, open, screenshot and close the simulators
//! and emulators the device hub sees. Opening a device shows it in the
//! user's Device pane.
//!
//! Driving a device (tapping, typing, reading the screen) isn't a tool
//! here: agents use the `agent-device` command line, which Mira puts on
//! their PATH while agent access is on. `device_open` says how, for the
//! pinned version, so nothing about it rides along in threads that never
//! open a device.

use std::path::PathBuf;
use std::time::Duration;

use base64::Engine as _;
use serde_json::{json, Value};

use super::host::service;
use super::toolchain::{self, AGENT_DEVICE};

const CMD_TIMEOUT: Duration = Duration::from_secs(60);

/// Run a tool and return stdout, or its stderr as the error.
pub(super) async fn run(
    program: &PathBuf,
    args: &[&str],
    timeout: Duration,
) -> Result<Vec<u8>, String> {
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

pub(super) fn xcrun() -> Option<PathBuf> {
    cfg!(target_os = "macos")
        .then(|| PathBuf::from("/usr/bin/xcrun"))
        .filter(|p| p.exists())
}

/// The Android SDK's adb: `ANDROID_HOME`, `ANDROID_SDK_ROOT`, the default
/// Android Studio location, or PATH.
pub(super) fn adb() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    [
        std::env::var_os("ANDROID_HOME").map(PathBuf::from),
        std::env::var_os("ANDROID_SDK_ROOT").map(PathBuf::from),
        home.as_ref().map(|h| h.join("Library/Android/sdk")),
        home.as_ref().map(|h| h.join("Android/Sdk")),
    ]
    .into_iter()
    .flatten()
    .map(|sdk| sdk.join("platform-tools/adb"))
    .find(|p| p.is_file())
    .or_else(|| {
        std::env::var_os("PATH").and_then(|paths| {
            std::env::split_paths(&paths)
                .map(|d| d.join("adb"))
                .find(|p| p.is_file())
        })
    })
}

/// Whether agents get device tools: the hub is on, the user allowed agent
/// access, and this machine can run at least one platform. Read on every
/// tool listing, so turning access off hides the tools from chats started
/// afterwards.
pub fn available() -> bool {
    let s = service().snapshot();
    s.hub_enabled && s.agent_access && (s.platforms.ios.available || s.platforms.android.available)
}

fn text(t: impl Into<String>) -> Value {
    json!({ "type": "text", "text": t.into() })
}

fn arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("`{key}` is required"))
}

/// A device from the hub's list, by id or (case-insensitively) by name.
async fn find(target: &str) -> Result<Value, String> {
    let list = service().list().await?;
    let devices = list["devices"].as_array().cloned().unwrap_or_default();
    let by_id = devices.iter().find(|d| d["id"] == target);
    let by_name = || {
        let mut named = devices.iter().filter(|d| {
            d["name"]
                .as_str()
                .is_some_and(|n| n.eq_ignore_ascii_case(target))
        });
        // Prefer one that's already running when names repeat.
        let all: Vec<&Value> = named.by_ref().collect();
        all.iter()
            .find(|d| d["booted"] == true)
            .or(all.first())
            .copied()
    };
    by_id
        .or_else(by_name)
        .cloned()
        .ok_or_else(|| format!("No device `{target}`. Call device_list for ids."))
}

fn platform_of(d: &Value) -> &str {
    d["platform"].as_str().unwrap_or("ios")
}

/// How to drive an open device with the pinned agent-device.
fn driving_notes(d: &Value) -> String {
    let id = d["id"].as_str().unwrap_or_default();
    let ios = platform_of(d) == "ios";
    let (platform, pin) = if ios {
        ("ios", format!("--udid {id}"))
    } else {
        ("android", format!("--serial {id}"))
    };
    format!(
        "Drive it from your shell with `agent-device` (v{version}), already on your PATH. Target this device with \
         `--platform {platform} {pin}` on the first command; later commands reuse the session.\n\
         - Start: `agent-device open <app id or name> --platform {platform} {pin} --foreground` returns a snapshot with @refs.\n\
         - Act: `agent-device press @e12 --settle`, `fill @e3 \"text\" --settle`, `scroll down --settle`, `back --settle`; each prints what changed.\n\
         - Check: `wait text \"…\"`, `is`, `get`, `find`. `agent-device help workflow` has the full loop.\n\
         - Finish with `agent-device close`.\n\
         Prefer agent-device over raw simctl/adb for input. The user is watching this device in Mira's Device pane.",
        version = toolchain::pinned(AGENT_DEVICE),
    )
}

pub async fn call(name: &str, args: &Value) -> Result<Vec<Value>, String> {
    let svc = service();
    match name {
        "device_list" => {
            let list = svc.list().await?;
            let mut lines: Vec<String> = list["devices"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|d| {
                    format!(
                        "- {} ({}, {}) id={} {}",
                        d["name"].as_str().unwrap_or("?"),
                        platform_of(d),
                        d["os"].as_str().unwrap_or("?"),
                        d["id"].as_str().unwrap_or("?"),
                        if d["booted"] == true {
                            "running"
                        } else {
                            "off"
                        }
                    )
                })
                .collect();
            if lines.is_empty() {
                lines.push("No devices.".into());
            }
            for n in list["notes"].as_array().into_iter().flatten() {
                if let Some(n) = n.as_str() {
                    lines.push(format!("Note: {n}"));
                }
            }
            Ok(vec![text(lines.join("\n"))])
        }
        "device_open" => {
            let d = find(arg(args, "device")?).await?;
            let platform = platform_of(&d).to_string();
            let opened = svc
                .boot(&platform, d["id"].as_str().unwrap_or_default())
                .await?;
            let mut d = d;
            // Android AVDs change id when they boot.
            d["id"] = opened["id"].clone();
            Ok(vec![text(format!(
                "Opened {} ({}, {}), id {}.\n\n{}",
                d["name"].as_str().unwrap_or("device"),
                platform,
                d["os"].as_str().unwrap_or(""),
                d["id"].as_str().unwrap_or(""),
                driving_notes(&d)
            ))])
        }
        "device_screenshot" => {
            let d = find(arg(args, "device")?).await?;
            if d["booted"] != true {
                return Err(format!(
                    "{} is off; call device_open first.",
                    d["name"].as_str().unwrap_or("The device")
                ));
            }
            let png = svc
                .screenshot(platform_of(&d), d["id"].as_str().unwrap_or_default())
                .await?;
            let png = downscale(png).await;
            Ok(vec![json!({
                "type": "image",
                "mimeType": "image/png",
                "data": base64::engine::general_purpose::STANDARD.encode(png),
            })])
        }
        "device_close" => {
            let d = find(arg(args, "device")?).await?;
            let name = d["name"].as_str().unwrap_or("The device").to_string();
            if args.get("power_off").and_then(Value::as_bool) == Some(true) {
                svc.shutdown(platform_of(&d), d["id"].as_str().unwrap_or_default())
                    .await?;
                Ok(vec![text(format!("{name} is powered off."))])
            } else {
                Ok(vec![text(format!(
                    "Done with {name}; it keeps running for the user. Run `agent-device close` if a session is open."
                ))])
            }
        }
        _ => Err(format!("unknown device tool `{name}`")),
    }
}

/// Screenshots come at native resolution (an iPhone is 1206×2622); a
/// smaller copy reads the same to a model for far fewer tokens.
async fn downscale(png: Vec<u8>) -> Vec<u8> {
    let sips = PathBuf::from("/usr/bin/sips");
    if !sips.exists() {
        return png;
    }
    let Ok(file) = tempfile::Builder::new().suffix(".png").tempfile() else {
        return png;
    };
    if std::fs::write(file.path(), &png).is_err() {
        return png;
    }
    let f = file.path().to_string_lossy().to_string();
    if run(&sips, &["-Z", "1400", &f], CMD_TIMEOUT).await.is_err() {
        return png;
    }
    std::fs::read(file.path()).unwrap_or(png)
}

fn spec(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false,
        },
    })
}

pub fn tool_specs() -> Vec<Value> {
    let device = json!({ "type": "string", "description": "Device id from device_list, or its name (\"iPhone 17\")." });
    vec![
        spec(
            "device_list",
            "List the iOS Simulators and Android Emulators on this machine, with ids and whether each is running.",
            json!({}),
            &[],
        ),
        spec(
            "device_open",
            "Start a device (or attach to it if running) and show it in the user's Device pane. Returns how to drive it with the agent-device command line.",
            json!({ "device": device }),
            &["device"],
        ),
        spec(
            "device_screenshot",
            "Take a screenshot of a running device.",
            json!({ "device": device }),
            &["device"],
        ),
        spec(
            "device_close",
            "Finish with a device. It keeps running for the user unless power_off is true.",
            json!({ "device": device, "power_off": { "type": "boolean", "description": "Shut the device down." } }),
            &["device"],
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_tools_with_schemas() {
        let names: Vec<String> = tool_specs()
            .iter()
            .map(|s| {
                assert!(s["inputSchema"]["properties"].is_object());
                s["name"].as_str().unwrap().to_string()
            })
            .collect();
        assert_eq!(
            names,
            [
                "device_list",
                "device_open",
                "device_screenshot",
                "device_close"
            ]
        );
    }

    #[test]
    fn driving_notes_pin_the_device() {
        let ios = driving_notes(&json!({ "platform": "ios", "id": "ABC-1" }));
        assert!(ios.contains("--platform ios --udid ABC-1"));
        assert!(ios.contains(toolchain::pinned(AGENT_DEVICE)));
        let android = driving_notes(&json!({ "platform": "android", "id": "emulator-5554" }));
        assert!(android.contains("--serial emulator-5554"));
    }
}
