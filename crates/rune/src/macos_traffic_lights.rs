//! The traffic lights at the app's own spot (`Chrome::traffic_lights`).

use objc2_app_kit::{NSWindowButton, NSWindowStyleMask};
use objc2_foundation::NSPoint;
use winit::window::Window;

use crate::macos::ns_window;

/// Electron's `trafficLightPosition`: the titlebar container grows to the buttons' height
/// plus twice `y`, and the buttons sit `x, y` from the top-left at AppKit's own pitch.
/// AppKit lays the titlebar out again on resize, so the shell calls this after each.
pub(crate) fn place(window: &Window, (x, y): (f32, f32)) {
    let Some(win) = ns_window(window) else { return };
    if win.styleMask().contains(NSWindowStyleMask::FullScreen) {
        return;
    }
    let buttons = [NSWindowButton::NSWindowCloseButton, NSWindowButton::NSWindowMiniaturizeButton, NSWindowButton::NSWindowZoomButton]
        .map(|b| win.standardWindowButton(b));
    let [Some(close), Some(mini), Some(zoom)] = buttons else { return };
    // SAFETY: AppKit's own view hierarchy, on the main thread.
    let Some(container) = (unsafe { close.superview().and_then(|v| v.superview()) }) else { return };
    let (x, y) = (f64::from(x), f64::from(y));
    let pitch = mini.frame().origin.x - close.frame().origin.x;
    let mut frame = container.frame();
    frame.size.height = close.frame().size.height + 2.0 * y;
    frame.origin.y = win.frame().size.height - frame.size.height;
    unsafe {
        container.setFrame(frame);
        for (i, b) in [close, mini, zoom].iter().enumerate() {
            b.setFrameOrigin(NSPoint::new(x + i as f64 * pitch, y));
        }
    }
}
