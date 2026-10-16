//! Native window-button geometry. CSS and AppKit share the centre line via
//! the desktop initialization script; no Tauri webview inset is involved.
#[cfg(target_os = "macos")]
use objc2::rc::Retained;
#[cfg(target_os = "macos")]
use objc2_app_kit::{NSButton, NSView, NSWindow};

#[cfg(target_os = "macos")]
pub const LEFT: f64 = 20.0;
pub const CENTER_Y: f64 = 30.0;

/// The close button and the title-bar container that holds all three.
#[cfg(target_os = "macos")]
fn title_bar(window: &NSWindow) -> Option<(Retained<NSButton>, Retained<NSView>)> {
    let close = window.standardWindowButton(objc2_app_kit::NSWindowButton::CloseButton)?;
    // SAFETY: these are retained standard AppKit views, used synchronously
    // on the main thread without mutating their ownership hierarchy.
    let parent = unsafe { close.superview() }?;
    // SAFETY: as above; retain the ancestor before moving its frame.
    let container = unsafe { parent.superview() }?;
    Some((close, container))
}

#[cfg(target_os = "macos")]
pub fn position(window: &NSWindow) {
    use objc2_app_kit::{NSWindowButton, NSWindowStyleMask};
    use objc2_foundation::NSPoint;

    // macOS owns the menu-bar controls in fullscreen. Leave those alone;
    // the normal window is positioned again when it leaves fullscreen.
    if window.styleMask().contains(NSWindowStyleMask::FullScreen) {
        return;
    }
    let Some((close, container)) = title_bar(window) else {
        return;
    };
    // SAFETY: as in `title_bar`.
    let Some(frame) = (unsafe { container.superview() }) else {
        return;
    };

    // Measure against the window's frame view rather than `window.frame()`:
    // mid-resize AppKit has already laid the views out at the new size while
    // the window frame still reports the old one. Moving the existing
    // container preserves the buttons' own frames, spacing and native
    // hover/hit regions, including background windows.
    let close_frame = close.frame();
    let current = close.convertPoint_toView(
        NSPoint::new(0.0, close_frame.size.height / 2.0),
        Some(&frame),
    );
    let from_top = if frame.isFlipped() {
        current.y
    } else {
        frame.bounds().size.height - current.y
    };
    let delta_y = from_top - CENTER_Y;
    if delta_y.abs() > 0.1 {
        let mut origin = container.frame().origin;
        origin.y += if frame.isFlipped() { -delta_y } else { delta_y };
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

/// AppKit lays the title bar out again on resize, focus, fullscreen and
/// appearance changes, which snaps the buttons back to their default spot
/// (centred 16pt down). Put them back whenever that happens: at once for the
/// window's own notifications, which arrive after layout, and on the next
/// turn of the run loop when the container's frame changes, since AppKit is
/// still mid-layout when it reports that and would undo a move made there.
///
/// Everything here stays in AppKit. These notifications can fire while
/// Tauri is dispatching a window event, and calling back into Tauri from
/// there panics on its borrowed window state.
#[cfg(target_os = "macos")]
pub fn keep_positioned(window: &NSWindow) {
    use block2::RcBlock;
    use objc2::runtime::AnyObject;
    use objc2_app_kit::{
        NSViewFrameDidChangeNotification, NSWindowDidBecomeKeyNotification,
        NSWindowDidChangeBackingPropertiesNotification, NSWindowDidEndLiveResizeNotification,
        NSWindowDidExitFullScreenNotification, NSWindowDidResignKeyNotification,
        NSWindowDidResizeNotification,
    };
    use objc2_foundation::{NSNotification, NSNotificationCenter, NSOperationQueue};
    use std::ptr::NonNull;

    let Some((_, container)) = title_bar(window) else {
        return;
    };
    // SAFETY (both blocks): AppKit passes a live notification for the
    // duration of the call; its object is the window or view observed below.
    let now = RcBlock::new(|note: NonNull<NSNotification>| {
        let note = unsafe { note.as_ref() };
        if let Some(window) = note.object().and_then(|o| o.downcast::<NSWindow>().ok()) {
            position(&window);
        }
    });
    let later = RcBlock::new(|note: NonNull<NSNotification>| {
        let note = unsafe { note.as_ref() };
        let Some(window) = note
            .object()
            .and_then(|o| o.downcast::<NSView>().ok())
            .and_then(|view| view.window())
        else {
            return;
        };
        let place = RcBlock::new(move || position(&window));
        // SAFETY: the main queue runs the block on the main thread.
        unsafe { NSOperationQueue::mainQueue().addOperationWithBlock(&place) };
    });

    let center = NSNotificationCenter::defaultCenter();
    let window: &AnyObject = window.as_ref();
    let container_object: &AnyObject = container.as_ref();
    container.setPostsFrameChangedNotifications(true);
    // SAFETY: the names are AppKit's own constants, the observed objects are
    // live AppKit objects, and these notifications are posted on the main
    // thread. The centre keeps each observer for the life of the window,
    // which is the life of the app.
    unsafe {
        for name in [
            NSWindowDidResizeNotification,
            NSWindowDidEndLiveResizeNotification,
            NSWindowDidBecomeKeyNotification,
            NSWindowDidResignKeyNotification,
            NSWindowDidExitFullScreenNotification,
            NSWindowDidChangeBackingPropertiesNotification,
        ] {
            center.addObserverForName_object_queue_usingBlock(Some(name), Some(window), None, &now);
        }
        center.addObserverForName_object_queue_usingBlock(
            Some(NSViewFrameDidChangeNotification),
            Some(container_object),
            None,
            &later,
        );
    }
}
