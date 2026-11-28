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
//!
//! The same window also asks "Still recording?" (`Hourly`, the Mac's `RecordingCheckIn`): every
//! recording, detected or started by hand, asks after the hours chosen in Settings → Meetings (1 by
//! default, Off allowed), then an hour after each "Keep recording". "Stop now" stops it as it is;
//! five minutes with no answer stops it and trims it to the last words, as "Still in a meeting?"
//! does. When both are due, "Still in a meeting?" asks first and the hourly one waits.
//!
//! While "Still recording?" is due or up, and after a "Keep recording" until the next one, it holds
//! off core's 4 h length cap, as on the Mac: a recording someone just kept is not cut off. The
//! held cap is 6 h instead (`end_thresholds`), the Mac's own hard limit, which nothing holds off.

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

use api::meetings::end_rule::EndThresholds;

use super::live::{Accumulated, Listener};
use super::speech_rule::{self, Tally};
use crate::l10n::tr;

/// No words this long → ask.
pub const QUIET_MS: i64 = 15 * 60_000;
/// No answer this long → stop.
pub const ANSWER_MS: i64 = 2 * 60_000;

/// "Still recording?": no answer this long → stop (and trim).
pub const HOURLY_COUNTDOWN_MS: i64 = 5 * 60_000;
/// "Still recording?": asks again this long after "Keep recording".
pub const HOURLY_REASK_MS: i64 = 60 * 60_000;
/// The length cap while "Still recording?" holds core's 4 h one off: the Mac's 6 h, never held.
pub const HELD_CAP_MS: i64 = 6 * 60 * 60_000;

/// Core's end rules for this poll: its 4 h length cap, or 6 h while "Still recording?" holds it
/// off (`hourly_holds_cap`). Every other rule is core's own.
pub fn end_thresholds(cap_held: bool) -> EndThresholds {
    let default = EndThresholds::default();
    if cap_held {
        EndThresholds { max_recording_duration_ms: HELD_CAP_MS.max(default.max_recording_duration_ms), ..default }
    } else {
        default
    }
}

/// The popup's label (also listed in `capabilities/default.json`).
pub const STILL_HERE: &str = "still-here";
const SIZE: (f64, f64) = (380.0, 170.0);
/// "Still recording?" has one more line, its countdown.
const HOURLY_SIZE: (f64, f64) = (380.0, 196.0);
const MARGIN: f64 = 12.0;

/// Which question the window asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum PromptKind {
    /// "Still in a meeting?": 15 quiet minutes.
    Quiet,
    /// "Still recording?": the hourly check.
    Hourly,
    /// "This recording is still paused."
    Paused,
}

/// What the window shows (`still_here_prompt`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StillHerePrompt {
    pub kind: PromptKind,
    /// "Still recording?" only: when it stops with no answer, unix ms.
    pub deadline_unix_ms: Option<i64>,
}

/// The first "Still recording?" for the setting's hours; None for 0 (Off).
pub fn check_in_after_ms(hours: u8) -> Option<i64> {
    (hours > 0).then(|| i64::from(hours) * 60 * 60_000)
}

/// What an hourly poll should do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HourlyAction {
    Nothing,
    /// Show "Still recording?"; it stops at this time (unix ms) with no answer.
    Ask { deadline_ms: i64 },
    /// No answer: stop and trim.
    Stop,
}

/// "Still recording?" for one recording (the Mac's `RecordingCheckIn`). Pure: times in unix ms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hourly {
    pub meeting_id: i64,
    next_prompt_ms: Option<i64>,
    shown_ms: Option<i64>,
    /// "Keep recording" was chosen at least once.
    kept: bool,
}

impl Hourly {
    /// `first_after_ms` None is Off: it never asks.
    pub fn new(meeting_id: i64, started_ms: i64, first_after_ms: Option<i64>) -> Self {
        Hourly { meeting_id, next_prompt_ms: first_after_ms.map(|after| started_ms + after), shown_ms: None, kept: false }
    }

    pub fn asking(&self) -> bool {
        self.shown_ms.is_some()
    }

    pub fn deadline_ms(&self) -> Option<i64> {
        self.shown_ms.map(|shown| shown + HOURLY_COUNTDOWN_MS)
    }

    pub fn poll(&mut self, now_ms: i64) -> HourlyAction {
        if let Some(shown) = self.shown_ms {
            if now_ms < shown + HOURLY_COUNTDOWN_MS {
                return HourlyAction::Nothing;
            }
            // Stop fires once; nothing re-arms.
            self.shown_ms = None;
            self.next_prompt_ms = None;
            return HourlyAction::Stop;
        }
        match self.next_prompt_ms {
            Some(next) if now_ms >= next => {
                self.shown_ms = Some(now_ms);
                self.next_prompt_ms = None;
                HourlyAction::Ask { deadline_ms: now_ms + HOURLY_COUNTDOWN_MS }
            }
            _ => HourlyAction::Nothing,
        }
    }

    /// "Keep recording": asks again an hour from now.
    pub fn keep(&mut self, now_ms: i64) {
        self.shown_ms = None;
        self.kept = true;
        self.next_prompt_ms = Some(now_ms + HOURLY_REASK_MS);
    }

    /// Whether core's 4 h cap is held off now (the Mac's `suppressesMaxDuration`): while a question
    /// is up (its own countdown decides), once one is due but not yet polled, and after "Keep
    /// recording" until the next question. The Mac bounds the last by 65 minutes after Keep; here
    /// it lasts while that next question is still to come, since a pause moves it later.
    pub fn holds_cap(&self, now_ms: i64) -> bool {
        if self.shown_ms.is_some() {
            return true;
        }
        match self.next_prompt_ms {
            Some(next) => self.kept || now_ms >= next,
            None => false,
        }
    }

    /// A paused poll. "This recording is still paused." covers a pause, so the hourly check is held:
    /// a question up is put away (true: close the window) and the next one comes an hour after the
    /// pause ends. Off stays off.
    pub fn hold(&mut self, now_ms: i64) -> bool {
        let was_asking = self.shown_ms.take().is_some();
        if was_asking || self.next_prompt_ms.is_some() {
            self.next_prompt_ms = Some(self.next_prompt_ms.unwrap_or(0).max(now_ms + HOURLY_REASK_MS));
        }
        was_asking
    }
}

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

/// Whether "Still in a meeting?" is asking now (the hourly check waits while it does).
pub fn quiet_asking() -> bool {
    timer().lock().unwrap().as_ref().is_some_and(Timer::asking)
}

fn hourly() -> &'static Mutex<Option<Hourly>> {
    static HOURLY: Mutex<Option<Hourly>> = Mutex::new(None);
    &HOURLY
}

/// Whether "Still recording?" is asking now ("Still in a meeting?" is not polled while it does).
pub fn hourly_asking() -> bool {
    hourly().lock().unwrap().as_ref().is_some_and(Hourly::asking)
}

/// One hourly poll of a recording that is not paused. `first_after_ms` (the setting, None for Off)
/// is read once, at the recording's first poll, so a change applies from the next recording.
pub fn poll_hourly(meeting_id: i64, started_ms: i64, now_ms: i64, first_after_ms: impl FnOnce() -> Option<i64>) -> HourlyAction {
    let mut slot = hourly().lock().unwrap();
    let h = match slot.as_mut() {
        Some(h) if h.meeting_id == meeting_id => h,
        _ => slot.insert(Hourly::new(meeting_id, started_ms, first_after_ms())),
    };
    let action = h.poll(now_ms);
    match action {
        HourlyAction::Ask { .. } => log_check_in("prompt-shown", "hourly", LAST_SPEECH_MS.load(Ordering::Relaxed), None, ""),
        HourlyAction::Stop => log_check_in("stop", "hourly-timeout", LAST_SPEECH_MS.load(Ordering::Relaxed), None, ""),
        HourlyAction::Nothing => {}
    }
    action
}

/// Whether "Still recording?" holds off the 4 h cap for this recording now (`Hourly::holds_cap`).
/// Another recording's check, left in the slot, holds nothing.
pub fn hourly_holds_cap(meeting_id: i64, now_ms: i64) -> bool {
    hourly().lock().unwrap().as_ref().is_some_and(|h| h.meeting_id == meeting_id && h.holds_cap(now_ms))
}

/// A paused poll holds the hourly check (`Hourly::hold`). True when its question was up and the
/// window should close.
pub fn hold_hourly(now_ms: i64) -> bool {
    hourly().lock().unwrap().as_mut().is_some_and(|h| h.hold(now_ms))
}

/// The recording ended: forget its timers.
pub fn reset() {
    *timer().lock().unwrap() = None;
    *hourly().lock().unwrap() = None;
}

/// "Keep recording" from the popup: answers whichever question is up.
pub fn keep(now_ms: i64) {
    if *SHOWING.lock().unwrap() == Some(PromptKind::Hourly) {
        if let Some(h) = hourly().lock().unwrap().as_mut() {
            h.keep(now_ms);
            log_check_in("keep", "hourly-answer", LAST_SPEECH_MS.load(Ordering::Relaxed), None, "");
        }
        return;
    }
    keep_quiet(now_ms);
}

/// Starts the quiet clock over (a resume, or "Keep recording" on "Still in a meeting?"), whatever
/// the window shows.
pub fn keep_quiet(now_ms: i64) {
    if let Some(t) = timer().lock().unwrap().as_mut() {
        t.keep(now_ms);
        log_check_in("keep", "quiet-answer", t.last_speech_ms, Some(t.last_speech_ms), "");
    }
}

/// The question the window is up for; None while it is closed.
static SHOWING: Mutex<Option<PromptKind>> = Mutex::new(None);

/// Opens the window for `kind`. Already open for it: nothing changes. Open for another question
/// (the hourly one after a pause, say): it is drawn again for this one.
pub fn show(app: &AppHandle, kind: PromptKind) {
    let open_for = *SHOWING.lock().unwrap();
    if let Some(window) = app.get_webview_window(STILL_HERE) {
        if open_for == Some(kind) {
            return;
        }
        let _ = window.destroy();
    }
    *SHOWING.lock().unwrap() = Some(kind);
    if let Err(e) = open(app, kind) {
        tracing::error!(error = %e, "could not open the still-in-a-meeting prompt");
    }
}

pub fn hide(app: &AppHandle) {
    *SHOWING.lock().unwrap() = None;
    if let Some(window) = app.get_webview_window(STILL_HERE) {
        let _ = window.destroy();
    }
}

/// What the window shows now; None when no question is up.
pub fn prompt() -> Option<StillHerePrompt> {
    let kind = (*SHOWING.lock().unwrap())?;
    let deadline_unix_ms = match kind {
        PromptKind::Hourly => hourly().lock().unwrap().as_ref().and_then(Hourly::deadline_ms),
        PromptKind::Quiet | PromptKind::Paused => None,
    };
    Some(StillHerePrompt { kind, deadline_unix_ms })
}

fn open(app: &AppHandle, kind: PromptKind) -> tauri::Result<()> {
    let (w, h) = crate::text_scale::scaled_size(if kind == PromptKind::Hourly { HOURLY_SIZE } else { SIZE });
    let title = match kind {
        PromptKind::Quiet => tr("Still in a meeting?", &[]),
        PromptKind::Hourly => tr("Still recording?", &[]),
        PromptKind::Paused => tr("This recording is still paused.", &[]),
    };
    let popup = WebviewWindowBuilder::new(app, STILL_HERE, WebviewUrl::App("index.html".into()))
        .title(title)
        .inner_size(w, h)
        .decorations(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .visible(false)
        .build()?;
    crate::text_scale::place_corner_alert(app, &popup, (w, h), MARGIN)?;
    crate::text_scale::show_corner_alert(&popup)?;
    Ok(())
}

/// What the window asks, for its view.
#[tauri::command]
pub fn still_here_prompt() -> Option<StillHerePrompt> {
    prompt()
}

/// "Keep recording".
#[tauri::command]
pub fn still_here_keep(app: AppHandle) {
    keep(chrono::Utc::now().timestamp_millis());
    hide(&app);
}

/// "Stop" on "Still in a meeting?": stops the recording; it is trimmed to the last words once
/// transcribed. "Stop now" on "Still recording?": stops it as it is (only no answer trims).
#[tauri::command]
pub async fn still_here_stop(app: AppHandle) -> Result<(), crate::error::CommandError> {
    let trim = *SHOWING.lock().unwrap() != Some(PromptKind::Hourly);
    if !trim {
        log_check_in("stop", "hourly-answer", LAST_SPEECH_MS.load(Ordering::Relaxed), None, "");
    }
    hide(&app);
    let state = app.state::<crate::state::AppState>();
    let meetings = state.meetings()?.clone();
    // The tray and the in-meeting window follow at once, as after the Stop button (`commands::meetings`), not on the next tick.
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let stopped = if trim { meetings.stop_quiet() } else { meetings.stop() }.map(|_| ());
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

    const HOUR: i64 = 60 * MIN;

    #[test]
    fn still_recording_asks_at_an_hour_and_stops_after_five_minutes_once() {
        let mut h = Hourly::new(1, 0, check_in_after_ms(1));
        assert_eq!(h.poll(59 * MIN), HourlyAction::Nothing);
        assert_eq!(h.poll(HOUR), HourlyAction::Ask { deadline_ms: HOUR + 5 * MIN });
        assert_eq!(h.deadline_ms(), Some(HOUR + 5 * MIN));
        assert_eq!(h.poll(HOUR + 4 * MIN), HourlyAction::Nothing, "asked once");
        assert_eq!(h.poll(HOUR + 5 * MIN), HourlyAction::Stop);
        assert_eq!(h.poll(3 * HOUR), HourlyAction::Nothing, "stop fires once and nothing re-arms");
    }

    #[test]
    fn keep_recording_asks_again_an_hour_later() {
        let mut h = Hourly::new(1, 0, check_in_after_ms(1));
        assert!(matches!(h.poll(HOUR), HourlyAction::Ask { .. }));
        h.keep(HOUR + 2 * MIN);
        assert!(!h.asking());
        assert_eq!(h.poll(2 * HOUR + MIN), HourlyAction::Nothing);
        assert!(matches!(h.poll(2 * HOUR + 2 * MIN), HourlyAction::Ask { .. }));
    }

    #[test]
    fn the_hours_setting_sets_the_first_check_and_off_never_asks() {
        assert_eq!(check_in_after_ms(0), None);
        let mut h = Hourly::new(1, 0, check_in_after_ms(3));
        assert_eq!(h.poll(3 * HOUR - 1), HourlyAction::Nothing);
        assert!(matches!(h.poll(3 * HOUR), HourlyAction::Ask { .. }));
        let mut off = Hourly::new(1, 0, check_in_after_ms(0));
        assert_eq!(off.poll(10 * HOUR), HourlyAction::Nothing);
        assert!(!off.hold(10 * HOUR));
        assert_eq!(off.poll(20 * HOUR), HourlyAction::Nothing, "a pause does not turn Off on");
    }

    #[test]
    fn a_pc_that_slept_past_the_hour_asks_on_its_first_poll_awake() {
        let mut h = Hourly::new(1, 0, check_in_after_ms(1));
        assert!(matches!(h.poll(5 * HOUR), HourlyAction::Ask { .. }));
    }

    #[test]
    fn a_pause_holds_still_recording_and_puts_its_question_away() {
        let mut h = Hourly::new(1, 0, check_in_after_ms(1));
        assert!(matches!(h.poll(HOUR), HourlyAction::Ask { .. }));
        // Paused while asking: the question goes, nothing stops at the old deadline.
        assert!(h.hold(HOUR + MIN));
        assert!(!h.hold(HOUR + 30 * MIN), "already put away");
        // Resumed at 1:30: asks an hour after the last paused poll, not at once.
        assert_eq!(h.poll(HOUR + 31 * MIN), HourlyAction::Nothing);
        assert_eq!(h.poll(2 * HOUR + 29 * MIN), HourlyAction::Nothing);
        assert!(matches!(h.poll(2 * HOUR + 30 * MIN), HourlyAction::Ask { .. }));
        // A short pause well before the hour keeps the original time.
        let mut early = Hourly::new(2, 0, check_in_after_ms(2));
        assert!(!early.hold(10 * MIN));
        assert!(matches!(early.poll(2 * HOUR), HourlyAction::Ask { .. }));
    }

    #[test]
    fn the_window_answers_whichever_question_is_up() {
        let _turn = serial();
        reset();
        *SHOWING.lock().unwrap() = None;
        assert_eq!(prompt(), None);
        // Still recording? is up: Keep re-arms the hourly check and leaves the quiet clock alone.
        assert!(matches!(poll_hourly(4, 0, HOUR, || check_in_after_ms(1)), HourlyAction::Ask { .. }));
        assert!(hourly_asking());
        *SHOWING.lock().unwrap() = Some(PromptKind::Hourly);
        assert_eq!(prompt(), Some(StillHerePrompt { kind: PromptKind::Hourly, deadline_unix_ms: Some(HOUR + 5 * MIN) }));
        // A resume starts only the quiet clock over: the hourly question is still up.
        keep_quiet(HOUR);
        assert!(hourly_asking());
        // Closed by something else (no SHOWING): the question still counts down, so the poll
        // shows it again (`service`), and nothing in here forgets it.
        *SHOWING.lock().unwrap() = None;
        assert!(hourly_asking());
        *SHOWING.lock().unwrap() = Some(PromptKind::Hourly);
        keep(HOUR + MIN);
        assert!(!hourly_asking());
        assert_eq!(poll_hourly(4, 0, 2 * HOUR, || unreachable!("read once per recording")), HourlyAction::Nothing);
        assert!(matches!(poll_hourly(4, 0, 2 * HOUR + MIN, || None), HourlyAction::Ask { .. }));
        // A new recording reads the setting again.
        reset();
        assert_eq!(poll_hourly(5, 0, 10 * HOUR, || None), HourlyAction::Nothing, "Off");
        *SHOWING.lock().unwrap() = None;
        reset();
    }

    #[test]
    fn the_cap_is_held_only_after_keep_recording() {
        // The Mac's testMaxDurationSuppressedOnlyAfterKeepRecording (2 h setting).
        let mut h = Hourly::new(1, 0, check_in_after_ms(2));
        assert!(!h.holds_cap(2 * HOUR - 1));
        assert!(matches!(h.poll(2 * HOUR), HourlyAction::Ask { .. }));
        h.keep(2 * HOUR + MIN);
        assert!(matches!(h.poll(3 * HOUR + MIN), HourlyAction::Ask { .. }));
        h.keep(3 * HOUR + MIN);
        assert!(h.holds_cap(4 * HOUR), "kept at 3:01: the 4 h cap waits");
        // From 4:01 the next question is due, then up: still held until it stops.
        assert!(h.holds_cap(4 * HOUR + 7 * MIN));
        assert!(matches!(h.poll(4 * HOUR + 7 * MIN), HourlyAction::Ask { .. }));
        assert!(h.holds_cap(4 * HOUR + 11 * MIN));
        assert_eq!(h.poll(4 * HOUR + 12 * MIN), HourlyAction::Stop);
        assert!(!h.holds_cap(4 * HOUR + 12 * MIN));
    }

    #[test]
    fn the_cap_is_held_while_the_question_is_up_until_it_stops() {
        let mut h = Hourly::new(1, 0, check_in_after_ms(2));
        assert!(!h.holds_cap(2 * HOUR - MIN));
        assert!(matches!(h.poll(2 * HOUR), HourlyAction::Ask { .. }));
        assert!(h.holds_cap(2 * HOUR + 4 * MIN));
        assert_eq!(h.poll(2 * HOUR + 5 * MIN), HourlyAction::Stop);
        assert!(!h.holds_cap(2 * HOUR + 5 * MIN));
    }

    #[test]
    fn a_question_due_at_four_hours_wins_over_the_cap_before_its_poll() {
        // Setting at 4 h: core's 4 h cap must not win the race with the question.
        let h = Hourly::new(1, 0, check_in_after_ms(4));
        assert!(!h.holds_cap(4 * HOUR - 1));
        assert!(h.holds_cap(4 * HOUR));
        // Off holds nothing, ever.
        let mut off = Hourly::new(1, 0, check_in_after_ms(0));
        assert!(!off.holds_cap(4 * HOUR));
        off.hold(4 * HOUR);
        assert!(!off.holds_cap(5 * HOUR));
    }

    #[test]
    fn a_pause_after_keep_does_not_let_the_cap_stop_the_recording_unasked() {
        // Kept at 3:01, paused 3:30 to 3:40: the next question moves to 4:40 (`hold`). The Mac's
        // 65 minutes after Keep would end at 4:06 with nothing asked; here the hold lasts to it.
        let mut h = Hourly::new(1, 0, check_in_after_ms(3));
        assert!(matches!(h.poll(3 * HOUR), HourlyAction::Ask { .. }));
        h.keep(3 * HOUR + MIN);
        for paused in (30..=40).step_by(5) {
            h.hold(3 * HOUR + paused * MIN);
        }
        assert!(h.holds_cap(4 * HOUR + 6 * MIN));
        assert_eq!(h.poll(4 * HOUR + 39 * MIN), HourlyAction::Nothing);
        assert!(matches!(h.poll(4 * HOUR + 40 * MIN), HourlyAction::Ask { .. }));
        assert!(h.holds_cap(4 * HOUR + 44 * MIN));
        assert_eq!(h.poll(4 * HOUR + 45 * MIN), HourlyAction::Stop);
        assert!(!h.holds_cap(4 * HOUR + 45 * MIN));
    }

    #[test]
    fn a_held_cap_is_six_hours_and_only_for_the_recording_that_was_kept() {
        let _turn = serial();
        reset();
        let held = end_thresholds(true);
        assert_eq!(held.max_recording_duration_ms, 6 * HOUR);
        assert_eq!(end_thresholds(false), EndThresholds::default());
        let default_cap = EndThresholds::default().max_recording_duration_ms;
        assert_eq!(EndThresholds { max_recording_duration_ms: default_cap, ..held }, EndThresholds::default(), "only the length cap changes");
        // The slot holds for its own recording, not a new one.
        assert!(!hourly_holds_cap(4, 4 * HOUR), "nothing polled yet");
        assert!(matches!(poll_hourly(4, 0, 4 * HOUR, || check_in_after_ms(4)), HourlyAction::Ask { .. }));
        assert!(hourly_holds_cap(4, 4 * HOUR + MIN));
        assert!(!hourly_holds_cap(5, 4 * HOUR + MIN), "another recording's question");
        reset();
        assert!(!hourly_holds_cap(4, 4 * HOUR + MIN));
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
