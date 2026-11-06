//! "Still in a meeting?" (spec 2026-11-05-trim-and-still-in-meeting-design.md): a recording started
//! by hand that has heard no words for 15 minutes asks, in a popup like the meeting alert (not a
//! toast: see `alert.rs`). "Keep recording" asks again after 15 more quiet minutes; "Stop", or two
//! minutes with no answer, stops it, and once transcribed it is trimmed to 30 s after the last line
//! with words (`pipeline`, `trim::auto_keep_ms`). Speech while the popup is up closes it.
//!
//! "Words" are a new live-transcript segment with text. With no live transcription (no model), the
//! recorder's audio level stands in. The clock is wall time, compared on every poll, so a PC that
//! slept through the quiet asks as soon as it wakes.

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};

use super::live::{Accumulated, Listener};
use crate::l10n::tr;

/// No words this long → ask.
pub const QUIET_MS: i64 = 15 * 60_000;
/// No answer this long → stop.
pub const ANSWER_MS: i64 = 2 * 60_000;

/// The popup's label (also listed in `capabilities/default.json`).
pub const STILL_HERE: &str = "still-here";
const SIZE: (f64, f64) = (380.0, 170.0);
const MARGIN: f64 = 12.0;

/// What a poll should do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Nothing,
    Ask,
    /// Asking ended without an answer, or with Speech: close the popup.
    Close,
    Stop,
}

/// The prompt's clock for one recording. Pure: every input is a time in unix ms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Timer {
    pub meeting_id: i64,
    last_speech_ms: i64,
    asking_since_ms: Option<i64>,
}

impl Timer {
    pub fn new(meeting_id: i64, started_ms: i64) -> Self {
        Timer { meeting_id, last_speech_ms: started_ms, asking_since_ms: None }
    }

    /// Words were heard at `at_ms` (never moves the clock back).
    pub fn heard(&mut self, at_ms: i64) -> Action {
        self.last_speech_ms = self.last_speech_ms.max(at_ms);
        if self.asking_since_ms.take().is_some() {
            Action::Close
        } else {
            Action::Nothing
        }
    }

    /// "Keep recording": no prompt again until 15 more quiet minutes.
    pub fn keep(&mut self, now_ms: i64) {
        self.last_speech_ms = self.last_speech_ms.max(now_ms);
        self.asking_since_ms = None;
    }

    pub fn asking(&self) -> bool {
        self.asking_since_ms.is_some()
    }

    pub fn tick(&mut self, now_ms: i64) -> Action {
        match self.asking_since_ms {
            Some(since) if now_ms - since >= ANSWER_MS => Action::Stop,
            Some(_) => Action::Nothing,
            None if now_ms - self.last_speech_ms >= QUIET_MS => {
                self.asking_since_ms = Some(now_ms);
                Action::Ask
            }
            None => Action::Nothing,
        }
    }
}

/// When the live transcript last gained a line with words, and whether a recording's live
/// transcription is feeding it (one recording at a time).
static LAST_SPEECH_MS: AtomicI64 = AtomicI64::new(0);
static LIVE_TRACKING: AtomicBool = AtomicBool::new(false);
static TEXT_SEGMENTS: AtomicUsize = AtomicUsize::new(0);

fn text_segments(state: &Accumulated) -> usize {
    state.system.iter().chain(state.mic.iter()).filter(|s| !s.text.trim().is_empty()).count()
}

/// Whether `state` holds more lines with words than `seen` (and how many it holds).
pub fn more_words(state: &Accumulated, seen: usize) -> (bool, usize) {
    let now = text_segments(state);
    (now > seen, now)
}

/// The live listener for a new recording: notes each new line with words, then hands the state to
/// `inner` (the question watcher, when armed). `tracking` is whether live transcription will run.
pub fn speech_listener(inner: Option<Listener>, tracking: bool) -> Listener {
    LAST_SPEECH_MS.store(0, Ordering::Relaxed);
    TEXT_SEGMENTS.store(0, Ordering::Relaxed);
    LIVE_TRACKING.store(tracking, Ordering::Relaxed);
    Listener(Arc::new(move |state: &Accumulated| {
        let (more, count) = more_words(state, TEXT_SEGMENTS.load(Ordering::Relaxed));
        if more {
            TEXT_SEGMENTS.store(count, Ordering::Relaxed);
            LAST_SPEECH_MS.store(chrono::Utc::now().timestamp_millis(), Ordering::Relaxed);
        }
        if let Some(inner) = &inner {
            (inner.0)(state);
        }
    }))
}

fn timer() -> &'static Mutex<Option<Timer>> {
    static TIMER: std::sync::OnceLock<Mutex<Option<Timer>>> = std::sync::OnceLock::new();
    TIMER.get_or_init(|| Mutex::new(None))
}

/// One poll of a recording started by hand, or of a browser call (`meetings::service`).
/// `audio_active` is the recorder's level check, used only when live transcription isn't running.
/// `present` is a sign the call is still on whatever was said: a browser's meeting tab in front
/// (Mike's choice, 2026-11-05: a browser call is asked about, never stopped silently, since Windows
/// sees only each window's front tab). Returns what to do.
pub fn poll(meeting_id: i64, started_ms: i64, now_ms: i64, audio_active: bool, present: bool) -> Action {
    let mut slot = timer().lock().unwrap();
    let t = match slot.as_mut() {
        Some(t) if t.meeting_id == meeting_id => t,
        _ => slot.insert(Timer::new(meeting_id, started_ms)),
    };
    let spoke = if present {
        Some(now_ms)
    } else if LIVE_TRACKING.load(Ordering::Relaxed) {
        Some(LAST_SPEECH_MS.load(Ordering::Relaxed)).filter(|ms| *ms > 0)
    } else {
        audio_active.then_some(now_ms)
    };
    if let Some(at) = spoke {
        if at > t.last_speech_ms && t.heard(at) == Action::Close {
            return Action::Close;
        }
    }
    t.tick(now_ms)
}

/// The recording ended: forget its timer.
pub fn reset() {
    *timer().lock().unwrap() = None;
}

/// "Keep recording" from the popup.
pub fn keep(now_ms: i64) {
    if let Some(t) = timer().lock().unwrap().as_mut() {
        t.keep(now_ms);
    }
}

pub fn show(app: &AppHandle) {
    if app.get_webview_window(STILL_HERE).is_some() {
        return;
    }
    if let Err(e) = open(app) {
        tracing::error!(error = %e, "could not open the still-in-a-meeting prompt");
    }
}

pub fn hide(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(STILL_HERE) {
        let _ = window.destroy();
    }
}

fn open(app: &AppHandle) -> tauri::Result<()> {
    let (w, h) = SIZE;
    let popup = WebviewWindowBuilder::new(app, STILL_HERE, WebviewUrl::App("index.html".into()))
        .title(tr("Still in a meeting?", &[]))
        .inner_size(w, h)
        .decorations(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .visible(false)
        .build()?;
    if let Some(monitor) = popup.primary_monitor()? {
        let scale = monitor.scale_factor();
        let area = monitor.work_area();
        let (x, mut y) = crate::alert::alert_origin(
            ((w * scale).round() as u32, (h * scale).round() as u32),
            (area.position.x, area.position.y, area.size.width, area.size.height),
            (MARGIN * scale).round() as i32,
        );
        for (label, height) in [(crate::alert::ALERT, 190.0), (crate::question_alert::QUESTION_ALERT, 210.0)] {
            if app.get_webview_window(label).is_some() {
                y += ((height + MARGIN) * scale).round() as i32;
            }
        }
        popup.set_position(PhysicalPosition::new(x, y))?;
    }
    popup.show()?;
    Ok(())
}

/// "Keep recording".
#[tauri::command]
pub fn still_here_keep(app: AppHandle) {
    keep(chrono::Utc::now().timestamp_millis());
    hide(&app);
}

/// "Stop": stops the recording; it is trimmed to the last words once transcribed.
#[tauri::command]
pub async fn still_here_stop(app: AppHandle) -> Result<(), crate::error::CommandError> {
    hide(&app);
    let state = app.state::<crate::state::AppState>();
    let meetings = state.meetings()?.clone();
    tauri::async_runtime::spawn_blocking(move || meetings.stop_quiet().map(|_| ()))
        .await
        .map_err(|e| crate::error::CommandError::store(e.to_string()))?
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;

    #[test]
    fn quiet_for_15_minutes_asks_then_stops_after_2() {
        let mut t = Timer::new(1, 0);
        assert_eq!(t.tick(14 * MIN), Action::Nothing);
        assert_eq!(t.tick(15 * MIN), Action::Ask);
        assert_eq!(t.tick(16 * MIN), Action::Nothing, "asked once");
        assert_eq!(t.tick(17 * MIN), Action::Stop);
    }

    #[test]
    fn words_reset_the_quiet_and_close_an_open_prompt() {
        let mut t = Timer::new(1, 0);
        assert_eq!(t.heard(10 * MIN), Action::Nothing);
        assert_eq!(t.tick(24 * MIN), Action::Nothing, "15 minutes from the last words, not the start");
        assert_eq!(t.tick(25 * MIN), Action::Ask);
        assert_eq!(t.heard(26 * MIN), Action::Close);
        assert!(!t.asking());
        assert_eq!(t.tick(40 * MIN), Action::Nothing);
        assert_eq!(t.tick(41 * MIN), Action::Ask);
    }

    #[test]
    fn keep_recording_asks_again_after_15_more_quiet_minutes() {
        let mut t = Timer::new(1, 0);
        assert_eq!(t.tick(15 * MIN), Action::Ask);
        t.keep(16 * MIN);
        assert_eq!(t.tick(30 * MIN), Action::Nothing);
        assert_eq!(t.tick(31 * MIN), Action::Ask);
    }

    #[test]
    fn a_pc_that_slept_through_the_quiet_asks_on_its_first_poll_awake() {
        let mut t = Timer::new(1, 0);
        assert_eq!(t.tick(3 * 60 * MIN), Action::Ask);
    }

    #[test]
    fn a_new_line_with_words_counts_and_blank_ones_do_not() {
        let seg = |text: &str| transcribe::TranscriptSegment { start_cs: 0, end_cs: 100, text: text.into() };
        let mut state = Accumulated::default();
        state.system.push(seg("  "));
        assert_eq!(more_words(&state, 0), (false, 0));
        state.mic.push(seg("hello"));
        assert_eq!(more_words(&state, 0), (true, 1));
        assert_eq!(more_words(&state, 1), (false, 1));
    }

    #[test]
    fn without_live_transcription_audible_audio_stands_in() {
        LIVE_TRACKING.store(false, Ordering::Relaxed);
        reset();
        assert_eq!(poll(7, 0, 14 * MIN, true, false), Action::Nothing);
        assert_eq!(poll(7, 0, 28 * MIN, false, false), Action::Nothing, "the sound at 14 min reset the quiet");
        assert_eq!(poll(7, 0, 29 * MIN, false, false), Action::Ask);
        reset();

        // A browser call: its meeting tab in front counts as the call going on. One test, as
        // both halves share the process-wide timer.
        LIVE_TRACKING.store(true, Ordering::Relaxed);
        LAST_SPEECH_MS.store(0, Ordering::Relaxed);
        reset();
        assert_eq!(poll(8, 0, 20 * MIN, false, true), Action::Nothing, "the tab in front: no prompt however quiet");
        assert_eq!(poll(8, 0, 34 * MIN, false, false), Action::Nothing, "15 minutes from when the tab was last in front");
        assert_eq!(poll(8, 0, 35 * MIN, false, false), Action::Ask);
        assert_eq!(poll(8, 0, 36 * MIN, false, true), Action::Close, "back on the meeting tab: the prompt closes");
        reset();
        LIVE_TRACKING.store(false, Ordering::Relaxed);
    }
}
