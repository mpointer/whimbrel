//! The meeting controller (M3.3 Task 3): the one place that starts and stops meeting
//! recordings and writes their rows, ported from the Mac's `AppState` start/end
//! orchestration.
//!
//! [`Lifecycle`] is the pure part: the Mac's `isStartingMeeting` / `captureGeneration`
//! race guards. A stop that lands while a start is still writing its row bumps the
//! generation, and the start then ends its own row instead of leaving a recording running
//! that nothing can stop: the row is removed when nothing was recorded, else completed.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use memory::{Meeting, MeetingStatus, MeetingTrigger, NewMeeting};
use schemars::JsonSchema;
use serde::Serialize;

use super::pause_clock::{PauseClock, PausedSpans};
use super::pipeline::{Pipeline, PipelineFailure, Processing};
use super::live::Accumulated;
use super::pipeline::{started_at, stamp_of};
use super::recorder::{session_stamp, stranded_audio, ActiveAudio, AudioMeeting, AudioTarget, RecordingFiles, StartAudio, Stopped};
use super::snapshots::service::{SnapshotHandle, SnapshotMeeting, StartSnapshots};
use crate::error::CommandError;
use crate::store_actor::StoreHandle;
use crate::l10n::tr;

/// A manual meeting's placeholder title; core retitles it from the summary (#65).
pub const MANUAL_TITLE: &str = "Meeting";
/// How many recent rows the launch repair looks at (the Mac's 200).
pub const RECONCILE_WINDOW: usize = 200;

#[derive(Debug)]
pub enum Phase<R> {
    Idle,
    Starting { generation: u64 },
    Recording { generation: u64, meeting_id: i64, started_unix_ms: i64, audio: R },
}

/// The start/stop state machine, generic over the recording so it tests without audio.
#[derive(Debug)]
pub struct Lifecycle<R> {
    generation: u64,
    phase: Phase<R>,
}

impl<R> Default for Lifecycle<R> {
    fn default() -> Self {
        Lifecycle { generation: 0, phase: Phase::Idle }
    }
}

/// What a finished start must do.
#[derive(Debug, PartialEq, Eq)]
pub enum StartOutcome {
    Recording,
    /// A stop arrived mid-start: stop the recording and complete this row.
    Orphaned,
}

impl<R> Lifecycle<R> {
    /// Claims the start, or `None` if a meeting is starting or recording.
    pub fn begin_start(&mut self) -> Option<u64> {
        if !matches!(self.phase, Phase::Idle) {
            return None;
        }
        self.generation += 1;
        self.phase = Phase::Starting { generation: self.generation };
        Some(self.generation)
    }

    /// Records a started meeting, unless a stop superseded this start. On `Orphaned` the
    /// recording is handed back for the caller to stop.
    pub fn finish_start(&mut self, generation: u64, meeting_id: i64, started_unix_ms: i64, audio: R) -> Result<(), R> {
        match self.phase {
            Phase::Starting { generation: g } if g == generation => {
                self.phase = Phase::Recording { generation, meeting_id, started_unix_ms, audio };
                Ok(())
            }
            _ => Err(audio),
        }
    }

    /// A start that failed before recording: back to idle if it's still the current one.
    pub fn fail_start(&mut self, generation: u64) {
        if matches!(self.phase, Phase::Starting { generation: g } if g == generation) {
            self.phase = Phase::Idle;
        }
    }

    /// Stops: hands back a recording meeting's id, start time and recording, or cancels
    /// a start in flight (its `finish_start` will then orphan it).
    pub fn begin_stop(&mut self) -> Option<(i64, i64, R)> {
        match std::mem::replace(&mut self.phase, Phase::Idle) {
            Phase::Recording { meeting_id, started_unix_ms, audio, .. } => {
                self.generation += 1;
                Some((meeting_id, started_unix_ms, audio))
            }
            Phase::Starting { .. } => {
                self.generation += 1;
                None
            }
            Phase::Idle => None,
        }
    }

    pub fn phase(&self) -> &Phase<R> {
        &self.phase
    }

    /// The phase, for the one change that keeps it: a recording pause.
    pub fn phase_mut(&mut self) -> &mut Phase<R> {
        &mut self.phase
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RecordingState {
    Idle,
    Starting,
    Recording,
}

/// What the UI and the tray show about meeting recording.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MeetingRecordingView {
    pub state: RecordingState,
    pub meeting_id: Option<i64>,
    pub started_unix_ms: Option<i64>,
    /// Why this recording may be missing audio, while it runs.
    pub degraded: Option<String>,
    /// The meeting being transcribed or summarized right now, if any (M3.3 Task 4).
    pub processing: Option<Processing>,
    /// The last meeting whose transcript or summary failed.
    pub failure: Option<PipelineFailure>,
    /// The recording is paused; the state stays `recording`, so a paused meeting is still a recording.
    pub paused: bool,
    /// When the running pause began, while paused.
    pub paused_since_unix_ms: Option<i64>,
    /// Time spent paused so far, a running pause included. Recorded time is wall time minus this.
    pub paused_total_ms: i64,
    /// Resume was asked for and audio has not arrived yet.
    pub resuming: bool,
}

/// The audio level meter's reading, 0 to 1 for each source. Read, drawn and thrown away.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AudioLevel {
    /// What the meeting app and the PC play.
    pub system: f64,
    /// The microphone.
    pub mic: f64,
}

/// What the status says about a recording pause.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct PauseView {
    paused: bool,
    since: Option<i64>,
    total_ms: i64,
    resuming: bool,
}

/// Rows this device left `recording` (the app quit or crashed mid-meeting). Pure.
pub fn orphaned_recordings(meetings: &[Meeting], own_device_id: &str) -> Vec<i64> {
    meetings
        .iter()
        .filter(|m| m.status == MeetingStatus::Recording && m.device_id == own_device_id)
        .map(|m| m.id)
        .collect()
}

/// How a row a previous run left `recording` gets completed. The `audio_path` it was
/// started with names its recording; if that run left audio on disk, the row gets that
/// file and an end at the recording's own length. Otherwise it completes as before: no
/// audio, ending `now`. Reads the file system only.
pub fn stranded_completion(audio_path: Option<&str>, recordings: &Path, now_unix_ms: i64) -> (Option<String>, i64) {
    let stamp = audio_path.and_then(|p| stamp_of(Path::new(p)));
    let found = stamp.as_deref().and_then(|stamp| Some((stranded_audio(recordings, stamp)?, started_at(stamp)?)));
    match found {
        Some(((path, length_ms), started)) => (Some(path.to_string_lossy().into_owned()), started.timestamp_millis() + length_ms),
        None => (None, now_unix_ms),
    }
}

/// A recording in progress: its audio and its snapshots, which stop together. Held in the
/// lifecycle as one value, so a stop that races a start can't leave either running.
pub struct Active {
    audio: Box<dyn ActiveAudio>,
    snapshots: Option<SnapshotHandle>,
    /// The recording's pauses.
    clock: PauseClock,
    /// The paused stretches on the transcript's clock, kept beside the recording for the transcriber.
    spans: PausedSpans,
    /// When the audio started (the recorder's own clock zero, to within a few ms).
    t0_unix_ms: i64,
}

impl Active {
    /// Starts or ends a recording pause on the audio and the snapshots together.
    fn set_paused(&self, paused: bool, now_unix_ms: i64) {
        self.audio.set_paused(paused, now_unix_ms);
        if let Some(snapshots) = &self.snapshots {
            snapshots.set_paused(paused);
        }
    }

    fn stop(self) -> Stopped {
        if let Some(snapshots) = &self.snapshots {
            snapshots.stop();
        }
        self.audio.stop()
    }
}

pub struct MeetingController {
    store: StoreHandle,
    device_id: String,
    recordings: PathBuf,
    start_audio: StartAudio,
    start_snapshots: StartSnapshots,
    pipeline: Arc<Pipeline>,
    lifecycle: Mutex<Lifecycle<Active>>,
    /// The recording was started by hand ("Still in a meeting?" applies, not the silence rule).
    manual: std::sync::atomic::AtomicBool,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

impl MeetingController {
    pub fn new(
        store: StoreHandle,
        device_id: String,
        recordings: PathBuf,
        start_audio: StartAudio,
        start_snapshots: StartSnapshots,
        pipeline: Arc<Pipeline>,
    ) -> Self {
        MeetingController {
            store,
            device_id,
            recordings,
            start_audio,
            start_snapshots,
            pipeline,
            lifecycle: Mutex::new(Lifecycle::default()),
            manual: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Whether the recording now running was started by hand.
    pub fn recording_is_manual(&self) -> bool {
        self.manual.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// The PC is going to sleep: a recording stops here, so the meeting ends at the moment of
    /// sleep (first support case: a laptop closed an hour into a call came back 4.5 h later to a
    /// 5 h 26 min "meeting"). Runs on the power-event thread before Windows suspends.
    pub fn stop_for_sleep(&self) {
        let Some(meeting_id) = self.status().meeting_id else { return };
        tracing::info!(meeting_id, "the PC is going to sleep: stopping the recording");
        if let Err(e) = self.stop() {
            tracing::error!(meeting_id, error = %e, "could not stop the recording before sleep");
        }
    }

    /// "Still in a meeting?" answered Stop, or not at all: stops the recording, and once it is
    /// transcribed the pipeline trims it to 30 s after the last words.
    pub fn stop_quiet(&self) -> Result<MeetingRecordingView, CommandError> {
        if let Some(meeting_id) = self.status().meeting_id {
            self.pipeline.trim_after_transcription(meeting_id);
        }
        self.stop()
    }

    /// Completes rows a previous run left `recording` (the Mac's
    /// `reconcileOrphanedRecordingMeetings`), re-linking the recording a crash left behind:
    /// see [`stranded_completion`]. Run once at launch, before the pipeline's repair, which
    /// then transcribes whatever was re-linked.
    pub fn reconcile_at_launch(&self) {
        let (device_id, recordings) = (self.device_id.clone(), self.recordings.clone());
        let result = self.store.call_blocking(move |s| -> Result<usize, String> {
            let meetings = s.recent_meetings(RECONCILE_WINDOW).map_err(|e| e.to_string())?;
            let orphans = orphaned_recordings(&meetings, &device_id);
            let now = now_ms();
            for meeting in meetings.iter().filter(|m| orphans.contains(&m.id)) {
                let (audio_path, end) = stranded_completion(meeting.audio_path.as_deref(), &recordings, now);
                if let Some(path) = &audio_path {
                    tracing::info!(meeting_id = meeting.id, path, "re-linked a recording a previous run left behind");
                }
                s.complete_meeting(meeting.id, end, audio_path.as_deref()).map_err(|e| e.to_string())?;
            }
            Ok(orphans.len())
        });
        match result {
            Ok(Ok(0)) => {}
            Ok(Ok(n)) => tracing::info!(completed = n, "completed meetings left recording by a previous run"),
            Ok(Err(e)) => tracing::warn!(error = %e, "could not repair meetings left recording"),
            Err(e) => tracing::warn!(error = %e, "could not repair meetings left recording"),
        }
    }

    /// "Record now": a manual meeting recording everything the PC plays plus the mic.
    /// "Record now": a manual meeting of everything the PC plays, plus the microphone.
    pub fn start_manual(&self) -> Result<MeetingRecordingView, CommandError> {
        self.start(MeetingTrigger::Manual, MANUAL_TITLE.to_string(), Vec::new(), AudioTarget::Endpoint, None)
    }

    /// A manual meeting with a title and invitees: Start on a meeting only another device's calendar
    /// has (synced, #792), which is never a calendar recording here.
    pub fn start_manual_titled(&self, title: String, attendees: Vec<String>) -> Result<MeetingRecordingView, CommandError> {
        self.start(MeetingTrigger::Manual, title, attendees, AudioTarget::Endpoint, None)
    }

    /// A detected call: the meeting app's own audio, and its name as the title
    /// (M3.3 Task 6). `root_pid` is the process tree process loopback records.
    pub fn start_detected(&self, title: String, root_pid: u32) -> Result<MeetingRecordingView, CommandError> {
        self.start(MeetingTrigger::Detected, title, Vec::new(), AudioTarget::Process(root_pid), None)
    }

    /// A calendar meeting (M3.3 Task 7): the event's title, invitees and start time. It
    /// records the meeting app when detection has found one, else everything the PC plays.
    pub fn start_calendar(
        &self,
        title: String,
        attendees: Vec<String>,
        attendee_details: Vec<memory::speaker_names::Attendee>,
        start_unix_ms: Option<i64>,
        root_pid: Option<u32>,
        event_id: Option<String>,
    ) -> Result<MeetingRecordingView, CommandError> {
        let target = root_pid.map_or(AudioTarget::Endpoint, AudioTarget::Process);
        let started = self.start(MeetingTrigger::Calendar, title, attendees, target, start_unix_ms)?;
        // The invitees' emails, for speaker names. A failure costs only the email match.
        if let (Some(meeting_id), false) = (started.meeting_id, attendee_details.is_empty()) {
            let stored = self.store.call_blocking(move |s| s.set_meeting_attendee_details(meeting_id, &attendee_details).map_err(|e| e.to_string()));
            if let Err(e) | Ok(Err(e)) = stored.map_err(|e| e.to_string()) {
                tracing::warn!(meeting_id, error = %e, "could not store the invitees' emails");
            }
        }
        // The calendar event it was recorded from: the durable link client exclusion follows
        // whatever the meeting's title or start become (X6). A failure leaves the time overlap.
        if let (Some(meeting_id), Some(event_id)) = (started.meeting_id, event_id.filter(|e| !e.is_empty())) {
            let stored = self.store.call_blocking(move |s| s.set_meeting_calendar_event(meeting_id, &event_id).map_err(|e| e.to_string()));
            if let Err(e) | Ok(Err(e)) = stored.map_err(|e| e.to_string()) {
                tracing::warn!(meeting_id, error = %e, "could not store the meeting's calendar event");
            }
        }
        Ok(started)
    }

    fn start(
        &self,
        trigger: MeetingTrigger,
        title: String,
        attendees: Vec<String>,
        target: AudioTarget,
        start_unix_ms: Option<i64>,
    ) -> Result<MeetingRecordingView, CommandError> {
        let generation = self
            .lifecycle
            .lock()
            .unwrap()
            .begin_start()
            .ok_or_else(|| CommandError::invalid_input(tr("A meeting is already recording.", &[])))?;
        self.manual.store(matches!(trigger, MeetingTrigger::Manual), std::sync::atomic::Ordering::Relaxed);
        let now = now_ms();
        // A calendar meeting's row spans its event; everything else starts now. The
        // recording's own clock still runs from this moment.
        let started = start_unix_ms.unwrap_or(now).min(now);
        // The files are named before the row is written so the row can name them: a crash
        // then leaves a row that knows its recording (`reconcile_at_launch`). `stop`
        // replaces the path with the file that actually got audio, or `None`.
        let stamp = session_stamp(chrono::Utc::now());
        let expected = self.recordings.join(format!("system-audio-{stamp}.wav"));
        let snapshot_title = title.clone();
        let row = NewMeeting {
            device_id: self.device_id.clone(),
            start_unix_ms: started,
            end_unix_ms: None,
            title,
            attendees,
            trigger,
            audio_path: Some(expected.to_string_lossy().into_owned()),
            status: MeetingStatus::Recording,
        };
        // A compaction holds the write lock for its whole VACUUM. This start is already marked
        // as starting, so no new one begins; wait for one already running rather than fail on
        // its lock (issue 850 review, I1). The insert below still retries if the cap runs out.
        let waited = std::time::Instant::now();
        if !crate::storage::wait_for_compaction(RECORDING_WAITS_FOR_COMPACTION) {
            tracing::warn!("a compaction is still running; starting the recording anyway");
        } else if waited.elapsed() > std::time::Duration::from_secs(1) {
            tracing::info!(ms = waited.elapsed().as_millis() as u64, "recording start waited for a compaction");
        }
        let inserted = self.store.call_blocking(move |s| insert_recording_row(s, &row));
        let meeting_id = match inserted {
            Ok(Ok(id)) => id,
            Ok(Err(e)) => {
                self.lifecycle.lock().unwrap().fail_start(generation);
                return Err(CommandError::store(e));
            }
            Err(e) => {
                self.lifecycle.lock().unwrap().fail_start(generation);
                return Err(e);
            }
        };

        let audio_t0 = now_ms();
        let audio = (self.start_audio)(target, &stamp, &AudioMeeting { meeting_id, title: snapshot_title.clone() });
        let root_pid = match target {
            AudioTarget::Process(pid) => Some(pid),
            AudioTarget::Endpoint => None,
        };
        let snapshots = (self.start_snapshots)(SnapshotMeeting { meeting_id, start_unix_ms: started, title: snapshot_title, root_pid });
        let finished = self.lifecycle.lock().unwrap().finish_start(generation, meeting_id, now, Active { audio, snapshots, clock: PauseClock::new(), spans: PausedSpans::default(), t0_unix_ms: audio_t0 });
        if let Err(active) = finished {
            tracing::info!(meeting_id, "a stop arrived while the meeting was starting");
            let stopped = active.stop();
            // The Mac's rule (`StartStopRules.stoppedStartOutcome`): a cancelled start that recorded
            // nothing leaves no meeting behind; one that got audio is completed as before.
            if cancelled_start_is_empty(&stopped.files, &stopped.transcribed) && self.discard_cancelled_start(meeting_id) {
                return Ok(self.status());
            }
            self.complete(meeting_id, stopped.files, stopped.transcribed);
        } else {
            tracing::info!(meeting_id, ?trigger, ?target, "meeting recording");
        }
        Ok(self.status())
    }

    /// Stops the recording meeting, if any, and completes its row.
    pub fn stop(&self) -> Result<MeetingRecordingView, CommandError> {
        let stopped = self.lifecycle.lock().unwrap().begin_stop();
        if let Some((meeting_id, started, active)) = stopped {
            super::still_here::reset();
            self.manual.store(false, std::sync::atomic::Ordering::Relaxed);
            let Stopped { files, transcribed } = active.stop();
            tracing::info!(meeting_id, seconds = (now_ms() - started) / 1000, degraded = ?files.degraded, "meeting recording stopped");
            self.complete(meeting_id, files, transcribed);
        }
        Ok(self.status())
    }

    /// Removes the row of a start a stop cancelled before any audio was captured, as the Mac's
    /// `MeetingSplit.discard` does: core's plain delete (`delete_meeting_returning_files`), then its files.
    /// No tombstone: a `recording` row is never pushed (`unpushed_rows` selects a meeting only once it
    /// is completed, and `complete_meeting` queues it), so no other device or server row can exist for it.
    /// True when the row is gone. False when the delete failed, and the caller then completes the row
    /// instead: an empty row is better than a lost recording.
    fn discard_cancelled_start(&self, meeting_id: i64) -> bool {
        let forgotten = self.store.call_blocking(move |s| s.delete_meeting_returning_files(meeting_id).map_err(|e| e.to_string()));
        match forgotten {
            Ok(Ok(deleted)) => {
                if let Some(deleted) = deleted {
                    api::retention::delete_all_meeting_files(&deleted, &self.recordings);
                    super::pipeline::remove_paused_spans_for(&deleted.audio_paths, &self.recordings);
                }
                tracing::info!(meeting_id, "the start was cancelled before any audio was recorded; its meeting was removed");
                true
            }
            Ok(Err(e)) => {
                tracing::error!(meeting_id, error = %e, "could not remove the cancelled start's meeting; completing it instead");
                false
            }
            Err(e) => {
                tracing::error!(meeting_id, error = %e, "could not remove the cancelled start's meeting; completing it instead");
                false
            }
        }
    }

    fn complete(&self, meeting_id: i64, files: RecordingFiles, transcribed: Accumulated) {
        let audio_path = files.audio_path().map(|p| p.to_string_lossy().into_owned());
        let result = self.store.call_blocking(move |s| s.complete_meeting(meeting_id, now_ms(), audio_path.as_deref()).map_err(|e| e.to_string()));
        match result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => tracing::error!(meeting_id, error = %e, "could not complete the meeting row"),
            Err(e) => tracing::error!(meeting_id, error = %e, "could not complete the meeting row"),
        }
        if files.audio_path().is_some() {
            self.pipeline.run_in_background(meeting_id, files, transcribed);
        } else {
            tracing::info!(meeting_id, "no audio was recorded; nothing to transcribe");
        }
    }

    pub fn status(&self) -> MeetingRecordingView {
        let (processing, failure) = (self.pipeline.processing(), self.pipeline.failure());
        let now = now_ms();
        let (recording, pause) = match self.lifecycle.lock().unwrap().phase() {
            Phase::Idle => ((RecordingState::Idle, None, None, None), PauseView::default()),
            Phase::Starting { .. } => ((RecordingState::Starting, None, None, None), PauseView::default()),
            Phase::Recording { meeting_id, started_unix_ms, audio: active, .. } => (
                (RecordingState::Recording, Some(*meeting_id), Some(*started_unix_ms), active.audio.degraded()),
                PauseView {
                    paused: active.clock.is_paused(),
                    since: active.clock.since(),
                    total_ms: active.clock.paused_total(now),
                    resuming: !active.clock.is_paused() && active.audio.resuming(now),
                },
            ),
        };
        MeetingRecordingView {
            state: recording.0,
            meeting_id: recording.1,
            started_unix_ms: recording.2,
            degraded: recording.3,
            processing,
            failure,
            paused: pause.paused,
            paused_since_unix_ms: pause.since,
            paused_total_ms: pause.total_ms,
            resuming: pause.resuming,
        }
    }

    /// Recording pause: audio and snapshots stop until [`resume`](Self::resume). The recording
    /// keeps running as far as everything else is concerned. Pausing a paused recording changes
    /// nothing; with no recording it is an error.
    pub fn pause(&self) -> Result<MeetingRecordingView, CommandError> {
        let now = now_ms();
        let (paused, spans) = {
            let mut lifecycle = self.lifecycle.lock().unwrap();
            match lifecycle.phase_mut() {
                Phase::Recording { audio: active, .. } => {
                    let began = active.clock.pause(now);
                    if began {
                        active.set_paused(true, now);
                        active.spans.open((now - active.t0_unix_ms) / 10);
                    }
                    (began, active.audio.stamp().map(|stamp| (stamp, active.spans.to_text())))
                }
                _ => return Err(CommandError::invalid_input(tr("No meeting is recording.", &[]))),
            }
        };
        if paused {
            self.keep_spans(spans);
            tracing::info!("meeting recording paused");
        }
        Ok(self.status())
    }

    /// Ends a recording pause: the streams reopen and the gap is written as silence. Resuming a
    /// recording that is not paused changes nothing; with no recording it is an error. The quiet
    /// check-in's clock starts over (`still_here::keep`).
    pub fn resume(&self) -> Result<MeetingRecordingView, CommandError> {
        let now = now_ms();
        let (resumed, spans) = {
            let mut lifecycle = self.lifecycle.lock().unwrap();
            match lifecycle.phase_mut() {
                Phase::Recording { audio: active, .. } => {
                    let ended = active.clock.resume(now);
                    if ended {
                        active.set_paused(false, now);
                        active.spans.close((now - active.t0_unix_ms) / 10);
                    }
                    (ended, active.audio.stamp().map(|stamp| (stamp, active.spans.to_text())))
                }
                _ => return Err(CommandError::invalid_input(tr("No meeting is recording.", &[]))),
            }
        };
        if resumed {
            self.keep_spans(spans);
            super::still_here::keep(now);
            tracing::info!("meeting recording resumed");
        }
        Ok(self.status())
    }

    /// Writes the paused stretches beside the recording, so the transcriber can drop anything it hears
    /// in them even after a quit. A failure costs only that protection.
    fn keep_spans(&self, spans: Option<(String, String)>) {
        let Some((stamp, text)) = spans else { return };
        let path = super::pipeline::paused_spans_marker_in(&self.recordings, &stamp);
        if let Err(e) = std::fs::write(&path, text) {
            tracing::warn!(path = %path.display(), error = %e, "could not note the paused stretches beside the recording");
        }
    }

    /// The level meter's reading: zero with nothing recording and while paused (no packet arrives, and
    /// the reading must not linger). The first call after a recording starts turns sampling on.
    pub fn audio_level(&self) -> AudioLevel {
        match self.lifecycle.lock().unwrap().phase() {
            Phase::Recording { audio: active, .. } if !active.clock.is_paused() => {
                let (system, mic) = active.audio.levels(now_ms());
                AudioLevel { system: system as f64, mic: mic as f64 }
            }
            _ => AudioLevel::default(),
        }
    }

    /// When the running pause began, while paused.
    pub fn paused_since(&self) -> Option<i64> {
        match self.lifecycle.lock().unwrap().phase() {
            Phase::Recording { audio: active, .. } => active.clock.since(),
            _ => None,
        }
    }

    /// The recording is paused (a paused recording is still [`is_recording`](Self::is_recording)).
    pub fn is_paused(&self) -> bool {
        matches!(self.lifecycle.lock().unwrap().phase(), Phase::Recording { audio: active, .. } if active.clock.is_paused())
    }

    pub fn is_recording(&self) -> bool {
        matches!(self.lifecycle.lock().unwrap().phase(), Phase::Recording { .. })
    }

    /// A recording is starting or running: no compaction may begin (`storage::compact_unless_recording`).
    pub fn is_starting_or_recording(&self) -> bool {
        !matches!(self.lifecycle.lock().unwrap().phase(), Phase::Idle)
    }

    /// The file stamp of the recording in progress, so a retention sweep can leave its
    /// WAVs alone.
    pub fn recording_stamp(&self) -> Option<String> {
        match self.lifecycle.lock().unwrap().phase() {
            Phase::Recording { audio: active, .. } => active.audio.stamp(),
            _ => None,
        }
    }

    /// Whether the recording meeting's audio is live right now, for the end rules.
    pub fn audio_active(&self, now_unix_ms: i64) -> Option<bool> {
        match self.lifecycle.lock().unwrap().phase() {
            Phase::Recording { audio: active, .. } => Some(active.audio.audio_active(now_unix_ms)),
            _ => None,
        }
    }
}

/// Whether a start a stop cancelled recorded nothing: no audio file (a stream that never delivered
/// audio has its file removed at stop) and no transcript. The Mac's `audioFileExists` rule, with the
/// transcript as a second guard: any doubt keeps the meeting.
pub fn cancelled_start_is_empty(files: &RecordingFiles, transcribed: &Accumulated) -> bool {
    files.audio_path().is_none() && transcribed.system.is_empty() && transcribed.mic.is_empty()
}

/// Each attempt's wait for another connection's lock while starting a recording. Shorter than the
/// lanes' [`BUSY_TIMEOUT`](crate::store_actor::BUSY_TIMEOUT): the user pressed Start and is waiting.
const RECORDING_START_BUSY_WAIT: std::time::Duration = std::time::Duration::from_secs(5);
/// How long a start keeps retrying after a busy error, the Mac's `whimbrel_memory_insert_meeting`
/// budget (issue 850): with the per-attempt wait, a start answers within about 15 s.
const RECORDING_START_RETRY: std::time::Duration = std::time::Duration::from_secs(10);
/// How long a start waits for a compaction already running when it was pressed: past a minute-long
/// VACUUM of a few GB, with room for a slow disk.
const RECORDING_WAITS_FOR_COMPACTION: std::time::Duration = std::time::Duration::from_secs(180);

/// The insert that starts a recording, which must not fail on a lock another lane holds briefly
/// (a sync round, a capture write, the retention sweep): it waits up to
/// [`RECORDING_START_BUSY_WAIT`] an attempt and retries busy errors for [`RECORDING_START_RETRY`],
/// then puts the connection's busy timeout back as it was, 0 included (as the Mac FFI does); only
/// an unreadable one becomes the lanes' [`BUSY_TIMEOUT`](crate::store_actor::BUSY_TIMEOUT).
/// Any other error returns at once.
fn insert_recording_row(s: &memory::Store, row: &NewMeeting) -> Result<i64, String> {
    let usual = s.busy_timeout_ms().ok().map(|ms| std::time::Duration::from_millis(ms.max(0) as u64));
    let _ = s.set_busy_timeout(RECORDING_START_BUSY_WAIT);
    let inserted = memory::retry_when_busy(RECORDING_START_RETRY, || s.insert_meeting(row));
    let _ = s.set_busy_timeout(usual.unwrap_or(crate::store_actor::BUSY_TIMEOUT));
    inserted.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;

    #[test]
    fn a_recording_start_inserts_its_row_and_puts_the_busy_timeout_back() {
        let store = memory::Store::open_in_memory().unwrap();
        store.set_busy_timeout(crate::store_actor::BUSY_TIMEOUT).unwrap();
        let row = NewMeeting {
            device_id: "this-pc".into(),
            start_unix_ms: 1_000,
            end_unix_ms: None,
            title: "Standup".into(),
            attendees: Vec::new(),
            trigger: MeetingTrigger::Manual,
            audio_path: None,
            status: MeetingStatus::Recording,
        };
        let id = insert_recording_row(&store, &row.clone()).unwrap();
        assert!(store.get_meeting(id).unwrap().is_some());
        assert_eq!(store.busy_timeout_ms().unwrap(), 30_000);
        // Issue 850 review, M8: a timeout of 0 comes back as 0, as the Mac FFI restores it.
        store.set_busy_timeout(std::time::Duration::ZERO).unwrap();
        insert_recording_row(&store, &row).unwrap();
        assert_eq!(store.busy_timeout_ms().unwrap(), 0);
    }

    // ---- Lifecycle ----

    #[test]
    fn a_start_then_stop_hands_back_the_recording() {
        let mut l = Lifecycle::default();
        let g = l.begin_start().unwrap();
        assert!(l.finish_start(g, 7, 100, "audio").is_ok());
        assert_eq!(l.begin_stop(), Some((7, 100, "audio")));
        assert!(matches!(l.phase(), Phase::Idle));
    }

    #[test]
    fn a_second_start_is_refused_while_starting_or_recording() {
        let mut l: Lifecycle<&str> = Lifecycle::default();
        let g = l.begin_start().unwrap();
        assert_eq!(l.begin_start(), None, "while starting");
        l.finish_start(g, 1, 0, "a").unwrap();
        assert_eq!(l.begin_start(), None, "while recording");
    }

    #[test]
    fn a_stop_during_a_start_orphans_that_start() {
        let mut l = Lifecycle::default();
        let g = l.begin_start().unwrap();
        assert_eq!(l.begin_stop(), None, "nothing to stop yet");
        assert_eq!(l.finish_start(g, 9, 0, "audio"), Err("audio"), "the start must clean up its own row");
        assert!(matches!(l.phase(), Phase::Idle));
        assert!(l.begin_start().is_some(), "a new start is allowed");
    }

    #[test]
    fn a_stale_start_cannot_take_over_a_newer_one() {
        let mut l = Lifecycle::default();
        let old = l.begin_start().unwrap();
        l.begin_stop();
        let new = l.begin_start().unwrap();
        assert_eq!(l.finish_start(old, 1, 0, "old"), Err("old"));
        assert!(l.finish_start(new, 2, 0, "new").is_ok());
    }

    #[test]
    fn a_failed_start_returns_to_idle_only_if_still_current() {
        let mut l: Lifecycle<&str> = Lifecycle::default();
        let old = l.begin_start().unwrap();
        l.begin_stop();
        let new = l.begin_start().unwrap();
        l.fail_start(old);
        assert!(matches!(l.phase(), Phase::Starting { generation } if *generation == new));
        l.fail_start(new);
        assert!(matches!(l.phase(), Phase::Idle));
    }

    // ---- Controller against a real store, with fake audio ----

    struct FakeAudio {
        files: RecordingFiles,
    }

    impl ActiveAudio for FakeAudio {
        fn stop(self: Box<Self>) -> Stopped {
            // Nothing transcribed: the pipeline does all of it, which is what the tests
            // that follow assert against.
            self.files.into()
        }
        fn audio_active(&self, _now: i64) -> bool {
            false
        }
        fn degraded(&self) -> Option<String> {
            self.files.degraded.clone()
        }
        fn stamp(&self) -> Option<String> {
            Some("S".into())
        }
        fn set_paused(&self, paused: bool, _now: i64) {
            PAUSES.with(|log| log.borrow_mut().push(paused));
        }
        fn resuming(&self, _now: i64) -> bool {
            RESUMING.with(|flag| flag.get())
        }
        fn levels(&self, _now: i64) -> (f32, f32) {
            (0.5, 0.25)
        }
    }

    // What a test's controller was told about pauses, per test thread (each controller call runs on
    // the test's own thread), so the fake needs no new field.
    thread_local! {
        static PAUSES: std::cell::RefCell<Vec<bool>> = const { std::cell::RefCell::new(Vec::new()) };
        static RESUMING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    fn pauses() -> Vec<bool> {
        PAUSES.with(|log| log.borrow().clone())
    }

    /// A test thread may be shared (`--test-threads=1`): start from a clean log.
    fn reset_pauses() {
        PAUSES.with(|log| log.borrow_mut().clear());
        RESUMING.with(|flag| flag.set(false));
    }

    fn controller(files: RecordingFiles) -> (MeetingController, StoreHandle) {
        controller_in(files, PathBuf::from(r"C:\rec"), Arc::new(Mutex::new(None)))
    }

    /// A controller recording into `recordings`; `seen` receives the stamp each start
    /// hands the recorder.
    fn controller_in(files: RecordingFiles, recordings: PathBuf, seen: Arc<Mutex<Option<String>>>) -> (MeetingController, StoreHandle) {
        let start: StartAudio = Arc::new(move |_target, stamp, _meeting| {
            *seen.lock().unwrap() = Some(stamp.to_string());
            Box::new(FakeAudio { files: files.clone() })
        });
        controller_with_start(start, recordings)
    }

    /// A controller recording into `recordings` with the given audio starter.
    fn controller_with_start(start: StartAudio, recordings: PathBuf) -> (MeetingController, StoreHandle) {
        let store = StoreHandle::spawn(|| memory::Store::open_in_memory().map_err(|e| e.to_string())).unwrap();
        // A pipeline with no models installed: `run_in_background` reports a failure and
        // leaves the row alone, which is what these tests want.
        let paths = crate::paths::Paths::under(std::env::temp_dir().join(format!("whimbrel-controller-{}", std::process::id())));
        let pipeline = Arc::new(Pipeline::new(
            store.clone(),
            store.clone(),
            Arc::new(Mutex::new(crate::settings::SettingsFile::load(&paths.settings))),
            Arc::new(crate::secrets::MemorySecrets::default()),
            paths,
        ));
        (MeetingController::new(store.clone(), "this-pc".into(), recordings, start, Arc::new(|_| None), pipeline), store)
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("whimbrel-stranded-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A WAV of `audio_bytes` bytes after its 44-byte header, unfinalized like a crash's.
    fn wav(path: &Path, audio_bytes: usize) {
        std::fs::write(path, vec![0u8; 44 + audio_bytes]).unwrap();
    }

    /// A controller whose recorder presses Stop while it starts: the row is written and the audio is
    /// being opened, but the start has not finished. `during` runs there, once, right after that Stop.
    /// Returns the ids of the rows the starts wrote, in order.
    fn controller_stopped_mid_start(
        files: RecordingFiles,
        recordings: PathBuf,
        during: impl Fn(&MeetingController, &str) + Send + Sync + 'static,
    ) -> (Arc<MeetingController>, StoreHandle, Arc<Mutex<Vec<i64>>>) {
        let cell: Arc<std::sync::OnceLock<Arc<MeetingController>>> = Arc::default();
        let ids: Arc<Mutex<Vec<i64>>> = Arc::default();
        let first = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let start: StartAudio = {
            let (cell, ids) = (cell.clone(), ids.clone());
            Arc::new(move |_target, stamp, meeting| {
                ids.lock().unwrap().push(meeting.meeting_id);
                if first.swap(false, std::sync::atomic::Ordering::SeqCst) {
                    let c = cell.get().expect("the controller is set before any start");
                    c.stop().unwrap();
                    let c: &MeetingController = c;
                    during(c, stamp);
                }
                Box::new(FakeAudio { files: files.clone() })
            })
        };
        let (c, store) = controller_with_start(start, recordings);
        let c = Arc::new(c);
        let _ = cell.set(c.clone());
        (c, store, ids)
    }

    #[test]
    fn a_cancelled_start_is_empty_only_with_no_audio_and_no_transcript() {
        let none = RecordingFiles::default();
        let system = RecordingFiles { system: Some(PathBuf::from(r"C:\rec\system-audio-S.wav")), ..Default::default() };
        let mic = RecordingFiles { mic: Some(PathBuf::from(r"C:\rec\mic-audio-S.wav")), ..Default::default() };
        let said = Accumulated {
            mic: vec![transcribe::TranscriptSegment { start_cs: 0, end_cs: 100, text: "hello".into() }],
            ..Default::default()
        };
        assert!(cancelled_start_is_empty(&none, &Accumulated::default()));
        assert!(!cancelled_start_is_empty(&system, &Accumulated::default()), "system audio was captured");
        assert!(!cancelled_start_is_empty(&mic, &Accumulated::default()), "microphone audio was captured");
        assert!(!cancelled_start_is_empty(&none, &said), "words were transcribed");
    }

    #[test]
    fn a_stop_during_a_start_that_recorded_nothing_leaves_no_meeting_and_no_file() {
        let dir = scratch("cancel-empty");
        let orphan_dir = dir.clone();
        // The recorder had already created its (header-only) file when the Stop landed.
        let (c, store, ids) = controller_stopped_mid_start(RecordingFiles::default(), dir.clone(), move |_, stamp| {
            wav(&orphan_dir.join(format!("system-audio-{stamp}.wav")), 0);
        });
        // The device id every PC sets at launch; without it core withholds a meeting's file paths.
        store.call_blocking(|s| s.set_own_device_id("this-pc")).unwrap();

        let view = c.start_manual().unwrap();
        assert_eq!((view.state, view.meeting_id), (RecordingState::Idle, None));
        let id = ids.lock().unwrap()[0];
        let (row, recent, forgotten) = store
            .call_blocking(move |s| (s.get_meeting(id).unwrap(), s.recent_meetings(10).unwrap(), s.is_meeting_forgotten("this-pc", None, id).unwrap()))
            .unwrap();
        assert!(row.is_none(), "the cancelled start's row is gone");
        assert!(recent.is_empty(), "and no other row was left");
        assert!(!forgotten, "a row that never completed was never pushed, so it needs no tombstone");
        let leftovers: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name()).collect();
        assert!(leftovers.is_empty(), "no audio file is left behind: {leftovers:?}");
        assert!(!c.is_starting_or_recording(), "the controller is idle");
        assert!(c.start_manual().is_ok(), "a new start is allowed");
        c.stop().unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_stop_during_a_start_that_captured_audio_keeps_the_meeting() {
        for files in [
            RecordingFiles { system: Some(PathBuf::from(r"C:\rec\system-audio-S.wav")), ..Default::default() },
            RecordingFiles { mic: Some(PathBuf::from(r"C:\rec\mic-audio-S.wav")), ..Default::default() },
        ] {
            let expected = files.audio_path().unwrap().to_string_lossy().into_owned();
            let (c, store, ids) = controller_stopped_mid_start(files, PathBuf::from(r"C:\rec"), |_, _| {});
            let view = c.start_manual().unwrap();
            assert_eq!((view.state, view.meeting_id), (RecordingState::Idle, None));
            let id = ids.lock().unwrap()[0];
            let row = store.call_blocking(move |s| s.get_meeting(id).unwrap()).unwrap().expect("the recording's meeting is kept");
            assert_eq!(row.status, MeetingStatus::Completed);
            assert_eq!(row.audio_path.as_deref(), Some(expected.as_str()), "with the audio attached");
            assert!(row.end_unix_ms.is_some());
        }
    }

    #[test]
    fn a_cancelled_start_never_touches_a_newer_start() {
        // The Stop lands, and a new start begins before the cancelled one has finished opening its audio.
        let (c, store, ids) = controller_stopped_mid_start(RecordingFiles::default(), PathBuf::from(r"C:\rec"), |c, _| {
            c.start_manual().unwrap();
        });
        let view = c.start_manual().unwrap();
        let ids = ids.lock().unwrap().clone();
        assert_eq!(ids.len(), 2, "two starts wrote a row each");
        assert_eq!((view.state, view.meeting_id), (RecordingState::Recording, Some(ids[1])), "the newer start is the recording");
        let (cancelled, newer) = store
            .call_blocking(move |s| (s.get_meeting(ids[0]).unwrap(), s.get_meeting(ids[1]).unwrap()))
            .unwrap();
        assert!(cancelled.is_none(), "only the cancelled start's row went");
        assert_eq!(newer.expect("the newer row stays").status, MeetingStatus::Recording);
        c.stop().unwrap();
    }

    #[test]
    fn a_start_names_its_recording_on_the_row() {
        let seen = Arc::new(Mutex::new(None));
        let (c, store) = controller_in(RecordingFiles::default(), PathBuf::from(r"C:\rec"), seen.clone());
        let id = c.start_manual().unwrap().meeting_id.unwrap();
        let row = store.call_blocking(move |s| s.get_meeting(id).unwrap().unwrap()).unwrap();
        let stamp = seen.lock().unwrap().clone().expect("the recorder got a stamp");
        assert_eq!(row.status, MeetingStatus::Recording);
        assert_eq!(row.audio_path, Some(format!(r"C:\rec\system-audio-{stamp}.wav")), "the row names the recorder's files");
    }

    #[test]
    fn a_stop_that_got_no_audio_clears_the_named_path() {
        let (c, store) = controller(RecordingFiles::default());
        let id = c.start_manual().unwrap().meeting_id.unwrap();
        c.stop().unwrap();
        let row = store.call_blocking(move |s| s.get_meeting(id).unwrap().unwrap()).unwrap();
        assert_eq!((row.status, row.audio_path), (MeetingStatus::Completed, None));
    }

    #[test]
    fn a_stranded_recording_is_relinked_with_its_own_length() {
        let dir = scratch("both");
        let stamp = "2026-10-25T13-00-00.000Z";
        wav(&dir.join(format!("system-audio-{stamp}.wav")), 64_000); // 2 s
        wav(&dir.join(format!("mic-audio-{stamp}.wav")), 96_000); // 3 s
        let row = dir.join(format!("system-audio-{stamp}.wav")).to_string_lossy().into_owned();
        let started = started_at(stamp).unwrap().timestamp_millis();

        let (path, end) = stranded_completion(Some(&row), &dir, 0);
        assert_eq!(path, Some(dir.join(format!("system-audio-{stamp}.wav")).to_string_lossy().into_owned()));
        assert_eq!(end, started + 3_000, "the longer stream sets the end");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_stranded_system_file_with_no_audio_gives_way_to_the_mic() {
        let dir = scratch("mic");
        let stamp = "2026-10-25T13-00-00.000Z";
        wav(&dir.join(format!("system-audio-{stamp}.wav")), 0);
        wav(&dir.join(format!("mic-audio-{stamp}.wav")), 32_000);
        let row = dir.join(format!("system-audio-{stamp}.wav")).to_string_lossy().into_owned();
        let (path, _) = stranded_completion(Some(&row), &dir, 0);
        assert_eq!(path, Some(dir.join(format!("mic-audio-{stamp}.wav")).to_string_lossy().into_owned()));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_crash_before_any_audio_arrived_links_nothing() {
        let dir = scratch("silent");
        let stamp = "2026-10-25T13-00-00.000Z";
        wav(&dir.join(format!("system-audio-{stamp}.wav")), 0);
        wav(&dir.join(format!("mic-audio-{stamp}.wav")), 0);
        let row = dir.join(format!("system-audio-{stamp}.wav")).to_string_lossy().into_owned();
        assert_eq!(stranded_completion(Some(&row), &dir, 42), (None, 42), "as a stop with no audio would");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_row_with_nothing_on_disk_completes_as_before() {
        let dir = scratch("none");
        let named = dir.join("system-audio-2026-10-25T13-00-00.000Z.wav").to_string_lossy().into_owned();
        assert_eq!(stranded_completion(Some(&named), &dir, 42), (None, 42), "the files never appeared");
        assert_eq!(stranded_completion(None, &dir, 42), (None, 42), "a row from before this change");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn launch_repair_relinks_a_crash_and_hands_it_to_the_pipeline_repair() {
        let dir = scratch("launch");
        let (c, store) = controller_in(RecordingFiles::default(), dir.clone(), Arc::new(Mutex::new(None)));
        let stamp = "2026-10-25T13-00-00.000Z";
        let system = dir.join(format!("system-audio-{stamp}.wav"));
        wav(&system, 32_000);
        let (named, empty) = (system.to_string_lossy().into_owned(), dir.join("system-audio-2026-10-25T09-00-00.000Z.wav"));
        let empty = empty.to_string_lossy().into_owned();
        let ids = store
            .call_blocking(move |s| {
                let row = |audio_path: String| NewMeeting {
                    device_id: "this-pc".into(),
                    start_unix_ms: 1_000,
                    end_unix_ms: None,
                    title: "M".into(),
                    attendees: Vec::new(),
                    trigger: MeetingTrigger::Calendar,
                    audio_path: Some(audio_path),
                    status: MeetingStatus::Recording,
                };
                (s.insert_meeting(&row(named)).unwrap(), s.insert_meeting(&row(empty)).unwrap())
            })
            .unwrap();
        c.reconcile_at_launch();

        let (relinked, lost, recent) = store
            .call_blocking(move |s| (s.get_meeting(ids.0).unwrap().unwrap(), s.get_meeting(ids.1).unwrap().unwrap(), s.recent_meetings(10).unwrap()))
            .unwrap();
        assert_eq!(relinked.status, MeetingStatus::Completed);
        assert_eq!(relinked.audio_path.as_deref(), Some(system.to_string_lossy().as_ref()));
        assert_eq!(relinked.end_unix_ms, Some(started_at(stamp).unwrap().timestamp_millis() + 1_000));
        assert_eq!((lost.status, lost.audio_path), (MeetingStatus::Completed, None));
        assert_eq!(crate::meetings::pipeline::needs_pipeline(&recent, "this-pc"), vec![ids.0], "only the re-linked one is transcribed");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn record_now_writes_a_recording_row_and_stop_completes_it_with_the_system_audio() {
        let files = RecordingFiles {
            system: Some(PathBuf::from(r"C:\rec\system-audio-S.wav")),
            mic: Some(PathBuf::from(r"C:\rec\mic-audio-S.wav")),
            degraded: None,
        };
        let (c, store) = controller(files);
        let started = c.start_manual().unwrap();
        assert_eq!(started.state, RecordingState::Recording);
        let id = started.meeting_id.unwrap();

        let row = store.call_blocking(move |s| s.get_meeting(id).unwrap().unwrap()).unwrap();
        assert_eq!((row.status, row.trigger, row.title.as_str()), (MeetingStatus::Recording, MeetingTrigger::Manual, MANUAL_TITLE));
        assert_eq!(row.device_id, "this-pc");

        assert!(c.start_manual().is_err(), "one meeting at a time");
        assert_eq!(c.stop().unwrap().state, RecordingState::Idle);
        let row = store.call_blocking(move |s| s.get_meeting(id).unwrap().unwrap()).unwrap();
        assert_eq!(row.status, MeetingStatus::Completed);
        assert_eq!(row.audio_path.as_deref(), Some(r"C:\rec\system-audio-S.wav"));
        assert!(row.end_unix_ms.is_some());
    }

    #[test]
    fn a_recording_starts_its_snapshots_and_a_stop_stops_them() {
        let (mut c, _) = controller(RecordingFiles::default());
        let started: Arc<Mutex<Vec<SnapshotMeeting>>> = Arc::default();
        type Probe = Box<dyn Fn() -> bool + Send>;
        let probes: Arc<Mutex<Vec<Probe>>> = Arc::default();
        c.start_snapshots = {
            let (started, probes) = (started.clone(), probes.clone());
            Arc::new(move |meeting| {
                started.lock().unwrap().push(meeting);
                let (handle, probe) = SnapshotHandle::unwatched();
                probes.lock().unwrap().push(Box::new(probe));
                Some(handle)
            })
        };
        let id = c.start_detected("Zoom".into(), 42).unwrap().meeting_id.unwrap();
        {
            let started = started.lock().unwrap();
            assert_eq!((started[0].meeting_id, started[0].title.as_str(), started[0].root_pid), (id, "Zoom", Some(42)));
        }
        assert!(!probes.lock().unwrap()[0](), "running while the meeting records");
        c.stop().unwrap();
        assert!(probes.lock().unwrap()[0](), "stopped with the recording");

        c.start_manual().unwrap();
        assert_eq!(started.lock().unwrap()[1].root_pid, None, "a manual meeting has no app of its own");
    }

    #[test]
    fn stop_with_nothing_recording_is_a_no_op() {
        let (c, _) = controller(RecordingFiles::default());
        assert_eq!(c.stop().unwrap().state, RecordingState::Idle);
    }

    #[test]
    fn the_status_carries_a_degraded_reason() {
        let files = RecordingFiles { degraded: Some("mic only".into()), ..Default::default() };
        let (c, _) = controller(files);
        assert_eq!(c.start_manual().unwrap().degraded.as_deref(), Some("mic only"));
    }

    #[test]
    fn launch_repair_completes_only_this_devices_recording_rows() {
        let (c, store) = controller(RecordingFiles::default());
        let ids = store
            .call_blocking(|s| {
                let row = |device: &str, status| NewMeeting {
                    device_id: device.into(),
                    start_unix_ms: 1_000,
                    end_unix_ms: None,
                    title: "M".into(),
                    attendees: Vec::new(),
                    trigger: MeetingTrigger::Manual,
                    audio_path: None,
                    status,
                };
                (
                    s.insert_meeting(&row("this-pc", MeetingStatus::Recording)).unwrap(),
                    s.insert_meeting(&row("a-mac", MeetingStatus::Recording)).unwrap(),
                    s.insert_meeting(&row("this-pc", MeetingStatus::Completed)).unwrap(),
                )
            })
            .unwrap();
        c.reconcile_at_launch();
        let statuses = store
            .call_blocking(move |s| [ids.0, ids.1, ids.2].map(|id| s.get_meeting(id).unwrap().unwrap().status))
            .unwrap();
        assert_eq!(statuses, [MeetingStatus::Completed, MeetingStatus::Recording, MeetingStatus::Completed]);
    }
    // ---- Recording pause ----

    #[test]
    fn a_paused_recording_is_still_a_recording_for_everything_that_asks() {
        reset_pauses();
        let (c, _) = controller(RecordingFiles::default());
        c.start_manual().unwrap();
        let view = c.pause().unwrap();
        assert_eq!(view.state, RecordingState::Recording, "the state never becomes anything a check would read as idle");
        assert!(view.paused && view.paused_since_unix_ms.is_some());
        assert!(c.is_paused() && c.is_recording() && c.is_starting_or_recording());
        assert!(c.start_manual().is_err(), "a second meeting cannot start during a pause");
        assert_eq!(c.recording_stamp().as_deref(), Some("S"), "the retention sweep still leaves its files alone");
        assert_eq!(pauses(), vec![true], "the audio and snapshots were told once");
    }

    #[test]
    fn resume_ends_the_pause_banks_its_time_and_says_resuming_until_audio_arrives() {
        reset_pauses();
        RESUMING.with(|flag| flag.set(true));
        let (c, _) = controller(RecordingFiles::default());
        c.start_manual().unwrap();
        c.pause().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        let view = c.resume().unwrap();
        assert!(!view.paused && view.paused_since_unix_ms.is_none());
        assert!(view.paused_total_ms >= 20, "the pause stays in the total");
        assert!(view.resuming, "the audio side says audio has not arrived");
        RESUMING.with(|flag| flag.set(false));
        assert!(!c.status().resuming);
        assert_eq!(c.status().paused_total_ms, view.paused_total_ms, "the total holds still once resumed");
        assert_eq!(pauses(), vec![true, false]);
    }

    #[test]
    fn a_paused_recording_does_not_say_resuming() {
        reset_pauses();
        RESUMING.with(|flag| flag.set(true));
        let (c, _) = controller(RecordingFiles::default());
        c.start_manual().unwrap();
        assert!(!c.pause().unwrap().resuming);
        RESUMING.with(|flag| flag.set(false));
    }

    #[test]
    fn pausing_twice_or_resuming_when_running_changes_nothing() {
        reset_pauses();
        let (c, _) = controller(RecordingFiles::default());
        c.start_manual().unwrap();
        assert!(!c.resume().unwrap().paused);
        let first = c.pause().unwrap().paused_since_unix_ms;
        assert_eq!(c.pause().unwrap().paused_since_unix_ms, first, "the first pause start stands");
        assert_eq!(pauses(), vec![true], "the second pause was not passed on");
    }

    #[test]
    fn pause_and_resume_need_a_recording() {
        reset_pauses();
        let (c, _) = controller(RecordingFiles::default());
        assert!(c.pause().is_err());
        assert!(c.resume().is_err());
        assert!(!c.is_paused());
    }

    #[test]
    fn pausing_never_marks_the_audio_as_degraded() {
        reset_pauses();
        let (c, _) = controller(RecordingFiles::default());
        c.start_manual().unwrap();
        assert_eq!(c.pause().unwrap().degraded, None);
        assert_eq!(c.resume().unwrap().degraded, None);
    }

    #[test]
    fn a_paused_meeting_stops_cleanly_without_resuming_first() {
        reset_pauses();
        let files = RecordingFiles { system: Some(PathBuf::from(r"C:\rec\system-audio-S.wav")), ..Default::default() };
        let (c, store) = controller(files);
        let id = c.start_manual().unwrap().meeting_id.unwrap();
        c.pause().unwrap();
        let view = c.stop().unwrap();
        assert_eq!((view.state, view.paused), (RecordingState::Idle, false), "nothing about the pause outlives the recording");
        let row = store.call_blocking(move |s| s.get_meeting(id).unwrap().unwrap()).unwrap();
        assert_eq!(row.status, MeetingStatus::Completed);
        assert_eq!(pauses(), vec![true], "stop does not resume first");
    }

    #[test]
    fn sleep_stops_a_paused_recording_like_any_other() {
        reset_pauses();
        let (c, _) = controller(RecordingFiles::default());
        c.start_manual().unwrap();
        c.pause().unwrap();
        c.stop_for_sleep();
        assert_eq!(c.status().state, RecordingState::Idle);
    }

    #[test]
    fn a_new_recording_starts_unpaused_after_a_paused_one() {
        reset_pauses();
        let (c, _) = controller(RecordingFiles::default());
        c.start_manual().unwrap();
        c.pause().unwrap();
        c.stop().unwrap();
        let view = c.start_manual().unwrap();
        assert_eq!((view.paused, view.paused_total_ms), (false, 0));
    }

    #[test]
    fn a_paused_recording_starts_its_snapshots_paused_and_resume_lets_them_go_on() {
        reset_pauses();
        let (mut c, _) = controller(RecordingFiles::default());
        let probes: Arc<Mutex<Vec<Box<dyn Fn() -> bool + Send>>>> = Arc::default();
        c.start_snapshots = {
            let probes = probes.clone();
            Arc::new(move |_meeting| {
                let (handle, probe) = SnapshotHandle::unwatched_paused();
                probes.lock().unwrap().push(Box::new(probe));
                Some(handle)
            })
        };
        c.start_manual().unwrap();
        assert!(!probes.lock().unwrap()[0](), "snapshots run while recording");
        c.pause().unwrap();
        assert!(probes.lock().unwrap()[0](), "snapshots stop for a pause");
        c.resume().unwrap();
        assert!(!probes.lock().unwrap()[0](), "and start again on resume");
    }

    #[test]
    fn the_lifecycle_keeps_its_phase_through_a_pause() {
        let mut l: Lifecycle<u8> = Lifecycle::default();
        let g = l.begin_start().unwrap();
        l.finish_start(g, 5, 1_000, 7).unwrap();
        if let Phase::Recording { audio, .. } = l.phase_mut() {
            *audio = 8;
        }
        assert!(matches!(l.phase(), Phase::Recording { meeting_id: 5, audio: 8, .. }));
        assert!(l.begin_start().is_none(), "still one meeting at a time");
    }

    #[test]
    fn the_level_is_read_while_recording_and_is_zero_otherwise_and_while_paused() {
        reset_pauses();
        let (c, _) = controller(RecordingFiles::default());
        assert_eq!(c.audio_level(), AudioLevel::default(), "nothing recording");
        c.start_manual().unwrap();
        assert_eq!(c.audio_level(), AudioLevel { system: 0.5, mic: 0.25 });
        c.pause().unwrap();
        assert_eq!(c.audio_level(), AudioLevel::default(), "zero while paused");
        c.resume().unwrap();
        assert_eq!(c.audio_level().system, 0.5);
        c.stop().unwrap();
        assert_eq!(c.audio_level(), AudioLevel::default());
    }
}
