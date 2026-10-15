//! Native window-button geometry. CSS and AppKit share the centre line via
//! the desktop initialization script; no Tauri webview inset is involved.
#[cfg(target_os = "macos")]
pub const LEFT: f64 = 20.0;
pub const CENTER_Y: f64 = 30.0;

#[cfg(target_os = "macos")]
pub fn position(window: &objc2_app_kit::NSWindow) {
    use objc2_app_kit::{NSWindowButton, NSWindowStyleMask};
    use objc2_foundation::NSPoint;

    // macOS owns the menu-bar controls in fullscreen. Leave those alone;
    // the normal window is positioned again on its resize/focus events.
    if window.styleMask().contains(NSWindowStyleMask::FullScreen) {
        return;
    }
    let Some(close) = window.standardWindowButton(NSWindowButton::CloseButton) else {
        return;
    };
    // SAFETY: these are retained standard AppKit views, used synchronously
    // on the main thread without mutating their ownership hierarchy.
    let Some(parent) = (unsafe { close.superview() }) else {
        return;
    };
    // SAFETY: as above; retain the ancestor before moving its frame.
    let Some(container) = (unsafe { parent.superview() }) else {
        return;
    };

    // Measure in window coordinates (AppKit's origin is bottom-left).
    // Moving the existing container preserves the buttons' own frames,
    // spacing and native hover/hit regions, including background windows.
    let close_frame = close.frame();
    let current = close.convertPoint_toView(NSPoint::new(0.0, close_frame.size.height / 2.0), None);
    let delta_y = window.frame().size.height - CENTER_Y - current.y;
    if delta_y.abs() > 0.1 {
        let mut origin = container.frame().origin;
        origin.y += delta_y;
        container.setFrameOrigin(origin);
    }

    // Keep native button spacing; only translate their horizontal positions.
    let delta_x = LEFT - current.x;
    if delta_x.abs() > 0.1 {
        for kind in [
            NSWindowButton::CloseButton,
            NSWindowButton::MiniaturizeButton,
            NSWindowButton::ZoomButton,
        ] {
            if let Some(button) = window.standardWindowButton(kind) {
                let mut origin = button.frame().origin;
                origin.x += delta_x;
                button.setFrameOrigin(origin);
            }
        }
    }
}
