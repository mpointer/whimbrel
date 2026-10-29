//! The meeting controller (M3.3 Task 3): the one place that starts and stops meeting
//! recordings and writes their rows, ported from the Mac's `AppState` start/end
//! orchestration.
//!
//! [`Lifecycle`] is the pure part: the Mac's `isStartingMeeting` / `captureGeneration`
//! race guards. A stop that lands while a start is still writing its row bumps the
//! generation, and the start then completes its own row as an orphan instead of leaving
//! a recording running that nothing can stop.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use memory::{Meeting, MeetingStatus, MeetingTrigger, NewMeeting};
use schemars::JsonSchema;
use serde::Serialize;

use super::pipeline::{Pipeline, PipelineFailure, Processing};
use super::live::Accumulated;
use super::pipeline::{started_at, stamp_of};
use super::recorder::{session_stamp, stranded_audio, ActiveAudio, AudioTarget, RecordingFiles, StartAudio, Stopped};
use super::snapshots::service::{SnapshotHandle, SnapshotMeeting, StartSnapshots};
use crate::error::CommandError;
use crate::store_actor::StoreHandle;

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
}

impl Active {
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
        }
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
        start_unix_ms: Option<i64>,
        root_pid: Option<u32>,
    ) -> Result<MeetingRecordingView, CommandError> {
        let target = root_pid.map_or(AudioTarget::Endpoint, AudioTarget::Process);
        self.start(MeetingTrigger::Calendar, title, attendees, target, start_unix_ms)
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
            .ok_or_else(|| CommandError::invalid_input("A meeting is already recording."))?;
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
        let inserted = self.store.call_blocking(move |s| s.insert_meeting(&row).map_err(|e| e.to_string()));
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

        let audio = (self.start_audio)(target, &stamp);
        let root_pid = match target {
            AudioTarget::Process(pid) => Some(pid),
            AudioTarget::Endpoint => None,
        };
        let snapshots = (self.start_snapshots)(SnapshotMeeting { meeting_id, start_unix_ms: started, title: snapshot_title, root_pid });
        let finished = self.lifecycle.lock().unwrap().finish_start(generation, meeting_id, now, Active { audio, snapshots });
        if let Err(active) = finished {
            tracing::info!(meeting_id, "a stop arrived while the meeting was starting; completing it");
            let stopped = active.stop();
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
            let Stopped { files, transcribed } = active.stop();
            tracing::info!(meeting_id, seconds = (now_ms() - started) / 1000, degraded = ?files.degraded, "meeting recording stopped");
            self.complete(meeting_id, files, transcribed);
        }
        Ok(self.status())
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
        let recording = match self.lifecycle.lock().unwrap().phase() {
            Phase::Idle => (RecordingState::Idle, None, None, None),
            Phase::Starting { .. } => (RecordingState::Starting, None, None, None),
            Phase::Recording { meeting_id, started_unix_ms, audio: active, .. } => {
                (RecordingState::Recording, Some(*meeting_id), Some(*started_unix_ms), active.audio.degraded())
            }
        };
        MeetingRecordingView {
            state: recording.0,
            meeting_id: recording.1,
            started_unix_ms: recording.2,
            degraded: recording.3,
            processing,
            failure,
        }
    }

    pub fn is_recording(&self) -> bool {
        matches!(self.lifecycle.lock().unwrap().phase(), Phase::Recording { .. })
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;

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
    }

    fn controller(files: RecordingFiles) -> (MeetingController, StoreHandle) {
        controller_in(files, PathBuf::from(r"C:\rec"), Arc::new(Mutex::new(None)))
    }

    /// A controller recording into `recordings`; `seen` receives the stamp each start
    /// hands the recorder.
    fn controller_in(files: RecordingFiles, recordings: PathBuf, seen: Arc<Mutex<Option<String>>>) -> (MeetingController, StoreHandle) {
        let store = StoreHandle::spawn(|| memory::Store::open_in_memory().map_err(|e| e.to_string())).unwrap();
        let start: StartAudio = Arc::new(move |_target, stamp| {
            *seen.lock().unwrap() = Some(stamp.to_string());
            Box::new(FakeAudio { files: files.clone() })
        });
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
}
