//! Device settings for the Device pane's tools drawer, one typed command
//! per control. These run `simctl` / `adb` here with argument vectors; they
//! never go through the hub, whose settings panel works by sending it shell
//! commands. Each action answers with the value read back from the device.

use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};

use super::tools::{adb, run, xcrun};

const T: Duration = Duration::from_secs(30);

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "camelCase")]
pub enum Action {
    /// `light` | `dark`.
    Appearance {
        value: String,
    },
    /// iOS content size category, e.g. `large`, `extra-large`,
    /// `accessibility-extra-large`. Android: a font scale like `1.3`.
    TextSize {
        value: String,
    },
    Location {
        latitude: f64,
        longitude: f64,
    },
    /// iOS: a `simctl privacy` service (`photos`, `camera`, `location`…).
    /// Android: a permission (`android.permission.CAMERA`).
    Permission {
        app: String,
        service: String,
        grant: bool,
    },
    /// iOS only: an APNs payload delivered to `app`.
    Push {
        app: String,
        payload: Value,
    },
    /// Android only: 0, 90, 180 or 270.
    Orientation {
        degrees: u16,
    },
    /// Android only.
    Network {
        wifi: Option<bool>,
        data: Option<bool>,
    },
}

/// No leading `-`, so a value can't be read as an option.
fn leads_alnum(s: &str) -> bool {
    s.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric())
}

fn reverse_dns(s: &str) -> bool {
    leads_alnum(s)
        && s.len() < 256
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

fn word(s: &str) -> bool {
    leads_alnum(s)
        && s.len() < 64
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

fn text(out: Vec<u8>) -> String {
    String::from_utf8_lossy(&out).trim().to_string()
}

pub async fn apply(platform: &str, id: &str, action: Action) -> Result<Value, String> {
    match platform {
        "ios" => ios(id, action).await,
        "android" => android(id, action).await,
        _ => Err(format!("unknown platform `{platform}`")),
    }
}

async fn ios(udid: &str, action: Action) -> Result<Value, String> {
    let x = xcrun().ok_or("Xcode isn't installed")?;
    let simctl = |args: Vec<String>| {
        let x = x.clone();
        async move {
            let argv: Vec<&str> = std::iter::once("simctl")
                .chain(args.iter().map(String::as_str))
                .collect();
            run(&x, &argv, T).await.map(text)
        }
    };
    let u = udid.to_string();
    match action {
        Action::Appearance { value } => {
            if !matches!(value.as_str(), "light" | "dark") {
                return Err("appearance is light or dark".into());
            }
            simctl(vec!["ui".into(), u.clone(), "appearance".into(), value]).await?;
            let now = simctl(vec!["ui".into(), u, "appearance".into()]).await?;
            Ok(json!({ "appearance": now }))
        }
        Action::TextSize { value } => {
            if !word(&value) {
                return Err("unknown text size".into());
            }
            simctl(vec!["ui".into(), u.clone(), "content_size".into(), value]).await?;
            let now = simctl(vec!["ui".into(), u, "content_size".into()]).await?;
            Ok(json!({ "textSize": now }))
        }
        Action::Location {
            latitude,
            longitude,
        } => {
            check_coords(latitude, longitude)?;
            simctl(vec![
                "location".into(),
                u,
                "set".into(),
                format!("{latitude},{longitude}"),
            ])
            .await?;
            Ok(json!({ "location": { "latitude": latitude, "longitude": longitude } }))
        }
        Action::Permission {
            app,
            service,
            grant,
        } => {
            if !reverse_dns(&app) || !word(&service) {
                return Err("app must be a bundle id and service a privacy service name".into());
            }
            let verb = if grant { "grant" } else { "revoke" };
            simctl(vec![
                "privacy".into(),
                u,
                verb.into(),
                service.clone(),
                app.clone(),
            ])
            .await?;
            Ok(json!({ "permission": { "app": app, "service": service, "granted": grant } }))
        }
        Action::Push { app, payload } => {
            if !reverse_dns(&app) {
                return Err("app must be a bundle id".into());
            }
            let file = tempfile::Builder::new()
                .suffix(".apns")
                .tempfile()
                .map_err(|e| e.to_string())?;
            std::fs::write(
                file.path(),
                serde_json::to_vec(&payload).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            let path = file.path().to_string_lossy().to_string();
            simctl(vec!["push".into(), u, app, path]).await?;
            Ok(json!({ "pushed": true }))
        }
        Action::Orientation { .. } | Action::Network { .. } => {
            Err("not available on iOS Simulators".into())
        }
    }
}

async fn android(serial: &str, action: Action) -> Result<Value, String> {
    let adb = adb().ok_or("adb isn't installed")?;
    let shell = |args: Vec<String>| {
        let adb = adb.clone();
        let serial = serial.to_string();
        async move {
            let argv: Vec<&str> = ["-s", serial.as_str(), "shell"]
                .into_iter()
                .chain(args.iter().map(String::as_str))
                .collect();
            run(&adb, &argv, T).await.map(text)
        }
    };
    let s = |v: &str| v.to_string();
    match action {
        Action::Appearance { value } => {
            let night = match value.as_str() {
                "dark" => "yes",
                "light" => "no",
                _ => return Err("appearance is light or dark".into()),
            };
            shell(vec![s("cmd"), s("uimode"), s("night"), s(night)]).await?;
            let now = shell(vec![s("cmd"), s("uimode"), s("night")]).await?;
            Ok(json!({ "appearance": if now.contains("yes") { "dark" } else { "light" } }))
        }
        Action::TextSize { value } => {
            let scale: f32 = value
                .parse()
                .map_err(|_| "Android text size is a scale like 1.3")?;
            if !(0.5..=2.0).contains(&scale) {
                return Err("font scale is between 0.5 and 2".into());
            }
            shell(vec![
                s("settings"),
                s("put"),
                s("system"),
                s("font_scale"),
                scale.to_string(),
            ])
            .await?;
            let now = shell(vec![s("settings"), s("get"), s("system"), s("font_scale")]).await?;
            Ok(json!({ "textSize": now }))
        }
        Action::Location {
            latitude,
            longitude,
        } => {
            check_coords(latitude, longitude)?;
            // `emu geo fix` takes longitude first.
            run(
                &adb,
                &[
                    "-s",
                    serial,
                    "emu",
                    "geo",
                    "fix",
                    &longitude.to_string(),
                    &latitude.to_string(),
                ],
                T,
            )
            .await?;
            Ok(json!({ "location": { "latitude": latitude, "longitude": longitude } }))
        }
        Action::Permission {
            app,
            service,
            grant,
        } => {
            if !reverse_dns(&app) || !reverse_dns(&service) {
                return Err("app must be a package name and service a permission name".into());
            }
            let verb = if grant { "grant" } else { "revoke" };
            shell(vec![s("pm"), s(verb), app.clone(), service.clone()]).await?;
            Ok(json!({ "permission": { "app": app, "service": service, "granted": grant } }))
        }
        Action::Orientation { degrees } => {
            let rotation = match degrees {
                0 => "0",
                90 => "1",
                180 => "2",
                270 => "3",
                _ => return Err("orientation is 0, 90, 180 or 270".into()),
            };
            shell(vec![
                s("settings"),
                s("put"),
                s("system"),
                s("accelerometer_rotation"),
                s("0"),
            ])
            .await?;
            shell(vec![
                s("settings"),
                s("put"),
                s("system"),
                s("user_rotation"),
                s(rotation),
            ])
            .await?;
            Ok(json!({ "orientation": degrees }))
        }
        Action::Network { wifi, data } => {
            for (name, on) in [("wifi", wifi), ("data", data)] {
                if let Some(on) = on {
                    shell(vec![
                        s("svc"),
                        s(name),
                        s(if on { "enable" } else { "disable" }),
                    ])
                    .await?;
                }
            }
            let wifi_now = shell(vec![s("settings"), s("get"), s("global"), s("wifi_on")]).await?;
            let data_now =
                shell(vec![s("settings"), s("get"), s("global"), s("mobile_data")]).await?;
            Ok(json!({ "network": { "wifi": wifi_now == "1", "data": data_now == "1" } }))
        }
        Action::Push { .. } => Err("push notifications are iOS only".into()),
    }
}

fn check_coords(lat: f64, lon: f64) -> Result<(), String> {
    if (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon) {
        Ok(())
    } else {
        Err("latitude is -90…90 and longitude -180…180".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_typed_actions() {
        let a: Action =
            serde_json::from_value(json!({ "action": "appearance", "value": "dark" })).unwrap();
        assert!(matches!(a, Action::Appearance { .. }));
        let p: Action = serde_json::from_value(
            json!({ "action": "permission", "app": "com.example.app", "service": "photos", "grant": true }),
        )
        .unwrap();
        assert!(matches!(p, Action::Permission { grant: true, .. }));
        assert!(
            serde_json::from_value::<Action>(json!({ "action": "exec", "cmd": "rm -rf /" }))
                .is_err()
        );
    }

    #[test]
    fn rejects_argument_smuggling() {
        assert!(!reverse_dns("com.app; rm -rf"));
        assert!(!word("--all"));
        assert!(!reverse_dns("-x"));
        assert!(!word("photos all"));
        assert!(reverse_dns("com.example.app"));
        assert!(check_coords(91.0, 0.0).is_err());
    }
}
