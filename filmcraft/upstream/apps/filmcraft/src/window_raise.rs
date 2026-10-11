//! Bring the window forward for the control channel without taking keyboard focus.
//!
//! Occluded macOS windows stop running egui's `ui` pass, so UI-level control requests (synthetic
//! input, screenshots) need the window on screen. Activating the app for that
//! (`ViewportCommand::Focus`) steals keyboard focus from whatever the user is typing in, and their
//! keystrokes then land in FilmCraft as shortcuts. `orderFrontRegardless` orders the window to the
//! front without activating the app, so the key window (and the user's focus) stays where it is.

/// Order the app's windows to the front without activating the app (macOS). Returns whether it
/// did anything; elsewhere the caller only requests a repaint.
pub fn raise_without_focus() -> bool {
    #[cfg(target_os = "macos")]
    {
        let Some(mtm) = objc2::MainThreadMarker::new() else { return false };
        let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
        let windows = app.windows();
        for w in windows.iter() {
            if w.isVisible() || w.isMiniaturized() {
                w.orderFrontRegardless();
            }
        }
        true
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}
