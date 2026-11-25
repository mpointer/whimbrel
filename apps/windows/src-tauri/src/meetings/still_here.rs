//! "Still in a meeting?" (spec 2026-11-05-trim-and-still-in-meeting-design.md): a recording started
//! by hand that has heard no words for 15 minutes asks, in a popup like the meeting alert (not a
//! toast: see `alert.rs`). "Keep recording" asks again after 15 more quiet minutes; "Stop", or two
//! minutes with no answer, stops it, and once transcribed it is trimmed to 30 s after the last line
//! with words (`pipeline`, `trim::auto_keep_ms`). Speech while the popup is up closes it.
//!
//! "Words" are a new live-transcript line of speech: 2 or more words with letters and at least 1 s
//! of audio (`speech_rule`, the Mac's rule), so filler like "Thank you." from near-silent audio
//! neither resets the clock nor closes an open prompt. With no live transcription (no model), the
//! recorder's audio level stands in. The clock is wall time, compared on every poll, so a PC that
//! slept through the quiet asks as soon as it wakes.

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};

use super::live::{Accumulated, Listener};
use super::speech_rule::{self, Tally};
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

/// When the live transcript last gained a line of speech, and whether a recording's live
/// transcription is feeding it (one recording at a time).
static LAST_SPEECH_MS: AtomicI64 = AtomicI64::new(0);
static LIVE_TRACKING: AtomicBool = AtomicBool::new(false);
/// What the live transcript held at the last look, as counts (`speech_rule::Tally`).
static SEEN: Mutex<Tally> = Mutex::new(Tally::ZERO);

/// The counts of every line in `state` (never its text).
fn tally_of(state: &Accumulated) -> Tally {
    let mut tally = Tally::ZERO;
    for s in state.system.iter().chain(state.mic.iter()) {
        tally.add(s.start_cs, s.end_cs, &s.text);
    }
    tally
}

/// How many lines of speech `state` holds. A line counts only with 2 or more words with letters
/// and at least 1 s of audio (`speech_rule`), so filler like "Thank you." does not.
fn text_segments(state: &Accumulated) -> usize {
    tally_of(state).speech_lines
}

/// Whether `state` holds more lines of speech than `seen` (and how many it holds).
pub fn more_words(state: &Accumulated, seen: usize) -> (bool, usize) {
    let now = text_segments(state);
    (now > seen, now)
}

/// The check-in log (`target: "check-in"`): one line per event, counts and times only, never
/// transcript text. Same fields as the Mac's `CheckInLog`.
fn log_check_in(event: &str, reason: &str, last_speech_ms: i64, quiet_since_ms: Option<i64>, extra: &str) {
    let live = if !LIVE_TRACKING.load(Ordering::Relaxed) {
        "off"
    } else if crate::meeting_load::live_paused() {
        "paused"
    } else {
        "listening"
    };
    let line = speech_rule::check_in_line(
        event,
        reason,
        Some(last_speech_ms).filter(|ms| *ms > 0).map(clock).as_deref(),
        quiet_since_ms.map(clock).as_deref(),
        live,
        extra,
    );
    tracing::info!(target: "check-in", "{line}");
}

/// A local `HH:mm:ss` for the log.
fn clock(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|t| t.with_timezone(&chrono::Local).format("%H:%M:%S").to_string())
        .unwrap_or_else(|| "none".to_string())
}

/// A look at the live transcript at `now_ms`: a new line of speech moves the quiet clock's input
/// (`LAST_SPEECH_MS`), which `poll` reads (and which closes an open prompt). A line that is only
/// filler changes nothing and is logged as ignored. Returns whether speech arrived.
fn note_live_lines(state: &Accumulated, now_ms: i64) -> bool {
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    let (more, _) = more_words(state, seen.speech_lines);
    let now = tally_of(state);
    let arrival = now.since(&seen);
    *seen = now;
    drop(seen);
    if more {
        LAST_SPEECH_MS.store(now_ms, Ordering::Relaxed);
        log_check_in("speech-arrived", "live-line", now_ms, None, &arrival.fields());
    } else if arrival.filler_lines > 0 {
        log_check_in("filler-ignored", "live-line", LAST_SPEECH_MS.load(Ordering::Relaxed), None, &arrival.fields());
    }
    more
}

/// The live listener for a new recording: notes each new line of speech, then hands the state to
/// `inner` (the question watcher, when armed). `tracking` is whether live transcription will run.
pub fn speech_listener(inner: Option<Listener>, tracking: bool) -> Listener {
    LAST_SPEECH_MS.store(0, Ordering::Relaxed);
    *SEEN.lock().unwrap_or_else(|e| e.into_inner()) = Tally::ZERO;
    LIVE_TRACKING.store(tracking, Ordering::Relaxed);
    Listener(Arc::new(move |state: &Accumulated| {
        note_live_lines(state, chrono::Utc::now().timestamp_millis());
        if let Some(inner) = &inner {
            (inner.0)(state);
        }
    }))
}

/// When "This recording is still paused." was last asked; 0 for never. A time before the current
/// pause began is a previous pause's and is ignored.
static PAUSED_ASKED_MS: AtomicI64 = AtomicI64::new(0);

/// One poll of a paused recording: whether to ask now (an hour into the pause, then every hour).
/// Asking never ends the recording; there is no answer timer. The clock is wall time, so a PC that
/// slept through the hour asks on its first poll awake.
pub fn poll_paused(paused_since_ms: i64, now_ms: i64) -> bool {
    let asked = PAUSED_ASKED_MS.load(Ordering::Relaxed);
    let due = super::pause_clock::paused_check_due(paused_since_ms, Some(asked).filter(|at| *at > paused_since_ms), now_ms);
    if due {
        PAUSED_ASKED_MS.store(now_ms, Ordering::Relaxed);
    }
    due
}

fn timer() -> &'static Mutex<Option<Timer>> {
    static TIMER: std::sync::OnceLock<Mutex<Option<Timer>>> = std::sync::OnceLock::new();
    TIMER.get_or_init(|| Mutex::new(None))
}

/// One poll of a recording started by hand, or of a browser call (`meetings::service`).
/// `audio_active` is the recorder's level check, used only when live transcription isn't running
/// (no model, or paused by the load guard).
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
    } else if LIVE_TRACKING.load(Ordering::Relaxed) && !crate::meeting_load::live_paused() {
        Some(LAST_SPEECH_MS.load(Ordering::Relaxed)).filter(|ms| *ms > 0)
    } else {
        audio_active.then_some(now_ms)
    };
    if let Some(at) = spoke {
        if at > t.last_speech_ms && t.heard(at) == Action::Close {
            log_check_in("cancel", "speech-while-asked", t.last_speech_ms, Some(t.last_speech_ms), "");
            return Action::Close;
        }
    }
    let action = t.tick(now_ms);
    match action {
        Action::Ask => log_check_in("prompt-shown", "quiet", t.last_speech_ms, Some(t.last_speech_ms), ""),
        Action::Stop => log_check_in("stop", "quiet-timeout", t.last_speech_ms, Some(t.last_speech_ms), ""),
        Action::Nothing | Action::Close => {}
    }
    action
}

/// The recording ended: forget its timer.
pub fn reset() {
    *timer().lock().unwrap() = None;
}

/// "Keep recording" from the popup.
pub fn keep(now_ms: i64) {
    if let Some(t) = timer().lock().unwrap().as_mut() {
        t.keep(now_ms);
        log_check_in("keep", "quiet-answer", t.last_speech_ms, Some(t.last_speech_ms), "");
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
    let redesign = app.state::<crate::state::AppState>().settings.lock().unwrap().redesign_enabled();
    let (w, h) = crate::text_scale::size_for_look(SIZE, redesign);
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
    if redesign {
        crate::text_scale::place_corner_alert(app, &popup, (w, h), MARGIN)?;
        crate::text_scale::show_corner_alert(&popup)?;
        return Ok(());
    }
    if let Some(monitor) = popup.primary_monitor()? {
        let scale = monitor.scale_factor();
        let area = monitor.work_area();
        let (x, mut y) = crate::alert::alert_origin(
            ((w * scale).round() as u32, (h * scale).round() as u32),
            (area.position.x, area.position.y, area.size.width, area.size.height),
            (MARGIN * scale).round() as i32,
        );
        for (label, height) in [(crate::alert::ALERT, crate::alert::alert_height(app)), (crate::question_alert::QUESTION_ALERT, 210.0)] {
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
    // The tray and the in-meeting window follow at once, as after the Stop button (`commands::meetings`), not on the next tick.
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let stopped = meetings.stop_quiet().map(|_| ());
        crate::tray::refresh(&handle);
        stopped
    })
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
    fn a_paused_recording_asks_an_hour_in_and_every_hour_after_and_never_stops() {
        PAUSED_ASKED_MS.store(0, Ordering::Relaxed);
        assert!(!poll_paused(1_000, 1_000 + 59 * MIN));
        assert!(poll_paused(1_000, 1_000 + 60 * MIN));
        assert!(!poll_paused(1_000, 1_000 + 61 * MIN), "asked once for that hour");
        assert!(poll_paused(1_000, 1_000 + 120 * MIN));
        // A new pause forgets the last one's question.
        assert!(!poll_paused(1_000 + 200 * MIN, 1_000 + 201 * MIN));
        PAUSED_ASKED_MS.store(0, Ordering::Relaxed);
    }

    fn seg(start_cs: i64, end_cs: i64, text: &str) -> transcribe::TranscriptSegment {
        transcribe::TranscriptSegment { start_cs, end_cs, text: text.into() }
    }

    /// The statics are process-wide, so the tests that use them take turns.
    fn serial() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn a_new_line_of_speech_counts_and_blank_or_filler_ones_do_not() {
        let mut state = Accumulated::default();
        state.system.push(seg(0, 300, "  "));
        assert_eq!(more_words(&state, 0), (false, 0), "blank");
        state.mic.push(seg(0, 300, "hello"));
        assert_eq!(more_words(&state, 0), (false, 0), "one word is not speech");
        state.mic.push(seg(300, 340, "Thank you."));
        assert_eq!(more_words(&state, 0), (false, 0), "under a second is filler");
        state.mic.push(seg(400, 600, "let's begin the review"));
        assert_eq!(more_words(&state, 0), (true, 1));
        assert_eq!(more_words(&state, 1), (false, 1));
    }

    #[test]
    fn japanese_without_spaces_is_a_line_of_speech() {
        let mut state = Accumulated::default();
        state.system.push(seg(0, 300, "\u{4F1A}\u{8B70}\u{3092}\u{59CB}\u{3081}\u{307E}\u{3059}"));
        assert_eq!(more_words(&state, 0), (true, 1));
    }

    #[test]
    fn filler_does_not_reset_the_clock_or_close_a_prompt_but_speech_does() {
        let _turn = serial();
        LIVE_TRACKING.store(true, Ordering::Relaxed);
        LAST_SPEECH_MS.store(0, Ordering::Relaxed);
        *SEEN.lock().unwrap() = Tally::ZERO;
        reset();

        let mut state = Accumulated::default();
        assert_eq!(poll(9, 0, 5 * MIN, false, false), Action::Nothing);
        // "Thank you." at 14 minutes from near-silent audio: not speech, the clock keeps running.
        state.mic.push(seg(0, 40, "Thank you."));
        assert!(!note_live_lines(&state, 14 * MIN));
        assert_eq!(poll(9, 0, 15 * MIN, false, false), Action::Ask, "filler did not delay the prompt");
        // More filler while the prompt is open does not cancel it.
        state.mic.push(seg(50, 90, "Thanks."));
        assert!(!note_live_lines(&state, 16 * MIN));
        assert_eq!(poll(9, 0, 16 * MIN, false, false), Action::Nothing);
        assert_eq!(poll(9, 0, 17 * MIN, false, false), Action::Stop, "still asking, so the two minutes run out");

        // A real two-word line of 1 s or more resets the clock, and closes an open prompt.
        reset();
        LAST_SPEECH_MS.store(0, Ordering::Relaxed);
        *SEEN.lock().unwrap() = Tally::ZERO;
        let mut state = Accumulated::default();
        assert_eq!(poll(10, 0, 15 * MIN, false, false), Action::Ask);
        state.mic.push(seg(100, 250, "let's begin"));
        assert!(note_live_lines(&state, 16 * MIN));
        assert_eq!(poll(10, 0, 16 * MIN + 1, false, false), Action::Close);
        assert_eq!(poll(10, 0, 30 * MIN, false, false), Action::Nothing, "15 minutes from the speech");
        assert_eq!(poll(10, 0, 31 * MIN + 1, false, false), Action::Ask);
        reset();
        LIVE_TRACKING.store(false, Ordering::Relaxed);
        LAST_SPEECH_MS.store(0, Ordering::Relaxed);
    }

    #[test]
    fn a_listener_started_for_a_new_recording_forgets_the_last_ones_lines() {
        let _turn = serial();
        let mut state = Accumulated::default();
        state.mic.push(seg(0, 300, "let's begin"));
        let _ = speech_listener(None, true);
        assert!(note_live_lines(&state, 1_000));
        let _ = speech_listener(None, true);
        assert!(note_live_lines(&state, 2_000), "counted again for the new recording");
        LIVE_TRACKING.store(false, Ordering::Relaxed);
        LAST_SPEECH_MS.store(0, Ordering::Relaxed);
    }

    #[test]
    fn without_live_transcription_audible_audio_stands_in() {
        let _turn = serial();
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
