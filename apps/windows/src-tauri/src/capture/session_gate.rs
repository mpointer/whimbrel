//! When ambient capture pauses — docs/12 §4 "Lock / sleep" row (FR-CAP-7): the
//! session is locked, the display is off, the machine is suspending, or the user has
//! been idle for five minutes.

use std::time::Duration;

pub const IDLE_PAUSE: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseReason {
    Disabled,
    /// A timed pause the person chose ("Pause for 15 minutes"). `capture.enabled` stays true while it runs.
    TimedPause,
    Locked,
    DisplayOff,
    Suspended,
    Idle,
}

impl PauseReason {
    pub fn describe(self) -> &'static str {
        match self {
            PauseReason::Disabled => "capture is turned off",
            PauseReason::TimedPause => "capture is paused for a set time",
            PauseReason::Locked => "the screen is locked",
            PauseReason::DisplayOff => "the display is off",
            PauseReason::Suspended => "the PC is going to sleep",
            PauseReason::Idle => "no input for 5 minutes",
        }
    }
}

/// Whether a timed pause with this deadline is still running at `now_ms` (wall clock, unix ms). A deadline is
/// compared with the clock on every check, never trusted to a timer, so sleep cannot stretch or lose a pause.
pub fn timed_pause_active(resume_at_ms: Option<i64>, now_ms: i64) -> bool {
    resume_at_ms.is_some_and(|at| now_ms < at)
}

/// What startup keeps of a stored deadline: it while it is still ahead, nothing once it has passed (the pause
/// is over, and capture is back to what the on setting says).
pub fn deadline_to_keep(stored: Option<i64>, now_ms: i64) -> Option<i64> {
    stored.filter(|at| *at > now_ms)
}

/// The session state the capture loop learns from system notifications.
#[derive(Debug, Clone, Copy, Default)]
pub struct SessionGate {
    pub locked: bool,
    pub display_off: bool,
    pub suspended: bool,
}

impl SessionGate {
    pub fn pause_reason(&self, idle: Duration) -> Option<PauseReason> {
        if self.suspended {
            Some(PauseReason::Suspended)
        } else if self.locked {
            Some(PauseReason::Locked)
        } else if self.display_off {
            Some(PauseReason::DisplayOff)
        } else if idle >= IDLE_PAUSE {
            Some(PauseReason::Idle)
        } else {
            None
        }
    }
}

/// Time since the last keyboard or mouse input in this session (`GetLastInputInfo`).
pub fn idle_duration() -> Duration {
    use windows::Win32::System::SystemInformation::GetTickCount;
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    let mut info = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    unsafe {
        if GetLastInputInfo(&mut info).as_bool() {
            // Both are 32-bit tick counts; wrapping_sub survives the 49.7-day wrap.
            Duration::from_millis(GetTickCount().wrapping_sub(info.dwTime) as u64)
        } else {
            Duration::ZERO
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_active_unlocked_session_captures() {
        assert_eq!(SessionGate::default().pause_reason(Duration::from_secs(10)), None);
    }

    #[test]
    fn each_condition_pauses() {
        let active = Duration::from_secs(1);
        assert_eq!(SessionGate { locked: true, ..Default::default() }.pause_reason(active), Some(PauseReason::Locked));
        assert_eq!(SessionGate { display_off: true, ..Default::default() }.pause_reason(active), Some(PauseReason::DisplayOff));
        assert_eq!(SessionGate { suspended: true, ..Default::default() }.pause_reason(active), Some(PauseReason::Suspended));
        assert_eq!(SessionGate::default().pause_reason(IDLE_PAUSE), Some(PauseReason::Idle));
    }

    #[test]
    fn a_timed_pause_runs_until_its_deadline_and_not_after() {
        assert!(!timed_pause_active(None, 1_000), "no deadline, no pause");
        assert!(timed_pause_active(Some(2_000), 1_999));
        assert!(!timed_pause_active(Some(2_000), 2_000), "the deadline itself is the first moment back on");
        assert!(!timed_pause_active(Some(2_000), 9_000), "a deadline passed during sleep ends the pause");
    }

    #[test]
    fn startup_keeps_a_future_deadline_and_drops_a_past_one() {
        assert_eq!(deadline_to_keep(Some(5_000), 4_000), Some(5_000), "app starts, time not reached: still paused");
        assert_eq!(deadline_to_keep(Some(5_000), 5_000), None);
        assert_eq!(deadline_to_keep(Some(5_000), 6_000), None, "time reached while the app was closed: on at this start");
        assert_eq!(deadline_to_keep(None, 6_000), None);
    }

    #[test]
    fn a_timed_pause_has_its_own_reason() {
        assert_eq!(PauseReason::TimedPause.describe(), "capture is paused for a set time");
        assert_ne!(PauseReason::TimedPause, PauseReason::Disabled);
    }

    #[test]
    fn idle_duration_is_a_plausible_reading() {
        // Someone is running the tests, or a runner has been idle a while; either way
        // it is a real reading and not zero-from-failure on an interactive session.
        assert!(idle_duration() < Duration::from_secs(60 * 60 * 24 * 50));
    }
}
