//! The detection loop and the end detector (M3.3 Task 6).
//!
//! One thread, one 5 s poll (FR-MTG-2 allows 30 s to notice a call, and the debounce wants
//! two polls). While nothing records it looks for a call and raises the alert; while a
//! meeting records it feeds core's end rules instead, so a call that ends stops the
//! recording without the user doing anything.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use api::meetings::end_rule::{self, EndReason, EndSignals, EndState, EndThresholds};
use api::meetings::heuristic::Debounce;
use tauri::AppHandle;

use super::controller::MeetingController;
use super::detect::{self, Candidate};
use crate::capture::{audio, proc_stats, uia::Uia};
use crate::settings::SettingsFile;

/// How often the loop looks. Two of these is 10 s, well inside FR-MTG-2's 30 s.
pub const POLL: Duration = Duration::from_secs(5);

/// A meeting Whimbrel is offering to record: a call it noticed, or a calendar event whose
/// lead window has opened (M3.3 Task 7).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DetectedMeeting {
    /// `detected-<app>-<epoch seconds>` from core's debounce, or the calendar event's id.
    pub id: String,
    /// "Zoom", "Google Meet", or the event's title.
    pub title: String,
    pub app_id: String,
    /// The process tree to record, or 0 for a calendar meeting with no app in sight.
    pub root_pid: u32,
    /// When the alert appeared, for its countdown.
    pub prompted_unix_ms: i64,
    /// "detected" or "calendar": which trigger the meeting row gets.
    pub source: String,
    /// A calendar meeting's invitees, which become the expected speaker count.
    pub attendees: Vec<String>,
    /// A calendar meeting's own start, so the row spans the event, not the moment Accept
    /// was pressed.
    pub start_unix_ms: Option<i64>,
}

/// What the alert window shows.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AlertView {
    pub meeting: Option<DetectedMeeting>,
    /// Whether the countdown runs (Settings → Meetings → start recording automatically).
    pub auto_start: bool,
    pub countdown_seconds: i64,
}

/// The Mac's countdown before an auto-start (`MeetingAlertPanelController`).
pub const AUTO_START_COUNTDOWN_SECONDS: i64 = 10;

/// The prompt waiting for an answer, shared with the alert window's commands, plus the
/// app detection can see right now — a calendar meeting records that app when there is
/// one, rather than everything the PC plays.
#[derive(Default)]
pub struct Pending {
    prompt: Mutex<Option<DetectedMeeting>>,
    candidate_pid: std::sync::atomic::AtomicU32,
    /// When some other app first held the microphone, 0 while none does. A calendar
    /// prompt's countdown runs only while this is set (`api::meetings::in_call`).
    mic_busy_since: std::sync::atomic::AtomicI64,
}

impl Pending {
    pub fn get(&self) -> Option<DetectedMeeting> {
        self.prompt.lock().unwrap().clone()
    }

    pub fn take(&self) -> Option<DetectedMeeting> {
        self.prompt.lock().unwrap().take()
    }

    /// The detection loop's latest sighting: the process tree of the app in a call, if any.
    pub fn note_candidate(&self, root_pid: Option<u32>) {
        self.candidate_pid.store(root_pid.unwrap_or(0), std::sync::atomic::Ordering::Relaxed);
    }

    pub fn candidate_pid(&self) -> Option<u32> {
        Some(self.candidate_pid.load(std::sync::atomic::Ordering::Relaxed)).filter(|pid| *pid != 0)
    }

    /// The detection loop's latest look at the microphone: whether another app holds it.
    /// The first poll that sees it busy stamps the time; a poll that sees it free clears it,
    /// so a call that ends and another that begins restart the countdown.
    pub fn note_mic_in_use(&self, in_use: bool, now_unix_ms: i64) {
        use std::sync::atomic::Ordering::Relaxed;
        if !in_use {
            self.mic_busy_since.store(0, Relaxed);
        } else if self.mic_busy_since.load(Relaxed) == 0 {
            self.mic_busy_since.store(now_unix_ms, Relaxed);
        }
    }

    fn mic_busy_since(&self) -> Option<i64> {
        Some(self.mic_busy_since.load(std::sync::atomic::Ordering::Relaxed)).filter(|since| *since != 0)
    }

    fn set(&self, meeting: Option<DetectedMeeting>) {
        *self.prompt.lock().unwrap() = meeting;
    }

    /// Offers a calendar meeting whose lead window has opened (M3.3 Task 7).
    pub fn set_calendar(&self, meeting: api::connectors::calendar::QualifyingMeeting, now_unix_ms: i64) {
        self.set(Some(DetectedMeeting {
            id: meeting.id,
            title: meeting.title,
            app_id: String::new(),
            root_pid: 0,
            prompted_unix_ms: now_unix_ms,
            source: "calendar".into(),
            attendees: meeting.attendees,
            start_unix_ms: Some(meeting.start_unix_ms),
        }))
    }


}

pub struct Detection {
    controller: Arc<MeetingController>,
    settings: Arc<Mutex<SettingsFile>>,
    pub pending: Arc<Pending>,
    app: AppHandle,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Starts the loop. It runs for the life of the process.
pub fn start(app: AppHandle, controller: Arc<MeetingController>, settings: Arc<Mutex<SettingsFile>>, pending: Arc<Pending>) {
    let detection = Detection { controller, settings, pending, app };
    let spawned = std::thread::Builder::new().name("whimbrel-meeting-detect".into()).spawn(move || detection.run());
    if let Err(e) = spawned {
        tracing::error!(error = %e, "could not start meeting detection");
    }
}

impl Detection {
    fn run(self) {
        // UIA lives on this thread: it reads a browser's address bar when one holds the
        // microphone. A failure here only means browser calls aren't detected.
        let uia = Uia::new().map_err(|e| tracing::warn!(error = %e, "no UIA for browser detection")).ok();
        let mut debounce = Debounce::default();
        let mut end_state = EndState::default();
        let mut recording_meeting: Option<(i64, Candidate)> = None;
        loop {
            std::thread::sleep(POLL);
            let processes = proc_stats::list_processes();
            let sessions = match audio::audio_sessions() {
                Ok(sessions) => detect::Sessions::from(&sessions),
                Err(e) => {
                    tracing::warn!(error = %e, "could not read audio sessions");
                    continue;
                }
            };
            let found = detect::candidate(&processes, &sessions, |pid| {
                let tree = proc_stats::process_tree(pid, &processes);
                uia.as_ref().and_then(|uia| detect::browser_address(uia, pid, &tree))
            });
            self.pending.note_candidate(found.as_ref().filter(|c| c.audio_live).map(|c| c.root_pid));
            // Is this PC in a call, or only near one? Another app holding the microphone is
            // how a calendar prompt tells (Whimbrel's own tree is excluded).
            let own_tree = proc_stats::process_tree(std::process::id(), &processes);
            self.pending.note_mic_in_use(sessions.other_captures(&own_tree), now_ms());

            match self.controller.status().meeting_id {
                // Recording: the same signals now answer "is this call over?".
                Some(meeting_id) => {
                    let watched = recording_meeting.get_or_insert_with(|| {
                        (meeting_id, found.clone().unwrap_or(Candidate {
                            root_pid: 0,
                            exe: String::new(),
                            title: String::new(),
                            via_browser: false,
                            audio_live: false,
                        }))
                    });
                    if watched.0 != meeting_id {
                        *watched = (meeting_id, found.clone().unwrap_or_else(|| watched.1.clone()));
                        end_state = EndState::default();
                    }
                    self.check_end(meeting_id, &watched.1.clone(), &processes, &sessions, &mut end_state);
                    debounce = Debounce::default();
                }
                None => {
                    recording_meeting = None;
                    end_state = EndState::default();
                    self.offer(&mut debounce, found);
                }
            }
        }
    }

    /// Feeds the debounce and raises the alert when it says a call is real.
    fn offer(&self, debounce: &mut Debounce, found: Option<Candidate>) {
        if !self.settings.lock().unwrap().heuristic_detection_enabled() {
            *debounce = Debounce::default();
            return;
        }
        let root_pid = found.as_ref().map(|c| c.root_pid).unwrap_or(0);
        let Some(meeting) = debounce.observe(now_ms(), detect::observation(found)) else {
            return;
        };
        if self.pending.get().is_some() {
            return; // one alert at a time, as on the Mac
        }
        let detected = DetectedMeeting {
            id: meeting.id,
            title: meeting.title,
            app_id: meeting.app_bundle_id.unwrap_or_default(),
            root_pid,
            prompted_unix_ms: now_ms(),
            source: "detected".into(),
            attendees: Vec::new(),
            start_unix_ms: None,
        };
        tracing::info!(id = %detected.id, title = %detected.title, pid = detected.root_pid, "meeting detected");
        self.pending.set(Some(detected));
        crate::alert::show(&self.app);
    }

    /// Builds this poll's end signals and applies core's rules.
    fn check_end(
        &self,
        meeting_id: i64,
        watched: &Candidate,
        processes: &[proc_stats::ProcInfo],
        sessions: &detect::Sessions,
        state: &mut EndState,
    ) {
        let Some(started) = self.controller.status().started_unix_ms else { return };
        // A manual recording watches no app: it ends when the user says so.
        let watching = watched.root_pid != 0;
        let tree = watching.then(|| proc_stats::process_tree(watched.root_pid, processes)).unwrap_or_default();
        let app_running = !watching || processes.iter().any(|p| p.pid == watched.root_pid);
        let signals = EndSignals {
            now_unix_ms: now_ms(),
            recording_started_unix_ms: started,
            app_running,
            on_meeting_tab: None,
            scheduled_end_unix_ms: None,
            audio_active: self.controller.audio_active(now_ms()).unwrap_or(false),
            capture_session_active: (watching && app_running).then(|| sessions.tree_captures(&tree)),
        };
        if let Some(reason) = end_rule::evaluate(&signals, &EndThresholds::default(), state) {
            tracing::info!(meeting_id, ?reason, "the meeting ended on its own");
            self.end(meeting_id, reason);
        }
    }

    fn end(&self, meeting_id: i64, reason: EndReason) {
        if let Err(e) = self.controller.stop() {
            tracing::error!(meeting_id, ?reason, error = %e, "could not stop the ended meeting");
        }
        crate::tray::refresh(&self.app);
    }
}

/// What the alert shows right now: the prompt, and how long its countdown has left. A
/// calendar meeting counts down only while another app holds the microphone (this PC is in
/// the call); until then it shows the plain "Record it?" prompt, as on the Mac (#203).
pub fn alert_view(pending: &Pending, settings: &SettingsFile, now_unix_ms: i64) -> AlertView {
    let meeting = pending.get();
    // `Some(None)`: a calendar prompt still waiting for the call to begin.
    let countdown_start = meeting.as_ref().map(|m| {
        if m.source == "calendar" {
            api::meetings::in_call::countdown_start(m.prompted_unix_ms, pending.mic_busy_since())
        } else {
            Some(m.prompted_unix_ms)
        }
    });
    let waiting_for_call = matches!(countdown_start, Some(None));
    let auto_start = settings.auto_start_enabled() && !waiting_for_call;
    let countdown_seconds = match countdown_start.flatten() {
        Some(start) => (AUTO_START_COUNTDOWN_SECONDS - (now_unix_ms - start) / 1_000).max(0),
        None => AUTO_START_COUNTDOWN_SECONDS,
    };
    AlertView { meeting, auto_start, countdown_seconds }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detected(prompted_unix_ms: i64) -> DetectedMeeting {
        DetectedMeeting {
            id: "detected-zoom.exe-1".into(),
            title: "Zoom".into(),
            app_id: "zoom.exe".into(),
            root_pid: 10,
            prompted_unix_ms,
            source: "detected".into(),
            attendees: Vec::new(),
            start_unix_ms: None,
        }
    }

    fn calendar_meeting() -> api::connectors::calendar::QualifyingMeeting {
        api::connectors::calendar::QualifyingMeeting {
            id: "event-1".into(),
            title: "Standup".into(),
            start_unix_ms: 5_000,
            end_unix_ms: 9_000,
            attendees: vec!["me@x".into(), "you@x".into()],
            conference_link: None,
            source: api::connectors::calendar::MeetingSource::GoogleCalendar,
            app_bundle_id: None,
            ..Default::default()
        }
    }

    #[test]
    fn a_calendar_prompt_carries_its_event_and_no_process() {
        let pending = Pending::default();
        pending.set_calendar(calendar_meeting(), 1_000);
        let offered = pending.get().unwrap();
        assert_eq!((offered.source.as_str(), offered.title.as_str()), ("calendar", "Standup"));
        assert_eq!(offered.start_unix_ms, Some(5_000), "the row spans the event, not the moment Accept was pressed");
        assert_eq!(offered.attendees.len(), 2, "the invitees become the expected speaker count");
        assert_eq!(pending.candidate_pid(), None, "no app in a call right now");
        pending.note_candidate(Some(42));
        assert_eq!(pending.candidate_pid(), Some(42), "a calendar meeting records that app");
    }

    #[test]
    fn the_pending_prompt_is_answered_once() {
        let pending = Pending::default();
        pending.set(Some(detected(0)));
        assert!(pending.get().is_some());
        assert!(pending.take().is_some());
        assert_eq!(pending.take(), None, "a second Accept has nothing to accept");
    }

    #[test]
    fn the_countdown_runs_down_from_ten_and_stops_at_zero() {
        let dir = std::env::temp_dir().join(format!("whimbrel-alert-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut settings = SettingsFile::load(&dir.join("settings.json"));
        settings.set_meeting_settings(true, true, 1.0);
        let pending = Pending::default();
        pending.set(Some(detected(10_000)));

        assert_eq!(alert_view(&pending, &settings, 10_000).countdown_seconds, 10);
        assert_eq!(alert_view(&pending, &settings, 14_000).countdown_seconds, 6);
        assert_eq!(alert_view(&pending, &settings, 30_000).countdown_seconds, 0);
        assert!(alert_view(&pending, &settings, 10_000).auto_start);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn auto_start_settings(name: &str) -> (SettingsFile, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("whimbrel-alert-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut settings = SettingsFile::load(&dir.join("settings.json"));
        settings.set_meeting_settings(true, true, 1.0);
        (settings, dir)
    }

    #[test]
    fn a_calendar_prompt_waits_until_another_app_has_the_microphone() {
        let (settings, dir) = auto_start_settings("cal-wait");
        let pending = Pending::default();
        pending.set_calendar(calendar_meeting(), 10_000);

        let waiting = alert_view(&pending, &settings, 40_000);
        assert!(!waiting.auto_start, "this PC is not in a call: the prompt waits");
        assert_eq!(waiting.countdown_seconds, AUTO_START_COUNTDOWN_SECONDS);

        pending.note_mic_in_use(true, 41_000);
        assert!(alert_view(&pending, &settings, 41_000).auto_start);
        assert_eq!(alert_view(&pending, &settings, 41_000).countdown_seconds, 10);
        assert_eq!(alert_view(&pending, &settings, 45_000).countdown_seconds, 6);

        pending.note_mic_in_use(true, 43_000);
        assert_eq!(alert_view(&pending, &settings, 45_000).countdown_seconds, 6, "still busy: the countdown does not restart");

        pending.note_mic_in_use(false, 46_000);
        assert!(!alert_view(&pending, &settings, 46_000).auto_start, "the call ended: waiting again");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_call_already_running_when_the_calendar_prompt_appears_counts_from_the_prompt() {
        let (settings, dir) = auto_start_settings("cal-running");
        let pending = Pending::default();
        pending.note_mic_in_use(true, 2_000);
        pending.set_calendar(calendar_meeting(), 10_000);

        assert_eq!(alert_view(&pending, &settings, 10_000).countdown_seconds, 10, "the user gets the full countdown");
        assert_eq!(alert_view(&pending, &settings, 14_000).countdown_seconds, 6);
        assert!(alert_view(&pending, &settings, 14_000).auto_start);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_detected_call_counts_down_without_the_microphone_gate() {
        let (settings, dir) = auto_start_settings("detected");
        let pending = Pending::default();
        pending.set(Some(detected(10_000)));

        assert!(alert_view(&pending, &settings, 14_000).auto_start, "detection already means a call app is live");
        assert_eq!(alert_view(&pending, &settings, 14_000).countdown_seconds, 6);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn with_no_prompt_there_is_nothing_to_show() {
        let dir = std::env::temp_dir().join(format!("whimbrel-alert-none-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let settings = SettingsFile::load(&dir.join("settings.json"));
        let view = alert_view(&Pending::default(), &settings, 0);
        assert_eq!(view.meeting, None);
        assert!(!view.auto_start, "auto-start is off by default, as on the Mac");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
