//! Settings → "Keep screen awake while a session is running".
//!
//! While the setting is on and any chat is mid-turn (or has background work),
//! Mira holds the OS's own stay-awake assertion: `caffeinate -di` on macOS
//! (display and idle sleep), `systemd-inhibit` on Linux. The helper is
//! watched by pid on macOS and killed on drop, so it can't outlive Mira.
//! Windows has no helper yet; the setting is a no-op there.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::state::AppState;

static ENABLED: AtomicBool = AtomicBool::new(false);
const CHECK_EVERY: Duration = Duration::from_secs(5);

/// Turn the setting on or off; applied on the next check.
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::SeqCst);
}

fn hold() -> Option<tokio::process::Child> {
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = tokio::process::Command::new("caffeinate");
        // -d display, -i idle sleep; -w: end with Mira even if it crashes.
        c.args(["-di", "-w", &std::process::id().to_string()]);
        c
    } else if cfg!(target_os = "linux") {
        let mut c = tokio::process::Command::new("systemd-inhibit");
        c.args(["--what=idle:sleep", "--who=Mira", "--why=A chat is running", "--mode=block", "sleep", "infinity"]);
        c
    } else {
        return None;
    };
    cmd.kill_on_drop(true)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    match cmd.spawn() {
        Ok(child) => Some(child),
        Err(e) => {
            tracing::warn!(error = %e, "keep-awake helper unavailable");
            None
        }
    }
}

/// Start watching. Reads the setting once from config; later changes arrive
/// through [`set_enabled`].
pub fn spawn(state: AppState) {
    set_enabled(
        mira_config::MiraConfig::load_global()
            .map(|c| c.sessions.keep_awake())
            .unwrap_or(false),
    );
    tokio::spawn(async move {
        let mut held: Option<tokio::process::Child> = None;
        let mut tick = tokio::time::interval(CHECK_EVERY);
        loop {
            tick.tick().await;
            let want = ENABLED.load(Ordering::SeqCst) && any_running(&state).await;
            // A helper that exited on its own (killed by hand) is re-taken.
            if let Some(child) = held.as_mut() {
                if matches!(child.try_wait(), Ok(Some(_))) {
                    held = None;
                }
            }
            match (want, held.is_some()) {
                (true, false) => {
                    held = hold();
                    if held.is_some() {
                        tracing::info!("keeping the machine awake while a chat runs");
                    }
                }
                (false, true) => {
                    held = None; // kill_on_drop releases the assertion
                    tracing::info!("released keep-awake");
                }
                _ => {}
            }
        }
    });
}

async fn any_running(state: &AppState) -> bool {
    for slot in state.list_slots().await {
        if slot.is_running().await {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn the_helper_holds_until_dropped() {
        let mut child = hold().expect("caffeinate is part of macOS");
        assert!(matches!(child.try_wait(), Ok(None)), "still holding");
        let pid = child.id().unwrap();
        drop(child);
        tokio::time::sleep(Duration::from_millis(300)).await;
        // kill_on_drop: the assertion is gone with it.
        let alive = std::process::Command::new("kill").args(["-0", &pid.to_string()]).status().unwrap().success();
        assert!(!alive, "caffeinate {pid} outlived its handle");
    }
}
