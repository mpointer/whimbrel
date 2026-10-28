//! Retention (M3.3 Task 8): old captures, meetings and conversations expire, and audio
//! no meeting claims is swept.
//!
//! The rules and the deletions are core's (`api::retention`); this is the schedule, the
//! settings, and the one thing core can't know — which files the recorder is writing
//! right now, which must never be swept out from under it.
//!
//! Windows-only wrinkle: the first sweep waits for the user. The windows are defaults
//! nobody has agreed to, so until the first-run notice is acknowledged the schedule only
//! counts what it would delete ([`Retention::run_scheduled`]).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use memory::RetentionPolicy;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::CommandError;
use crate::meetings::controller::MeetingController;
use crate::settings::SettingsFile;
use crate::store_actor::StoreHandle;

/// The Mac's schedule: a first run shortly after launch, then daily.
pub const FIRST_RUN_AFTER: Duration = Duration::from_secs(30);
pub const EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// The Mac's defaults, in days.
pub const DEFAULT_CAPTURE_DAYS: u32 = 30;
pub const DEFAULT_MEETING_DAYS: u32 = 90;
pub const DEFAULT_CONVERSATION_DAYS: u32 = 30;

/// Settings → Retention. `None` days means "keep forever", which is how core reads a
/// missing window too.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RetentionSettings {
    pub capture_days: Option<u32>,
    pub meeting_days: Option<u32>,
    pub conversation_days: Option<u32>,
}

impl Default for RetentionSettings {
    fn default() -> Self {
        RetentionSettings {
            capture_days: Some(DEFAULT_CAPTURE_DAYS),
            meeting_days: Some(DEFAULT_MEETING_DAYS),
            conversation_days: Some(DEFAULT_CONVERSATION_DAYS),
        }
    }
}

impl RetentionSettings {
    pub fn policy(&self) -> RetentionPolicy {
        RetentionPolicy {
            capture_item_days: self.capture_days,
            meeting_days: self.meeting_days,
            conversation_days: self.conversation_days,
        }
    }
}

/// What a sweep would remove right now, for the first-run notice. The shell's own shape
/// so it can cross to TypeScript; the numbers are core's.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RetentionPreview {
    pub captures: usize,
    pub meetings: usize,
    pub conversations: usize,
    /// Recordings no meeting claims any more, and what they take up.
    pub orphans: usize,
    pub orphan_bytes: i64,
}

/// What one sweep removed, for the Settings card.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RetentionRun {
    pub captures_deleted: usize,
    pub meetings_deleted: usize,
    pub conversations_deleted: usize,
    /// Recordings on disk that no meeting claimed any more.
    pub orphans_deleted: usize,
    pub finished_unix_ms: i64,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// The orphan scan the sweep and the preview share, so Settings never counts a file the
/// sweep would keep. A recording modified in the last seven days is never an orphan: a
/// quit or crash mid-meeting leaves the meeting row without its audio path, and that
/// recording is the only copy of the meeting.
/// Set when the stored recording paths couldn't be pointed at the moved data folder
/// (`state::reroot_stored_paths`). They'd still name the old folder, so the orphan scan
/// would take every recording for an orphan and delete it. Held for the rest of the run.
pub static HOLD_ORPHAN_SCAN: AtomicBool = AtomicBool::new(false);

fn scan_orphans(s: &memory::Store, recordings: &Path, live: &[String]) -> Result<api::retention::OrphanScan, String> {
    if HOLD_ORPHAN_SCAN.load(Ordering::SeqCst) {
        tracing::warn!("not looking for orphaned recordings: their stored paths still name the old data folder");
        return Ok(api::retention::OrphanScan { orphaned: Vec::new(), total_bytes: 0 });
    }
    api::retention::scan_orphans_with_min_age(s, recordings, live, api::retention::ORPHAN_MIN_AGE).map_err(|e| e.to_string())
}

/// Runs one sweep: core deletes the expired rows and their files, then the orphan scan
/// removes recordings nothing points at. `live` are the files a recording is writing now;
/// they are excluded, or a sweep during a meeting would delete the meeting's own audio.
pub fn sweep(store: &StoreHandle, recordings: PathBuf, settings: RetentionSettings, live: Vec<String>) -> Result<RetentionRun, CommandError> {
    let policy = settings.policy();
    let (dir, now) = (recordings.clone(), now_ms());
    let result = store
        .call_blocking(move |s| api::retention::apply(s, &policy, now, &dir).map_err(|e| e.to_string()))?
        .map_err(CommandError::store)?;

    let orphans = store
        .call_blocking(move |s| scan_orphans(s, &recordings, &live))?
        .map_err(CommandError::store)?;
    let orphans_deleted = api::retention::delete_orphans(&orphans.orphaned);

    let run = RetentionRun {
        captures_deleted: result.capture_items_deleted,
        meetings_deleted: result.meetings_deleted,
        conversations_deleted: result.conversations_deleted,
        orphans_deleted,
        finished_unix_ms: now_ms(),
    };
    if run.captures_deleted + run.meetings_deleted + run.conversations_deleted + run.orphans_deleted > 0 {
        tracing::info!(
            captures = run.captures_deleted,
            meetings = run.meetings_deleted,
            conversations = run.conversations_deleted,
            orphans = run.orphans_deleted,
            freed_bytes = orphans.total_bytes,
            "retention swept"
        );
    }
    Ok(run)
}

/// The scheduler and the last run's result, shared with the Settings commands.
pub struct Retention {
    store: StoreHandle,
    settings: Arc<Mutex<SettingsFile>>,
    meetings: Option<Arc<MeetingController>>,
    recordings: PathBuf,
    last_run: Mutex<Option<RetentionRun>>,
}

/// Starts the daily sweep.
pub fn start(
    store: StoreHandle,
    settings: Arc<Mutex<SettingsFile>>,
    meetings: Option<Arc<MeetingController>>,
    recordings: PathBuf,
) -> Arc<Retention> {
    let retention = Arc::new(Retention { store, settings, meetings, recordings, last_run: Mutex::new(None) });
    let loops = retention.clone();
    let spawned = std::thread::Builder::new().name("whimbrel-retention".into()).spawn(move || {
        std::thread::sleep(FIRST_RUN_AFTER);
        loop {
            if let Err(e) = loops.run_scheduled() {
                tracing::warn!(error = %e, "retention sweep failed");
            }
            std::thread::sleep(EVERY);
        }
    });
    if let Err(e) = spawned {
        tracing::error!(error = %e, "could not start retention");
    }
    retention
}

impl Retention {
    pub fn last_run(&self) -> Option<RetentionRun> {
        *self.last_run.lock().unwrap()
    }

    pub fn settings(&self) -> RetentionSettings {
        self.settings.lock().unwrap().retention()
    }

    pub fn acknowledged(&self) -> bool {
        self.settings.lock().unwrap().retention_acknowledged()
    }

    /// One tick of the schedule. Before the first-run notice is acknowledged this counts
    /// and logs but deletes nothing: the windows are defaults nobody has been shown yet,
    /// and the first thing they'd do is delete a month of this PC's history. The notice
    /// is the user's chance to widen them first.
    pub fn run_scheduled(&self) -> Result<(), CommandError> {
        if !self.acknowledged() {
            let preview = self.preview()?;
            tracing::info!(
                captures = preview.captures,
                meetings = preview.meetings,
                conversations = preview.conversations,
                orphans = preview.orphans,
                "retention is holding off until the first-run notice is acknowledged"
            );
            return Ok(());
        }
        self.run_now()?;
        Ok(())
    }

    /// What a sweep would remove right now, without removing it.
    pub fn preview(&self) -> Result<RetentionPreview, CommandError> {
        let policy = self.settings().policy();
        let (dir, now, live) = (self.recordings.clone(), now_ms(), self.live_recording_files());
        let (counts, orphans) = self
            .store
            .call_blocking(move |s| {
                let counts = s.retention_preview(&policy, now).map_err(|e| e.to_string())?;
                Ok::<_, String>((counts, scan_orphans(s, &dir, &live)?))
            })?
            .map_err(CommandError::store)?;
        Ok(RetentionPreview {
            captures: counts.capture_items,
            meetings: counts.meetings,
            conversations: counts.conversations,
            orphans: orphans.orphaned.len(),
            orphan_bytes: orphans.total_bytes,
        })
    }

    /// Sweeps now, keeping the result for Settings. Asking for a sweep also ends the
    /// first-run hold, whether it came from the notice's "Keep current settings" or from
    /// Settings' "Delete old data now": both are someone saying yes to this deletion, so
    /// there is nothing left for the notice to ask.
    pub fn run_now(&self) -> Result<RetentionRun, CommandError> {
        {
            let mut file = self.settings.lock().unwrap();
            if !file.retention_acknowledged() {
                file.set_retention_acknowledged(true);
                file.save().map_err(|e| CommandError::store(format!("could not save settings: {e}")))?;
            }
        }
        let settings = self.settings();
        let live = self.live_recording_files();
        let run = sweep(&self.store, self.recordings.clone(), settings, live)?;
        *self.last_run.lock().unwrap() = Some(run);
        Ok(run)
    }

    /// The WAVs the recorder is writing right now, which the sweep must leave alone.
    fn live_recording_files(&self) -> Vec<String> {
        let Some(stamp) = self.meetings.as_ref().and_then(|m| m.recording_stamp()) else {
            return Vec::new();
        };
        ["system-audio", "mic-audio"]
            .iter()
            .map(|stem| self.recordings.join(format!("{stem}-{stamp}.wav")).to_string_lossy().into_owned())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use memory::{MeetingStatus, MeetingTrigger, NewMeeting};

    const DAY: i64 = 86_400_000;

    fn store_handle() -> StoreHandle {
        StoreHandle::spawn(|| memory::Store::open_in_memory().map_err(|e| e.to_string())).unwrap()
    }

    /// Ages a file past the orphan guard, as last month's leftover recording would be.
    fn backdate(path: &Path) {
        let old = std::time::SystemTime::now() - Duration::from_secs(8 * 86_400);
        std::fs::File::options().write(true).open(path).unwrap().set_modified(old).unwrap();
    }

    #[test]
    fn the_defaults_are_the_macs() {
        let policy = RetentionSettings::default().policy();
        assert_eq!((policy.capture_item_days, policy.meeting_days, policy.conversation_days), (Some(30), Some(90), Some(30)));
    }

    #[test]
    fn keep_forever_is_a_missing_window() {
        let settings = RetentionSettings { capture_days: None, meeting_days: Some(90), conversation_days: None };
        let policy = settings.policy();
        assert_eq!(policy.capture_item_days, None, "nothing expires");
        assert_eq!(policy.meeting_days, Some(90));
    }

    #[test]
    fn a_sweep_deletes_an_expired_meeting_and_its_files_but_keeps_a_live_recording() {
        let dir = std::env::temp_dir().join(format!("whimbrel-retention-{}", std::process::id()));
        let recordings = dir.join("recordings");
        std::fs::create_dir_all(&recordings).unwrap();
        let now = now_ms();

        let old_audio = recordings.join("system-audio-OLD.wav");
        let old_mic = recordings.join("mic-audio-OLD.wav");
        let orphan = recordings.join("system-audio-ORPHAN.wav");
        let live_system = recordings.join("system-audio-LIVE.wav");
        let live_mic = recordings.join("mic-audio-LIVE.wav");
        for path in [&old_audio, &old_mic, &orphan, &live_system, &live_mic] {
            std::fs::write(path, b"audio").unwrap();
        }
        backdate(&orphan);

        let store = store_handle();
        let inserted = {
            let path = old_audio.to_string_lossy().into_owned();
            store
                .call_blocking(move |s| {
                    s.insert_meeting(&NewMeeting {
                        device_id: "this-pc".into(),
                        start_unix_ms: now - 100 * DAY,
                        end_unix_ms: Some(now - 100 * DAY),
                        title: "Old".into(),
                        attendees: Vec::new(),
                        trigger: MeetingTrigger::Manual,
                        audio_path: Some(path),
                        status: MeetingStatus::Completed,
                    })
                    .unwrap()
                })
                .unwrap()
        };

        let live = vec![live_system.to_string_lossy().into_owned(), live_mic.to_string_lossy().into_owned()];
        let run = sweep(&store, recordings.clone(), RetentionSettings::default(), live).unwrap();

        assert_eq!(run.meetings_deleted, 1);
        assert!(!old_audio.exists() && !old_mic.exists(), "the meeting's own files go with it");
        assert_eq!(run.orphans_deleted, 1);
        assert!(!orphan.exists(), "a recording no meeting claims is swept");
        assert!(live_system.exists() && live_mic.exists(), "the recording in progress is left alone");
        assert!(store.call_blocking(move |s| s.get_meeting(inserted).unwrap()).unwrap().is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The whole point of the first-run notice: an upgrade must not delete a month of
    /// this PC's history 30 seconds after launch, before anyone has seen the windows.
    #[test]
    fn a_scheduled_run_deletes_nothing_until_the_notice_is_acknowledged() {
        let dir = std::env::temp_dir().join(format!("whimbrel-retention-hold-{}", std::process::id()));
        let recordings = dir.join("recordings");
        std::fs::create_dir_all(&recordings).unwrap();
        let orphan = recordings.join("system-audio-ORPHAN.wav");
        std::fs::write(&orphan, b"audio").unwrap();
        backdate(&orphan);
        let now = now_ms();

        let store = store_handle();
        store
            .call_blocking(move |s| {
                s.insert_meeting(&NewMeeting {
                    device_id: "this-pc".into(),
                    start_unix_ms: now - 100 * DAY,
                    end_unix_ms: Some(now - 100 * DAY),
                    title: "Old".into(),
                    attendees: Vec::new(),
                    trigger: MeetingTrigger::Manual,
                    audio_path: None,
                    status: MeetingStatus::Completed,
                })
                .unwrap()
            })
            .unwrap();

        let settings = Arc::new(Mutex::new(SettingsFile::load(&dir.join("settings.json"))));
        let retention = Arc::new(Retention {
            store,
            settings: settings.clone(),
            meetings: None,
            recordings: recordings.clone(),
            last_run: Mutex::new(None),
        });

        assert_eq!(retention.preview().unwrap().meetings, 1, "there is something to delete");
        retention.run_scheduled().unwrap();
        assert!(retention.last_run().is_none(), "the schedule held off");
        assert_eq!(retention.preview().unwrap().meetings, 1, "and deleted nothing");
        assert!(orphan.exists(), "not even a leftover recording");

        // What the notice's button does: sweep, and stop asking.
        retention.run_now().unwrap();
        assert!(settings.lock().unwrap().retention_acknowledged());
        retention.run_scheduled().unwrap();
        assert_eq!(retention.preview().unwrap().meetings, 0);
        assert!(!orphan.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A quit or crash mid-meeting leaves the row without its audio path. The recording
    /// it strands must survive the sweep, and Settings must not count it as deletable.
    #[test]
    fn a_recording_stranded_this_week_is_kept_and_not_counted() {
        let dir = std::env::temp_dir().join(format!("whimbrel-retention-stranded-{}", std::process::id()));
        let recordings = dir.join("recordings");
        std::fs::create_dir_all(&recordings).unwrap();
        let stranded = recordings.join("system-audio-STRANDED.wav");
        let leftover = recordings.join("system-audio-LEFTOVER.wav");
        std::fs::write(&stranded, b"audio").unwrap();
        std::fs::write(&leftover, b"audio").unwrap();
        backdate(&leftover);

        let settings = Arc::new(Mutex::new(SettingsFile::load(&dir.join("settings.json"))));
        let retention = Retention {
            store: store_handle(),
            settings,
            meetings: None,
            recordings: recordings.clone(),
            last_run: Mutex::new(None),
        };

        let preview = retention.preview().unwrap();
        assert_eq!((preview.orphans, preview.orphan_bytes), (1, 5), "only the old leftover is deletable");
        assert_eq!(retention.run_now().unwrap().orphans_deleted, 1);
        assert!(stranded.exists(), "this week's stranded recording is kept");
        assert!(!leftover.exists(), "an old recording nothing claims is still swept");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn keeping_everything_forever_sweeps_nothing_but_orphans() {
        let dir = std::env::temp_dir().join(format!("whimbrel-retention-keep-{}", std::process::id()));
        let recordings = dir.join("recordings");
        std::fs::create_dir_all(&recordings).unwrap();
        let now = now_ms();
        let store = store_handle();
        store
            .call_blocking(move |s| {
                s.insert_meeting(&NewMeeting {
                    device_id: "this-pc".into(),
                    start_unix_ms: now - 1_000 * DAY,
                    end_unix_ms: Some(now - 1_000 * DAY),
                    title: "Ancient".into(),
                    attendees: Vec::new(),
                    trigger: MeetingTrigger::Manual,
                    audio_path: None,
                    status: MeetingStatus::Completed,
                })
                .unwrap()
            })
            .unwrap();

        let settings = RetentionSettings { capture_days: None, meeting_days: None, conversation_days: None };
        let run = sweep(&store, recordings, settings, Vec::new()).unwrap();
        assert_eq!((run.meetings_deleted, run.captures_deleted, run.conversations_deleted), (0, 0, 0));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
