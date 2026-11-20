//! Keeping fixed-length runs at full speed when the window cannot be seen.
//!
//! macOS puts a process whose windows are all hidden (behind other windows,
//! on another space, or under a locked screen) into App Nap: its timers are
//! coalesced and its threads run at a lower priority. For a measurement that
//! turns into frames ten times slower partway through a run. An activity
//! declared through `NSProcessInfo` keeps the process out of App Nap for as
//! long as it is held.

/// Declares a latency-critical, user-initiated activity that lasts until the
/// process exits. Used only by `--exit-after-frames` runs; normal play may
/// nap when hidden, which saves battery.
#[cfg(target_os = "macos")]
pub fn stay_out_of_app_nap() {
    use objc2_foundation::{NSActivityOptions, NSProcessInfo, NSString};
    let reason = NSString::from_str("World: fixed-length measurement run");
    let activity = NSProcessInfo::processInfo().beginActivityWithOptions_reason(
        NSActivityOptions::UserInitiated | NSActivityOptions::LatencyCritical,
        &reason,
    );
    // The activity ends when its token is released; the run ends with the
    // process, so it is never released.
    std::mem::forget(activity);
}

#[cfg(not(target_os = "macos"))]
pub fn stay_out_of_app_nap() {}

#[cfg(test)]
mod tests {
    #[test]
    fn declaring_the_activity_is_harmless_and_repeatable() {
        super::stay_out_of_app_nap();
        super::stay_out_of_app_nap();
    }
}
