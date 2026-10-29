//! Meeting snapshots while a meeting records (FR-MTG-11): every `INTERVAL` the meeting
//! window is grabbed, and each slide shown in it is kept once, as a PNG and a row with its
//! text. The Mac's `MeetingSnapshotService.swift`.
//!
//! [`Session`] is one meeting's tick logic, handed frames so it tests without a screen.
//! [`starter`] runs one on its own below-normal-priority thread per recording, with the
//! real window, face detector and OCR. Nothing here can stop a recording: a failed tick is
//! logged and the next one tries again.

use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Local, NaiveTime, TimeZone};
use memory::NewMeetingSnapshot;

use super::filter::{self, Decision, Features, Luma};
use super::{faces::Faces, storage, target};
use crate::capture::exclusions::Exclusions;
use crate::capture::frame::{Frame, Grabber, Rect};
use crate::capture::{ocr::Ocr, uia::Uia};
use crate::settings::SettingsFile;
use crate::store_actor::StoreHandle;

/// How often the meeting window is sampled, as on the Mac.
pub const INTERVAL: Duration = Duration::from_secs(5);

/// The recording meeting, as the controller hands it over.
#[derive(Debug, Clone)]
pub struct SnapshotMeeting {
    pub meeting_id: i64,
    /// The row's start, which names the meeting's folder.
    pub start_unix_ms: i64,
    pub title: String,
    /// The meeting app's process tree, if the meeting has one.
    pub root_pid: Option<u32>,
}

/// Starts a recording meeting's snapshots; `None` if they couldn't start.
pub type StartSnapshots = Arc<dyn Fn(SnapshotMeeting) -> Option<SnapshotHandle> + Send + Sync>;

/// What a stop and a tick share. A tick holds `stopped` while it records a kept slide, so
/// once `stop` returns no new slide lands on the meeting.
#[derive(Default)]
struct Stop {
    stopped: Mutex<bool>,
    wake: Condvar,
}

/// A running meeting's snapshots. Dropping it doesn't stop them; `stop` does.
pub struct SnapshotHandle {
    stop: Arc<Stop>,
}

impl SnapshotHandle {
    /// Stops at once, without waiting for a tick in flight: that tick keeps nothing.
    pub fn stop(&self) {
        *self.stop.stopped.lock().unwrap() = true;
        self.stop.wake.notify_all();
    }
}

#[cfg(test)]
impl SnapshotHandle {
    /// A handle with no worker behind it, and a probe for whether it has been stopped.
    pub(crate) fn unwatched() -> (Self, impl Fn() -> bool) {
        let stop = Arc::new(Stop::default());
        let probe = stop.clone();
        (SnapshotHandle { stop }, move || *probe.stopped.lock().unwrap())
    }
}

/// What [`starter`] needs besides the meeting.
pub struct Deps {
    pub store: StoreHandle,
    pub settings: Arc<Mutex<SettingsFile>>,
    /// Where folders go when the chosen root can't be written.
    pub fallback_root: PathBuf,
    /// Detection's live call app, for a meeting with no process tree of its own.
    pub candidate: Arc<dyn Fn() -> Option<u32> + Send + Sync>,
}

/// One worker thread per recording meeting. The setting is read every tick, so turning
/// snapshots off mid-meeting pauses them and turning it back on resumes them.
pub fn starter(deps: Deps) -> StartSnapshots {
    let deps = Arc::new(deps);
    Arc::new(move |meeting| {
        let stop = Arc::new(Stop::default());
        let root = storage::effective_root(deps.settings.lock().unwrap().meeting_snapshots_root(), &deps.fallback_root);
        let (deps, thread_stop, meeting_id) = (deps.clone(), stop.clone(), meeting.meeting_id);
        let spawned = std::thread::Builder::new()
            .name("whimbrel-meeting-snapshots".into())
            .spawn(move || run(meeting, &deps, root, thread_stop));
        match spawned {
            Ok(_) => Some(SnapshotHandle { stop }),
            Err(e) => {
                tracing::error!(meeting_id, error = %e, "could not start meeting snapshots");
                None
            }
        }
    })
}

fn run(meeting: SnapshotMeeting, deps: &Deps, root: PathBuf, stop: Arc<Stop>) {
    use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL};
    // The meeting app and the recording come first.
    let _ = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL) };
    let meeting_id = meeting.meeting_id;
    // Created on this thread: the D3D device and UIA are used only here.
    let ocr = Ocr::new().map_err(|e| tracing::warn!(error = %e, "no OCR: slides are kept without their text")).ok();
    let own_grabber = match ocr {
        Some(_) => None,
        None => Grabber::new().map_err(|e| tracing::error!(meeting_id, error = %e, "no screen capture: no snapshots")).ok(),
    };
    let Some(grabber) = ocr.as_ref().map(Ocr::grabber).or(own_grabber.as_ref()) else { return };
    let faces = Faces::new().map_err(|e| tracing::warn!(error = %e, "no face detector: crops ignore webcam tiles")).ok();
    let uia = Uia::new().map_err(|e| tracing::warn!(error = %e, "no UIA: browser meetings get no snapshots")).ok();

    let mut session = Session::new(
        meeting.clone(),
        deps.store.clone(),
        root,
        deps.fallback_root.clone(),
        stop.clone(),
        Box::new(|frame: &Frame| faces.as_ref().and_then(|f| f.detect(frame).ok()).unwrap_or_default()),
        Box::new(|frame: &Frame| ocr.as_ref().and_then(|o| o.recognize(frame).ok()).unwrap_or_default()),
    );
    tracing::info!(meeting_id, "meeting snapshots running");
    let mut next = Instant::now() + INTERVAL;
    while !wait_until(&stop, next) {
        // A tick that overran skips the ticks it ran into.
        while next <= Instant::now() {
            next += INTERVAL;
        }
        let (enabled, exclusions) = {
            let settings = deps.settings.lock().unwrap();
            (settings.meeting_snapshots_enabled(), Exclusions { user: settings.excluded_bundle_ids() })
        };
        if !enabled {
            continue;
        }
        let ticked = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let hwnd = target::meeting_window(meeting.root_pid.or_else(|| (deps.candidate)()), uia.as_ref(), &exclusions);
            let frame = hwnd.and_then(|h| grabber.grab_window(h).map_err(|e| tracing::debug!(error = %e, "no meeting frame")).ok());
            session.tick(frame, Local::now())
        }));
        match ticked {
            Ok(Ok(Decision::KeepNew)) => tracing::info!(meeting_id, "kept a slide"),
            Ok(Ok(Decision::ReplaceLast)) => tracing::info!(meeting_id, "replaced the last slide with its build"),
            Ok(Ok(_)) => {}
            Ok(Err(e)) => tracing::warn!(meeting_id, error = %e, "snapshot not saved"),
            Err(_) => tracing::error!(meeting_id, "a snapshot tick panicked"),
        }
    }
    tracing::info!(meeting_id, kept = session.kept, "meeting snapshots stopped");
}

/// Sleeps until `deadline`; `true` if stopped meanwhile.
fn wait_until(stop: &Stop, deadline: Instant) -> bool {
    let mut stopped = stop.stopped.lock().unwrap();
    while !*stopped {
        let now = Instant::now();
        if now >= deadline {
            return false;
        }
        stopped = stop.wake.wait_timeout(stopped, deadline - now).unwrap().0;
    }
    true
}

/// One frame's analysis. `found` is `None` for junk and participant-grid frames.
#[derive(Clone)]
struct Analysis {
    width: u32,
    height: u32,
    luma: Luma,
    found: Option<(Features, Rect)>,
}

struct Kept {
    features: Features,
    id: i64,
    path: PathBuf,
    /// The slide's first appearance, which names its file even after a build replaces it.
    captured: NaiveTime,
}

/// The face detector and OCR a session uses, injected so tests can stand in for them.
type FaceFn<'a> = Box<dyn FnMut(&Frame) -> Vec<Rect> + 'a>;
type TextFn<'a> = Box<dyn FnMut(&Frame) -> String + 'a>;

/// One meeting's snapshots: what it has seen and kept so far.
pub struct Session<'a> {
    meeting: SnapshotMeeting,
    store: StoreHandle,
    root: PathBuf,
    fallback: PathBuf,
    stop: Arc<Stop>,
    faces: FaceFn<'a>,
    text: TextFn<'a>,
    previous_sample: Option<Features>,
    last_analysis: Option<Analysis>,
    last_kept: Option<Kept>,
    /// Made on the first kept slide, so a meeting that shows none gets no folder.
    folder: Option<PathBuf>,
    kept: usize,
}

impl<'a> Session<'a> {
    fn new(
        meeting: SnapshotMeeting,
        store: StoreHandle,
        root: PathBuf,
        fallback: PathBuf,
        stop: Arc<Stop>,
        faces: FaceFn<'a>,
        text: TextFn<'a>,
    ) -> Self {
        Session {
            meeting,
            store,
            root,
            fallback,
            stop,
            faces,
            text,
            previous_sample: None,
            last_analysis: None,
            last_kept: None,
            folder: None,
            kept: 0,
        }
    }

    fn stopped(&self) -> bool {
        *self.stop.stopped.lock().unwrap()
    }

    /// One tick, with the frame grabbed for it (`None`: nothing to capture this time).
    /// Returns what the filter decided; junk, grids and missing frames are `Skip`.
    pub fn tick(&mut self, frame: Option<Frame>, now: DateTime<Local>) -> Result<Decision, String> {
        let Some(frame) = frame.filter(|_| !self.stopped()) else { return Ok(Decision::Skip) };
        let Some(analysis) = self.analyze(&frame) else { return Ok(Decision::Skip) };
        self.last_analysis = Some(analysis.clone());
        let Some((current, crop)) = analysis.found else { return Ok(Decision::Skip) };
        let Some(image) = frame.crop(crop) else { return Ok(Decision::Skip) };

        let decision = filter::decide(&current, self.previous_sample.as_ref(), self.last_kept.as_ref().map(|k| &k.features));
        self.previous_sample = Some(current.clone());
        match decision {
            Decision::KeepNew => self.keep(&image, current, now)?,
            Decision::ReplaceLast => self.replace_last(&image, current)?,
            Decision::Skip | Decision::WaitForStable => {}
        }
        Ok(decision)
    }

    /// Junk check, face-aware crop, then the cropped frame's hash and text. A frame that
    /// hasn't changed since the last analysed one reuses its result: no faces, no OCR,
    /// which is what keeps a static slide cheap. `None` only for an empty frame.
    fn analyze(&mut self, frame: &Frame) -> Option<Analysis> {
        let luma = Luma::thumbnail(frame)?;
        // Compared with the analysed frame, not the last tick's, so slow drift still gets
        // a fresh look. Same pixel size too: a same-aspect resize matches the thumbnail
        // but not the crop.
        if let Some(previous) = &self.last_analysis {
            if (previous.width, previous.height) == (frame.width, frame.height)
                && previous.luma.matches(&luma, filter::UNCHANGED_PIXEL_STEP)
            {
                return Some(previous.clone());
            }
        }
        let mut found = None;
        if !filter::is_junk(frame) {
            let faces = (self.faces)(frame);
            if let Some((crop, image)) = filter::content_crop(frame, &faces).and_then(|c| Some((c, frame.crop(c)?))) {
                found = Some((Features { hash: filter::dhash(&image), text: (self.text)(&image) }, crop));
            }
        }
        Some(Analysis { width: frame.width, height: frame.height, luma, found })
    }

    fn keep(&mut self, image: &Frame, features: Features, now: DateTime<Local>) -> Result<(), String> {
        let folder = match &self.folder {
            Some(folder) => folder.clone(),
            None => {
                let start = Local.timestamp_millis_opt(self.meeting.start_unix_ms).single().unwrap_or(now).naive_local();
                let made = storage::make_meeting_folder(&self.root, &self.fallback, start, &self.meeting.title)
                    .map_err(|e| format!("snapshot folder: {e}"))?;
                self.folder = Some(made.clone());
                made
            }
        };
        let path = storage::write_png(image, &folder, now.time()).map_err(|e| format!("snapshot file: {e}"))?;
        let meeting_id = self.meeting.meeting_id;
        let (dir, row) = (
            folder.to_string_lossy().into_owned(),
            NewMeetingSnapshot {
                meeting_id,
                captured_unix_ms: now.timestamp_millis(),
                file_path: path.to_string_lossy().into_owned(),
                width: image.width as i64,
                height: image.height as i64,
                ocr_text: features.text.clone(),
            },
        );
        let inserted = {
            let stopped = self.stop.stopped.lock().unwrap();
            if *stopped {
                drop(stopped);
                self.discard(&path);
                return Ok(());
            }
            self.store.call_blocking(move |s| -> Result<Option<i64>, String> {
                s.set_meeting_snapshots_dir(meeting_id, &dir).map_err(|e| e.to_string())?;
                s.insert_meeting_snapshot(&row).map_err(|e| e.to_string())
            })
        };
        match inserted.map_err(|e| e.to_string()).and_then(|r| r) {
            Ok(Some(id)) => {
                self.last_kept = Some(Kept { features, id, path, captured: now.time() });
                self.kept += 1;
                Ok(())
            }
            Ok(None) => {
                self.discard(&path);
                Err("the meeting's row is gone".into())
            }
            Err(e) => {
                self.discard(&path);
                Err(e)
            }
        }
    }

    fn replace_last(&mut self, image: &Frame, features: Features) -> Result<(), String> {
        let (Some(last), Some(folder)) = (&self.last_kept, &self.folder) else { return Ok(()) };
        let (id, old, captured) = (last.id, last.path.clone(), last.captured);
        // Named for the slide's first appearance, so the folder stays in meeting order.
        let path = storage::write_png(image, folder, captured).map_err(|e| format!("snapshot file: {e}"))?;
        let (file, width, height, text) = (path.to_string_lossy().into_owned(), image.width as i64, image.height as i64, features.text.clone());
        let replaced = {
            let stopped = self.stop.stopped.lock().unwrap();
            if *stopped {
                drop(stopped);
                self.discard(&path);
                return Ok(());
            }
            self.store.call_blocking(move |s| s.replace_meeting_snapshot(id, &file, width, height, &text).map_err(|e| e.to_string()))
        };
        match replaced.map_err(|e| e.to_string()).and_then(|r| r) {
            Ok(true) => {
                let _ = std::fs::remove_file(&old);
                self.last_kept = Some(Kept { features, id, path, captured });
                Ok(())
            }
            Ok(false) => {
                self.discard(&path);
                Err("the slide's row is gone".into())
            }
            Err(e) => {
                self.discard(&path);
                Err(e)
            }
        }
    }

    /// Removes a file that won't be recorded, and the folder too if nothing was recorded
    /// in it (core removes only a folder that is all Whimbrel's own).
    fn discard(&mut self, path: &Path) {
        let _ = std::fs::remove_file(path);
        if self.last_kept.is_none() {
            if let Some(folder) = self.folder.take() {
                api::retention::delete_snapshots_dir(&folder);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meetings::snapshots::fixtures as fx;
    use memory::{MeetingStatus, MeetingTrigger, NewMeeting};

    const ROADMAP: [&str; 4] = ["Roadmap", "Ship meeting snapshots", "Calendar sync for Outlook", "Faster summaries"];

    struct Rig {
        store: StoreHandle,
        meeting_id: i64,
        root: PathBuf,
        stop: Arc<Stop>,
        ocr: Ocr,
    }

    fn rig(name: &str) -> Rig {
        let root = std::env::temp_dir().join(format!("whimbrel-snapservice-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let store = StoreHandle::spawn(|| memory::Store::open_in_memory().map_err(|e| e.to_string())).unwrap();
        let meeting_id = store
            .call_blocking(|s| {
                s.insert_meeting(&NewMeeting {
                    device_id: "this-pc".into(),
                    start_unix_ms: 1_761_660_000_000,
                    end_unix_ms: None,
                    title: "Acme sync".into(),
                    attendees: Vec::new(),
                    trigger: MeetingTrigger::Manual,
                    audio_path: None,
                    status: MeetingStatus::Recording,
                })
                .unwrap()
            })
            .unwrap();
        Rig { store, meeting_id, root, stop: Arc::new(Stop::default()), ocr: Ocr::new().expect("the Windows OCR engine") }
    }

    impl Rig {
        /// A session whose face detector reports `faces` for every frame.
        fn session(&self, faces: Vec<Rect>) -> Session<'_> {
            let meeting =
                SnapshotMeeting { meeting_id: self.meeting_id, start_unix_ms: 1_761_660_000_000, title: "Acme sync".into(), root_pid: None };
            Session::new(
                meeting,
                self.store.clone(),
                self.root.clone(),
                self.root.join("fallback"),
                self.stop.clone(),
                Box::new(move |_: &Frame| faces.clone()),
                Box::new(|frame: &Frame| self.ocr.recognize(frame).unwrap()),
            )
        }

        fn rows(&self) -> Vec<memory::MeetingSnapshot> {
            let id = self.meeting_id;
            self.store.call_blocking(move |s| s.list_meeting_snapshots(id).unwrap()).unwrap()
        }

        /// Every PNG under the root.
        fn pngs(&self) -> Vec<PathBuf> {
            let Ok(folders) = std::fs::read_dir(&self.root) else { return Vec::new() };
            let mut found: Vec<PathBuf> = folders
                .flatten()
                .filter(|f| f.path().is_dir())
                .flat_map(|f| std::fs::read_dir(f.path()).unwrap().flatten().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "png"))
                .collect();
            found.sort();
            found
        }

        fn folders(&self) -> usize {
            std::fs::read_dir(&self.root).map(|d| d.flatten().filter(|f| f.path().is_dir()).count()).unwrap_or(0)
        }
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    /// Ticks 5 s apart from 10:14:00 local.
    fn at(tick: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 10, 28, 10, 14, 0).unwrap() + chrono::Duration::seconds(5 * tick as i64)
    }

    #[test]
    fn a_stable_slide_over_four_ticks_keeps_one_file() {
        let rig = rig("stable");
        let mut session = rig.session(Vec::new());
        let decisions: Vec<Decision> = (0..4).map(|t| session.tick(Some(fx::slide(&ROADMAP)), at(t)).unwrap()).collect();
        assert_eq!(decisions, [Decision::WaitForStable, Decision::KeepNew, Decision::Skip, Decision::Skip]);
        let rows = rig.rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rig.pngs(), vec![PathBuf::from(&rows[0].file_path)]);
        assert!(rows[0].ocr_text.contains("Roadmap"), "{}", rows[0].ocr_text);
        assert_eq!(rows[0].captured_unix_ms, at(1).timestamp_millis());
        let dir = rig.store.call_blocking({
            let id = rig.meeting_id;
            move |s| s.meeting_snapshots_info(id).unwrap().unwrap().0
        });
        assert_eq!(dir.unwrap().map(PathBuf::from), Some(PathBuf::from(&rows[0].file_path).parent().unwrap().to_path_buf()));
    }

    #[test]
    fn a_build_replaces_the_previous_file() {
        let rig = rig("build");
        let mut session = rig.session(Vec::new());
        for t in 0..2 {
            session.tick(Some(fx::slide(&ROADMAP[..2])), at(t)).unwrap();
        }
        let first = rig.pngs();
        assert_eq!(first.len(), 1);
        assert_eq!(session.tick(Some(fx::slide(&ROADMAP[..3])), at(2)).unwrap(), Decision::WaitForStable);
        assert_eq!(session.tick(Some(fx::slide(&ROADMAP[..3])), at(3)).unwrap(), Decision::ReplaceLast);

        let (rows, pngs) = (rig.rows(), rig.pngs());
        assert_eq!(rows.len(), 1, "the build replaced the row");
        assert_eq!(pngs, vec![PathBuf::from(&rows[0].file_path)], "and its file");
        assert!(!first[0].exists(), "the old file is gone");
        assert!(rows[0].ocr_text.contains("Calendar"), "{}", rows[0].ocr_text);
        assert_eq!(rows[0].captured_unix_ms, at(1).timestamp_millis(), "the slide keeps its first appearance");
        assert!(pngs[0].file_name().unwrap().to_string_lossy().starts_with("10.14.05"), "{}", pngs[0].display());
    }

    #[test]
    fn a_new_slide_after_the_first_keeps_a_second_file() {
        let rig = rig("two");
        let mut session = rig.session(Vec::new());
        for t in 0..2 {
            session.tick(Some(fx::slide(&ROADMAP)), at(t)).unwrap();
        }
        let hiring = ["Hiring plan", "Two engineers in Q1", "One designer in Q2"];
        for t in 2..4 {
            session.tick(Some(fx::slide(&hiring)), at(t)).unwrap();
        }
        assert_eq!(rig.rows().len(), 2);
        assert_eq!(rig.pngs().len(), 2);
        assert_eq!(rig.folders(), 1, "one folder per meeting");
    }

    #[test]
    fn face_grids_and_blank_frames_create_no_folder() {
        let rig = rig("grid");
        let (grid, faces) = fx::face_grid(3, 2);
        let mut session = rig.session(faces);
        for t in 0..3 {
            session.tick(Some(grid.clone()), at(t)).unwrap();
        }
        let mut blank = rig.session(Vec::new());
        for t in 0..3 {
            blank.tick(Some(fx::solid(0.1)), at(t)).unwrap();
        }
        assert_eq!(rig.rows().len(), 0);
        assert_eq!(rig.folders(), 0);
    }

    #[test]
    fn a_missing_frame_skips_the_tick_without_ending_the_session() {
        let rig = rig("failure");
        let mut session = rig.session(Vec::new());
        assert_eq!(session.tick(None, at(0)).unwrap(), Decision::Skip);
        session.tick(Some(fx::slide(&ROADMAP)), at(1)).unwrap();
        assert_eq!(session.tick(None, at(2)).unwrap(), Decision::Skip);
        assert_eq!(session.tick(Some(fx::slide(&ROADMAP)), at(3)).unwrap(), Decision::KeepNew, "the gap didn't reset stability");
        assert_eq!(rig.rows().len(), 1);
    }

    #[test]
    fn nothing_is_kept_after_stop() {
        let rig = rig("stopped");
        let mut session = rig.session(Vec::new());
        session.tick(Some(fx::slide(&ROADMAP)), at(0)).unwrap();
        SnapshotHandle { stop: rig.stop.clone() }.stop();
        for t in 1..4 {
            assert_eq!(session.tick(Some(fx::slide(&ROADMAP)), at(t)).unwrap(), Decision::Skip);
        }
        assert_eq!(rig.rows().len(), 0);
        assert_eq!(rig.folders(), 0);
    }

    #[test]
    fn a_stop_during_a_keep_removes_the_file_and_the_empty_folder() {
        let rig = rig("midkeep");
        let mut session = rig.session(Vec::new());
        session.tick(Some(fx::slide(&ROADMAP)), at(0)).unwrap();
        // Past the tick's own stop check, as a stop landing while the PNG is written.
        let image = fx::slide(&ROADMAP);
        *rig.stop.stopped.lock().unwrap() = true;
        session.keep(&image, Features { hash: 0, text: "Roadmap".into() }, at(1)).unwrap();
        assert_eq!(rig.rows().len(), 0);
        assert_eq!(rig.folders(), 0, "the folder made for it went too");
    }

    #[test]
    fn an_unchanged_frame_is_not_analysed_again() {
        let rig = rig("unchanged");
        let reads = std::cell::Cell::new(0);
        let meeting = SnapshotMeeting { meeting_id: rig.meeting_id, start_unix_ms: 0, title: "M".into(), root_pid: None };
        let mut session = Session::new(
            meeting,
            rig.store.clone(),
            rig.root.clone(),
            rig.root.clone(),
            rig.stop.clone(),
            Box::new(|_: &Frame| Vec::new()),
            Box::new(|frame: &Frame| {
                reads.set(reads.get() + 1);
                rig.ocr.recognize(frame).unwrap()
            }),
        );
        for t in 0..4 {
            session.tick(Some(fx::slide(&ROADMAP)), at(t)).unwrap();
        }
        drop(session);
        assert_eq!(reads.get(), 1, "OCR ran once for four identical frames");
    }

    /// `cargo test --lib -- --ignored --nocapture snapshots::service::tests::tick_cost`, run
    /// alone (it reads the whole process's CPU time): what a tick costs at 1080p and 4K, for
    /// the plan's as-built note.
    #[test]
    #[ignore = "a measurement, not a check"]
    fn tick_cost() {
        use crate::capture::{proc_stats, window};
        let cpu_ms = || proc_stats::cpu_time_100ns(std::process::id()).unwrap() as f64 / 10_000.0;
        let faces = Faces::new().unwrap();
        for (w, h) in [(1920, 1080), (3840, 2160)] {
            let rig = rig(&format!("cost-{w}"));
            let meeting = SnapshotMeeting { meeting_id: rig.meeting_id, start_unix_ms: 0, title: "M".into(), root_pid: None };
            let mut session = Session::new(
                meeting,
                rig.store.clone(),
                rig.root.clone(),
                rig.root.clone(),
                rig.stop.clone(),
                Box::new(|f: &Frame| faces.detect(f).unwrap()),
                Box::new(|f: &Frame| rig.ocr.recognize(f).unwrap()),
            );
            let frame = fx::slide(&ROADMAP).downscaled(w, h);
            let mut costs = Vec::new();
            for t in 0..3 {
                let (c, wall) = (cpu_ms(), Instant::now());
                let decision = session.tick(Some(frame.clone()), at(t)).unwrap();
                costs.push(format!("{decision:?} {:.0} ms CPU / {:.0} ms", cpu_ms() - c, wall.elapsed().as_secs_f64() * 1000.0));
            }
            println!("{w}x{h}: {}", costs.join(", "));
        }
        let (c, wall) = (cpu_ms(), Instant::now());
        let windows = window::top_level_windows();
        println!("{} windows listed: {:.0} ms CPU / {:.0} ms", windows.len(), cpu_ms() - c, wall.elapsed().as_secs_f64() * 1000.0);
        let grabber = Grabber::new().unwrap();
        let biggest = windows.iter().filter(|w| !w.minimized && !window::is_cloaked(w.hwnd)).max_by_key(|w| window::area(w.hwnd)).unwrap();
        for _ in 0..2 {
            let (c, wall) = (cpu_ms(), Instant::now());
            let frame = grabber.grab_window(biggest.hwnd).unwrap();
            println!("grab {}x{}: {:.0} ms CPU / {:.0} ms", frame.width, frame.height, cpu_ms() - c, wall.elapsed().as_secs_f64() * 1000.0);
        }
    }

    #[test]
    fn a_stopped_worker_wakes_at_once() {
        let stop = Arc::new(Stop::default());
        let waiter = {
            let stop = stop.clone();
            std::thread::spawn(move || {
                let t = Instant::now();
                (wait_until(&stop, Instant::now() + Duration::from_secs(30)), t.elapsed())
            })
        };
        std::thread::sleep(Duration::from_millis(50));
        SnapshotHandle { stop }.stop();
        let (stopped, waited) = waiter.join().unwrap();
        assert!(stopped);
        assert!(waited < Duration::from_secs(5), "{waited:?}");
    }
}
