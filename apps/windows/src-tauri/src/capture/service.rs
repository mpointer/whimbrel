//! The ambient capture service (plan Task 3): reads the foreground window's text and
//! stores it as `kind = 'capture'` items with this device's id.
//!
//! One thread owns the UIA client (and, lazily, the OCR engine); a second pumps the
//! system notifications in `events.rs`. Rows go into the store through the store actor
//! and `ingest::ingest_record`, the same normalize → chunk → dedupe path the Mac's
//! ingest uses. The cadence rules live in `schedule.rs`, pure and tested.

use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use schemars::JsonSchema;
use serde::Serialize;

use crate::capture::events::{self, SystemEvent};
use crate::capture::exclusions::{ExcludeReason, Exclusions};
use crate::capture::schedule::{is_chromium_host, Cadence, ChangeFilter, OcrLimiter, Reason, Scheduler, WindowKey};
use crate::capture::session_gate::{idle_duration, timed_pause_active, PauseReason, SessionGate};
use crate::capture::{app_id, merge, ocr, policy, uia, window};
use crate::error::CommandError;
use crate::store_actor::StoreHandle;

/// What Settings shows about capture.
#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CaptureStatus {
    pub enabled: bool,
    /// False if the service could not start (see `last_error`).
    pub running: bool,
    /// Why capture is paused right now, if it is.
    pub paused: Option<String>,
    /// Reads that produced new stored items.
    pub stored: u64,
    /// Reads skipped as near-identical to the window's last stored text.
    pub unchanged: u64,
    /// Reads skipped because the window is excluded.
    pub excluded: u64,
    pub ocr_runs: u64,
    pub last_stored_at_ms: Option<i64>,
    pub last_app: Option<String>,
    pub last_error: Option<String>,
    /// How screen-text reading (OCR) last went wrong: "unavailable" (it could not start this session) or "failed" (a window's picture could not be read). Cleared when an OCR read works again.
    pub ocr_problem: Option<String>,
    /// When `ocr_problem` was recorded (unix ms).
    pub ocr_problem_at_unix_ms: Option<i64>,
    /// When a timed pause ends (unix ms). Set only while one runs; `enabled` stays true during it.
    pub resume_at_unix_ms: Option<i64>,
}

/// How long a "failed" OCR read counts as current when no later OCR read has worked: a window that needed OCR failed, and
/// nothing has needed it since. After this the problem is history, not news.
pub const OCR_PROBLEM_FRESH_MS: i64 = 10 * 60 * 1000;

/// What is wrong with screen-text reading now, in a form the tray and the popup can say in plain words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadProblem {
    /// OCR could not start (no engine, or no screen grabber): it stays off until Whimbrel restarts.
    Unavailable,
    /// The last OCR read of a window failed, recently.
    Failed,
}

impl CaptureStatus {
    /// Records that OCR could not start (`kind` "unavailable") or that a read failed ("failed").
    pub(crate) fn note_ocr_problem(&mut self, kind: &str, now_ms: i64) {
        self.ocr_problem = Some(kind.to_string());
        self.ocr_problem_at_unix_ms = Some(now_ms);
    }

    /// Records that an OCR read worked: a "failed" problem is over. "unavailable" is not: OCR cannot have run.
    pub(crate) fn note_ocr_worked(&mut self) {
        if self.ocr_problem.as_deref() == Some("failed") {
            self.ocr_problem = None;
            self.ocr_problem_at_unix_ms = None;
        }
    }

    /// The OCR problem that is current at `now_ms`. Same rule as the popup's `ocrProblem` (ui/src/popup/popupModel.ts).
    pub fn read_problem(&self, now_ms: i64) -> Option<ReadProblem> {
        match self.ocr_problem.as_deref() {
            Some("unavailable") => Some(ReadProblem::Unavailable),
            Some("failed") if self.ocr_problem_at_unix_ms.is_some_and(|at| now_ms - at < OCR_PROBLEM_FRESH_MS) => Some(ReadProblem::Failed),
            _ => None,
        }
    }
}

enum Msg {
    System(SystemEvent),
    SetEnabled(bool),
    SetResumeAt(Option<i64>),
    SetUserExclusions(Vec<String>),
}

pub struct CaptureHandle {
    tx: Sender<Msg>,
    status: Arc<Mutex<CaptureStatus>>,
}

impl CaptureHandle {
    pub fn status(&self) -> CaptureStatus {
        self.status.lock().unwrap().clone()
    }

    pub fn set_enabled(&self, enabled: bool) {
        {
            let mut st = self.status.lock().unwrap();
            st.enabled = enabled;
            // Turning capture off ends any timed pause: off wins, and nothing is left to expire.
            if !enabled {
                st.resume_at_unix_ms = None;
            }
        }
        let _ = self.tx.send(Msg::SetEnabled(enabled));
    }

    /// Starts, replaces (`Some`) or ends (`None`) a timed pause. It never changes `enabled`.
    pub fn set_resume_at(&self, resume_at_unix_ms: Option<i64>) {
        self.status.lock().unwrap().resume_at_unix_ms = resume_at_unix_ms;
        let _ = self.tx.send(Msg::SetResumeAt(resume_at_unix_ms));
    }

    pub fn set_user_exclusions(&self, user: Vec<String>) {
        let _ = self.tx.send(Msg::SetUserExclusions(user));
    }
}

/// Starts the service. It runs for the life of the process; `enabled = false` idles
/// it without tearing down the hooks.
pub fn start(
    store: StoreHandle,
    device_id: String,
    enabled: bool,
    resume_at_unix_ms: Option<i64>,
    user_exclusions: Vec<String>,
) -> Result<CaptureHandle, String> {
    let (tx, rx) = mpsc::channel::<Msg>();
    let status = Arc::new(Mutex::new(CaptureStatus { enabled, running: true, resume_at_unix_ms, ..Default::default() }));
    let event_tx = tx.clone();
    events::start(move |e| {
        let _ = event_tx.send(Msg::System(e));
    })?;
    let loop_status = status.clone();
    std::thread::Builder::new()
        .name("whimbrel-capture".into())
        .spawn(move || {
            let mut service = match Service::new(store, device_id, enabled, resume_at_unix_ms, user_exclusions, loop_status.clone()) {
                Ok(s) => s,
                Err(e) => {
                    let mut st = loop_status.lock().unwrap();
                    st.running = false;
                    st.last_error = Some(e);
                    return;
                }
            };
            service.run(rx);
        })
        .map_err(|e| e.to_string())?;
    Ok(CaptureHandle { tx, status })
}

fn now_unix_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

struct Service {
    store: StoreHandle,
    device_id: String,
    enabled: bool,
    /// The end of a timed pause (unix ms). While it is ahead, nothing is read; `enabled` is untouched.
    resume_at_ms: Option<i64>,
    exclusions: Exclusions,
    status: Arc<Mutex<CaptureStatus>>,
    uia: uia::Uia,
    ocr: Option<ocr::Ocr>,
    ocr_unavailable: bool,
    scheduler: Scheduler,
    ocr_limiter: OcrLimiter,
    changes: ChangeFilter,
    gate: SessionGate,
    own_pid: u32,
}

impl Service {
    fn new(
        store: StoreHandle,
        device_id: String,
        enabled: bool,
        resume_at_ms: Option<i64>,
        user_exclusions: Vec<String>,
        status: Arc<Mutex<CaptureStatus>>,
    ) -> Result<Self, String> {
        let cadence = Cadence::default();
        Ok(Service {
            store,
            device_id,
            enabled,
            resume_at_ms,
            exclusions: Exclusions { user: user_exclusions },
            status,
            uia: uia::Uia::new().map_err(|e| format!("UI Automation: {e}"))?,
            ocr: None,
            ocr_unavailable: false,
            scheduler: Scheduler::new(cadence),
            ocr_limiter: OcrLimiter::new(cadence.ocr_min_interval),
            changes: ChangeFilter::default(),
            gate: SessionGate::default(),
            own_pid: std::process::id(),
        })
    }

    fn run(&mut self, rx: mpsc::Receiver<Msg>) {
        tracing::info!(enabled = self.enabled, "capture service running");
        loop {
            match rx.recv_timeout(self.scheduler.next_wake(Instant::now())) {
                Ok(msg) => {
                    self.handle(msg);
                    while let Ok(msg) = rx.try_recv() {
                        self.handle(msg);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            // A timed pause ends when the wall clock reaches its deadline, checked on every pass (a timer would
            // drift across sleep). Only the pause ends; `enabled` is whatever the person set.
            let now_ms = now_unix_ms();
            if self.resume_at_ms.is_some() && !timed_pause_active(self.resume_at_ms, now_ms) {
                tracing::info!("capture timed pause ended");
                let ended = self.resume_at_ms.take();
                let mut st = self.status.lock().unwrap();
                // Only if nobody has set a newer deadline since: that message is still waiting in the queue.
                if st.resume_at_unix_ms == ended {
                    st.resume_at_unix_ms = None;
                }
            }
            let pause = if !self.enabled {
                Some(PauseReason::Disabled)
            } else if timed_pause_active(self.resume_at_ms, now_ms) {
                Some(PauseReason::TimedPause)
            } else {
                self.gate.pause_reason(idle_duration())
            };
            self.status.lock().unwrap().paused = pause.map(|p| p.describe().to_string());
            if pause.is_some() {
                continue;
            }
            let now = Instant::now();
            if let Some(reason) = self.scheduler.take_due(now) {
                self.read_foreground(now, reason);
            }
        }
    }

    fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::System(SystemEvent::Focus) => self.scheduler.on_trigger(Instant::now()),
            Msg::System(SystemEvent::SessionLocked(locked)) => self.gate.locked = locked,
            Msg::System(SystemEvent::DisplayOn(on)) => self.gate.display_off = !on,
            Msg::System(SystemEvent::Suspending(s)) => self.gate.suspended = s,
            Msg::SetEnabled(enabled) => {
                tracing::info!(enabled, "capture toggled");
                self.enabled = enabled;
                if !enabled {
                    self.resume_at_ms = None;
                }
            }
            Msg::SetResumeAt(resume_at) => {
                tracing::info!(?resume_at, "capture timed pause set");
                self.resume_at_ms = resume_at;
            }
            Msg::SetUserExclusions(user) => self.exclusions.user = user,
        }
    }

    fn read_foreground(&mut self, now: Instant, reason: Reason) {
        let Some(w) = window::foreground() else {
            self.scheduler.read(now, None, false);
            return;
        };
        let app = app_id::normalize(&w.app_id, &w.exe);
        let key = WindowKey { hwnd: w.hwnd.0 as isize, app_id: app.clone() };
        let chromium = is_chromium_host(&w.exe);
        self.scheduler.read(now, Some(&key), chromium);

        if let Some(why) = self.exclusions.check(&w.exe, &app, &w.title, w.pid, self.own_pid) {
            if why != ExcludeReason::Whimbrel {
                self.status.lock().unwrap().excluded += 1;
            }
            return;
        }
        if w.minimized {
            return;
        }

        let pattern = self.uia.extract(w.hwnd, uia::Strategy::TextPattern);
        let walk = self.uia.extract(w.hwnd, uia::Strategy::CachedWalk);
        let (text_pattern, walk_text) = (
            pattern.as_ref().map(|e| e.text.as_str()).unwrap_or(""),
            walk.as_ref().map(|e| e.text.as_str()).unwrap_or(""),
        );
        let document_found = pattern.as_ref().is_ok_and(|e| e.document_found) || walk.as_ref().is_ok_and(|e| e.document_found);
        let mut text = merge::union_text(text_pattern, walk_text);
        let mut source = "uia";
        let uia_hash = ingest::simhash(&ingest::normalize(&text));

        let wants_ocr = policy::should_ocr(&policy::FallbackInput {
            app_id: &app,
            exe: &w.exe,
            uia_chars: uia::count_chars(&text),
            document_found,
        });
        if wants_ocr && !self.ocr_unavailable && self.ocr_limiter.allow(&key, now, reason, uia_hash) {
            if self.ocr.is_none() {
                match ocr::Ocr::new() {
                    Ok(o) => self.ocr = Some(o),
                    Err(e) => {
                        self.ocr_unavailable = true;
                        let mut st = self.status.lock().unwrap();
                        st.last_error = Some(format!("OCR unavailable: {e}"));
                        st.note_ocr_problem("unavailable", now_unix_ms());
                    }
                }
            }
            if let Some(o) = &self.ocr {
                self.ocr_limiter.ran(&key, now, uia_hash);
                match o.recognize_window(w.hwnd) {
                    Ok(out) if uia::count_chars(&out.text) >= policy::MAC_MIN_CHARS => {
                        text = out.text;
                        source = "ocr";
                        let mut st = self.status.lock().unwrap();
                        st.ocr_runs += 1;
                        st.note_ocr_worked();
                    }
                    Ok(_) => {
                        // The engine ran and found little text: it worked.
                        let mut st = self.status.lock().unwrap();
                        st.ocr_runs += 1;
                        st.note_ocr_worked();
                    }
                    Err(e) => {
                        let mut st = self.status.lock().unwrap();
                        st.last_error = Some(format!("OCR: {e}"));
                        st.note_ocr_problem("failed", now_unix_ms());
                    }
                }
            }
        }

        if uia::count_chars(&text) < policy::MAC_MIN_CHARS {
            return;
        }
        let normalized = ingest::normalize(&text);
        if !self.changes.is_new(&key, ingest::simhash(&normalized)) {
            self.status.lock().unwrap().unchanged += 1;
            return;
        }

        let ts_unix_ms = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
        let record = ingest::CaptureRecord {
            ts_unix_ms,
            bundle_id: app.clone(),
            window_title: w.title.clone(),
            url: None,
            text,
            source: source.to_string(),
        };
        let device_id = self.device_id.clone();
        // The closure's own `Result` (a store error from the insert) joins the actor's.
        let stored = self
            .store
            .call_blocking(move |s| ingest::ingest_record(s, &device_id, &record))
            .and_then(|r| r.map_err(|e| CommandError::store(e.to_string())));
        match stored {
            Ok(stats) => {
                let mut st = self.status.lock().unwrap();
                if stats.items_inserted > 0 {
                    st.stored += 1;
                    st.last_stored_at_ms = Some(ts_unix_ms);
                    st.last_app = Some(app);
                }
            }
            Err(e) => {
                // Retried at the window's next read, which the change filter would otherwise
                // skip as unchanged (issue 850 review, I5: a busy store dropped the line).
                self.changes.forget(&key);
                tracing::warn!(error = %e, "capture insert failed");
                self.status.lock().unwrap().last_error = Some("The capture could not be stored; retrying.".to_string());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_read_is_current_until_one_works_or_it_goes_stale() {
        let mut st = CaptureStatus::default();
        assert_eq!(st.read_problem(1_000), None);
        st.note_ocr_problem("failed", 1_000);
        assert_eq!(st.read_problem(1_000 + OCR_PROBLEM_FRESH_MS - 1), Some(ReadProblem::Failed));
        assert_eq!(st.read_problem(1_000 + OCR_PROBLEM_FRESH_MS), None, "old news is not shown");
        st.note_ocr_worked();
        assert_eq!(st.read_problem(1_001), None);
        assert_eq!(st.ocr_problem_at_unix_ms, None);
    }

    #[test]
    fn unavailable_stays_until_restart_and_a_working_read_cannot_clear_it() {
        let mut st = CaptureStatus::default();
        st.note_ocr_problem("unavailable", 5);
        st.note_ocr_worked();
        assert_eq!(st.read_problem(5 + 100 * OCR_PROBLEM_FRESH_MS), Some(ReadProblem::Unavailable));
    }

    #[test]
    fn an_unknown_kind_is_no_problem() {
        let mut st = CaptureStatus::default();
        st.note_ocr_problem("something else", 1);
        assert_eq!(st.read_problem(2), None);
    }
}
