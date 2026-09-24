//! Live X11 check. Needs a display with xdotool installed, e.g.
//!
//! ```sh
//! # -noreset: otherwise Xvfb resets (re-centering the pointer) each time
//! # its last client disconnects, i.e. after every xdotool call.
//! Xvfb :99 -screen 0 1920x1080x24 -noreset & DISPLAY=:99 cargo test -p mira-computer --test x11_live -- --ignored
//! ```

#![cfg(target_os = "linux")]

use std::sync::Arc;
use std::time::Duration;

use mira_computer::{backend, Computer, ComputerAction, ComputerOptions};
use serde_json::json;

#[tokio::test]
#[ignore = "needs an X display and xdotool"]
async fn screenshot_move_and_read_back_cursor() {
    let backend: Arc<dyn backend::ComputerBackend> =
        Arc::from(backend::detect().expect("x11 backend"));
    let c = Computer::new(
        backend,
        ComputerOptions {
            settle: Duration::ZERO,
            screenshot_after_action: false,
            ..Default::default()
        },
    );
    let shot = c.screenshot().await.expect("screenshot");
    assert!(shot.width > 0 && shot.height > 0);
    let (w, h) = c.display_size().await.unwrap();
    assert_eq!((w, h), (shot.width, shot.height));

    let target = [w as i32 / 3, h as i32 / 4];
    c.execute(
        &ComputerAction::from_args(&json!({"action": "mouse_move", "coordinate": target})).unwrap(),
    )
    .await
    .unwrap();
    let out = c
        .execute(&ComputerAction::from_args(&json!({"action": "cursor_position"})).unwrap())
        .await
        .unwrap();
    // Round-tripping through input space may be off by one pixel.
    let nums: Vec<i32> = out
        .text
        .trim_start_matches("Cursor at (")
        .trim_end_matches(").")
        .split(", ")
        .map(|n| n.parse().unwrap())
        .collect();
    assert!(
        (nums[0] - target[0]).abs() <= 1 && (nums[1] - target[1]).abs() <= 1,
        "{}",
        out.text
    );

    c.execute(
        &ComputerAction::from_args(&json!({"action": "left_click", "coordinate": target})).unwrap(),
    )
    .await
    .unwrap();
    c.execute(&ComputerAction::from_args(&json!({"action": "key", "text": "ctrl+a"})).unwrap())
        .await
        .unwrap();
    c.execute(&ComputerAction::from_args(&json!({"action": "type", "text": "hi there"})).unwrap())
        .await
        .unwrap();
}
