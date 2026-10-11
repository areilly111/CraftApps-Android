//! Opt out of macOS App Nap.
//!
//! When the window is hidden or behind other windows (always the case when the app is driven over
//! the control channel, and common while previews render or a sequence plays on a second
//! display), App Nap lowers every thread of the process to background priority (4) and defers
//! its timers. On a busy machine the app then gets no CPU at all: the UI thread stays runnable
//! inside the run loop's timer callout, frame workers and the control channel stall, and the app
//! looks hung (seen after Render In to Out + play: 0 % CPU, every thread at priority 4, recovered
//! instantly with App Nap off). An editor must keep playing and rendering in the background, so
//! the process holds a user-initiated, latency-critical activity for its whole lifetime, which
//! is what `NSAppSleepDisabled` does for an app bundle. Idle system sleep stays allowed.

/// Begin the activity (macOS); returns whether it is held. Call once at startup.
pub fn disable() -> bool {
    #[cfg(target_os = "macos")]
    {
        use objc2_foundation::{NSActivityOptions, NSProcessInfo, NSString};
        let options = NSActivityOptions::UserInitiatedAllowingIdleSystemSleep | NSActivityOptions::LatencyCritical;
        let reason = NSString::from_str("FilmCraft plays and renders video in the background");
        let token = NSProcessInfo::processInfo().beginActivityWithOptions_reason(options, &reason);
        // The activity lasts while the token is alive: keep it for the life of the process.
        std::mem::forget(token);
        true
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn app_nap_is_disabled_on_macos() {
        assert_eq!(super::disable(), cfg!(target_os = "macos"));
    }
}
