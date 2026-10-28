//! FR-MTG-5: when a recording meeting should end. Ported from apps/macos
//! `MeetingEndDetector.evaluate`, which now calls this through the FFI so both shells
//! share one rule set. Adds one signal the Mac can't observe today: the meeting app
//! releasing its microphone (issue #84). On Windows `IAudioSessionManager2` reports a
//! per-process capture session; when Zoom or Teams drops back to its lobby it releases
//! that session while the process keeps running, which the silence rule alone misses.

use serde::{Deserialize, Serialize};

/// One poll's observations, gathered by the shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndSignals {
    pub now_unix_ms: i64,
    pub recording_started_unix_ms: i64,
    /// `false` once the detected meeting's app has quit. `true` when no app is watched.
    pub app_running: bool,
    /// `Some` only while a browser meeting tab is tracked: is it still on the meeting?
    pub on_meeting_tab: Option<bool>,
    /// The calendar event's end; `None` for detected and manual meetings.
    pub scheduled_end_unix_ms: Option<i64>,
    /// Audible content captured recently (the recorder's peak-level check).
    pub audio_active: bool,
    /// `Some` only when the shell watches the meeting app's own capture session: is it
    /// still active? macOS passes `None`.
    pub capture_session_active: Option<bool>,
}

/// Timings, in milliseconds. [`Default`] is the Mac's values plus the capture-release
/// grace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndThresholds {
    /// Nothing but an app quit may end a recording younger than this. Live bug
    /// (2026-08-27): a late-accepted calendar recording hit the hard ceiling on its
    /// first poll, and the recalibrated silence threshold (docs/superpowers/specs/
    /// 2026-08-06-silence-threshold-recalibration-design.md) can read a quiet first
    /// minute (people still joining) as silence.
    pub minimum_recording_age_ms: i64,
    pub silence_ms: i64,
    /// Shorter silence once a calendar meeting's scheduled end has passed.
    pub silence_after_scheduled_end_ms: i64,
    /// Losing the meeting URL is a stronger signal than silence, but not instant: one
    /// poll on a different tab (a brief switch, an AX read hiccup) isn't proof.
    pub browser_tab_gone_grace_ms: i64,
    /// Ends a calendar meeting this far past its scheduled end regardless of audio,
    /// because the peak-level silence check isn't calibrated for every mic or room and
    /// was seen live to never fire.
    pub scheduled_end_hard_ceiling_ms: i64,
    /// Last-resort cap for any meeting, including detected ones with no scheduled end.
    pub max_recording_duration_ms: i64,
    /// How long the meeting app's capture session must stay inactive. Provisional:
    /// M3.3 Task 6 measures mute, device switches and lobby returns on real Teams and
    /// Zoom calls before this number is settled.
    pub capture_released_grace_ms: i64,
}

impl Default for EndThresholds {
    fn default() -> Self {
        Self {
            minimum_recording_age_ms: 120_000,
            silence_ms: 60_000,
            silence_after_scheduled_end_ms: 15_000,
            browser_tab_gone_grace_ms: 15_000,
            scheduled_end_hard_ceiling_ms: 10 * 60_000,
            max_recording_duration_ms: 4 * 60 * 60_000,
            capture_released_grace_ms: 20_000,
        }
    }
}

/// Clocks [`evaluate`] carries between polls.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndState {
    pub silence_started_unix_ms: Option<i64>,
    pub browser_tab_gone_started_unix_ms: Option<i64>,
    pub capture_released_started_unix_ms: Option<i64>,
}

/// Why a recording ended, for the `[meeting-end]` log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndReason {
    AppQuit,
    CaptureReleased,
    BrowserTabGone,
    ScheduledEndHardCeiling,
    MaxDuration,
    Silence,
}

/// Rules in priority order: app quit (immediate, ignores the minimum age) → capture
/// released → browser tab gone → scheduled end + ceiling → max duration → silence. Every
/// rule but app quit and max duration waits for the minimum recording age, and every
/// grace clock starts on the first poll that sees the condition, even inside that age.
pub fn evaluate(signals: &EndSignals, thresholds: &EndThresholds, state: &mut EndState) -> Option<EndReason> {
    let now = signals.now_unix_ms;
    if !signals.app_running {
        return Some(EndReason::AppQuit);
    }

    let age = now - signals.recording_started_unix_ms;
    let grace_passed = age >= thresholds.minimum_recording_age_ms;

    if let Some(active) = signals.capture_session_active {
        if active {
            state.capture_released_started_unix_ms = None;
        } else {
            let since = *state.capture_released_started_unix_ms.get_or_insert(now);
            if grace_passed && now - since >= thresholds.capture_released_grace_ms {
                return Some(EndReason::CaptureReleased);
            }
        }
    }

    if let Some(on_tab) = signals.on_meeting_tab {
        if on_tab {
            state.browser_tab_gone_started_unix_ms = None;
        } else {
            let since = *state.browser_tab_gone_started_unix_ms.get_or_insert(now);
            if grace_passed && now - since >= thresholds.browser_tab_gone_grace_ms {
                return Some(EndReason::BrowserTabGone);
            }
        }
    }

    if let Some(end) = signals.scheduled_end_unix_ms {
        if grace_passed && now >= end + thresholds.scheduled_end_hard_ceiling_ms {
            return Some(EndReason::ScheduledEndHardCeiling);
        }
    }
    if age >= thresholds.max_recording_duration_ms {
        return Some(EndReason::MaxDuration);
    }

    if signals.audio_active {
        state.silence_started_unix_ms = None;
        return None;
    }
    let silence_started = *state.silence_started_unix_ms.get_or_insert(now);
    if !grace_passed {
        return None;
    }
    let required = match signals.scheduled_end_unix_ms {
        Some(end) if now >= end => thresholds.silence_after_scheduled_end_ms,
        _ => thresholds.silence_ms,
    };
    (now - silence_started >= required).then_some(EndReason::Silence)
}

#[cfg(test)]
mod tests {
    //! The first ten tests port apps/macos `MeetingEndDetectorTests` one for one.
    use super::*;

    const START: i64 = 1_000_000_000;
    const S: i64 = 1_000;

    fn at(offset_s: i64) -> EndSignals {
        EndSignals {
            now_unix_ms: START + offset_s * S,
            recording_started_unix_ms: START,
            app_running: true,
            on_meeting_tab: None,
            scheduled_end_unix_ms: None,
            audio_active: true,
            capture_session_active: None,
        }
    }

    fn eval(signals: EndSignals, state: &mut EndState) -> Option<EndReason> {
        evaluate(&signals, &EndThresholds::default(), state)
    }

    #[test]
    fn app_quit_fires_instantly_even_within_grace_period() {
        let mut state = EndState::default();
        assert_eq!(eval(EndSignals { app_running: false, ..at(5) }, &mut state), Some(EndReason::AppQuit));
    }

    #[test]
    fn hard_ceiling_does_not_fire_within_grace_period_even_if_already_past_ceiling() {
        let mut state = EndState::default();
        let signals = EndSignals { scheduled_end_unix_ms: Some(START - 11 * 60 * S), ..at(5) };
        assert_eq!(eval(signals, &mut state), None);
    }

    #[test]
    fn hard_ceiling_fires_once_grace_period_elapses() {
        let mut state = EndState::default();
        let signals = EndSignals { scheduled_end_unix_ms: Some(START - 11 * 60 * S), ..at(125) };
        assert_eq!(eval(signals, &mut state), Some(EndReason::ScheduledEndHardCeiling));
    }

    #[test]
    fn silence_does_not_fire_within_grace_period() {
        let mut state = EndState::default();
        let mut reason = None;
        for offset in (5..=115).step_by(5) {
            reason = eval(EndSignals { audio_active: false, ..at(offset) }, &mut state);
        }
        assert_eq!(reason, None);
    }

    #[test]
    fn silence_fires_after_grace_period_and_required_window() {
        let mut state = EndState::default();
        let mut reason = None;
        for offset in (5..=185).step_by(5) {
            reason = eval(EndSignals { audio_active: false, ..at(offset) }, &mut state);
            if reason.is_some() {
                break;
            }
        }
        assert_eq!(reason, Some(EndReason::Silence));
    }

    #[test]
    fn audio_activity_resets_silence_clock() {
        let mut state = EndState::default();
        eval(EndSignals { audio_active: false, ..at(150) }, &mut state);
        assert!(state.silence_started_unix_ms.is_some());
        eval(at(155), &mut state);
        assert_eq!(state.silence_started_unix_ms, None);
    }

    #[test]
    fn browser_tab_gone_does_not_fire_within_grace_period() {
        let mut state = EndState::default();
        assert_eq!(eval(EndSignals { on_meeting_tab: Some(false), ..at(10) }, &mut state), None);
    }

    #[test]
    fn browser_tab_gone_fires_after_grace_period_and_own_grace_window() {
        let mut state = EndState::default();
        eval(EndSignals { on_meeting_tab: Some(false), ..at(130) }, &mut state);
        let reason = eval(EndSignals { on_meeting_tab: Some(false), ..at(150) }, &mut state);
        assert_eq!(reason, Some(EndReason::BrowserTabGone));
    }

    #[test]
    fn returning_to_meeting_tab_resets_browser_tab_gone_clock() {
        let mut state = EndState::default();
        eval(EndSignals { on_meeting_tab: Some(false), ..at(130) }, &mut state);
        assert!(state.browser_tab_gone_started_unix_ms.is_some());
        eval(EndSignals { on_meeting_tab: Some(true), ..at(135) }, &mut state);
        assert_eq!(state.browser_tab_gone_started_unix_ms, None);
    }

    #[test]
    fn shorter_silence_window_applies_once_past_scheduled_end() {
        let mut state = EndState::default();
        let end = Some(START + 150 * S);
        let mut reason = None;
        for offset in (155..=175).step_by(5) {
            reason = eval(
                EndSignals { scheduled_end_unix_ms: end, audio_active: false, ..at(offset) },
                &mut state,
            );
            if reason.is_some() {
                break;
            }
        }
        assert_eq!(reason, Some(EndReason::Silence));
    }

    #[test]
    fn max_duration_fires_for_a_meeting_with_no_scheduled_end() {
        let mut state = EndState::default();
        assert_eq!(eval(at(4 * 60 * 60), &mut state), Some(EndReason::MaxDuration));
    }

    // ---- capture released (issue #84, Windows) ----

    #[test]
    fn capture_released_does_not_fire_before_the_minimum_age() {
        let mut state = EndState::default();
        let mut reason = None;
        for offset in (5..=115).step_by(5) {
            reason = eval(EndSignals { capture_session_active: Some(false), ..at(offset) }, &mut state);
        }
        assert_eq!(reason, None);
    }

    #[test]
    fn capture_released_fires_after_its_grace_even_while_audio_plays() {
        let mut state = EndState::default();
        eval(EndSignals { capture_session_active: Some(false), ..at(130) }, &mut state);
        assert_eq!(
            eval(EndSignals { capture_session_active: Some(false), ..at(145) }, &mut state),
            None,
            "15 s is inside the 20 s grace"
        );
        assert_eq!(
            eval(EndSignals { capture_session_active: Some(false), ..at(150) }, &mut state),
            Some(EndReason::CaptureReleased)
        );
    }

    #[test]
    fn capture_reacquired_resets_the_clock() {
        let mut state = EndState::default();
        eval(EndSignals { capture_session_active: Some(false), ..at(130) }, &mut state);
        eval(EndSignals { capture_session_active: Some(true), ..at(140) }, &mut state);
        assert_eq!(state.capture_released_started_unix_ms, None);
        assert_eq!(eval(EndSignals { capture_session_active: Some(false), ..at(150) }, &mut state), None);
    }

    #[test]
    fn capture_released_outranks_silence() {
        let mut state = EndState::default();
        for offset in (130..=200).step_by(5) {
            let signals = EndSignals { capture_session_active: Some(false), audio_active: false, ..at(offset) };
            if let Some(reason) = eval(signals, &mut state) {
                assert_eq!(reason, EndReason::CaptureReleased);
                return;
            }
        }
        panic!("expected an end");
    }

    #[test]
    fn unwatched_capture_session_never_ends_a_meeting() {
        let mut state = EndState::default();
        assert_eq!(eval(at(3 * 60 * 60), &mut state), None);
        assert_eq!(state.capture_released_started_unix_ms, None);
    }
}
