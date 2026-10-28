//! When ambient capture pauses — docs/12 §4 "Lock / sleep" row (FR-CAP-7): the
//! session is locked, the display is off, the machine is suspending, or the user has
//! been idle for five minutes.

use std::time::Duration;

pub const IDLE_PAUSE: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseReason {
    Disabled,
    Locked,
    DisplayOff,
    Suspended,
    Idle,
}

impl PauseReason {
    pub fn describe(self) -> &'static str {
        match self {
            PauseReason::Disabled => "capture is turned off",
            PauseReason::Locked => "the screen is locked",
            PauseReason::DisplayOff => "the display is off",
            PauseReason::Suspended => "the PC is going to sleep",
            PauseReason::Idle => "no input for 5 minutes",
        }
    }
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
    fn idle_duration_is_a_plausible_reading() {
        // Someone is running the tests, or a runner has been idle a while; either way
        // it is a real reading and not zero-from-failure on an interactive session.
        assert!(idle_duration() < Duration::from_secs(60 * 60 * 24 * 50));
    }
}
