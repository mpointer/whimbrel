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
use crate::capture::session_gate::{idle_duration, PauseReason, SessionGate};
use crate::capture::{app_id, merge, ocr, policy, uia, window};
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
}

enum Msg {
    System(SystemEvent),
    SetEnabled(bool),
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
        self.status.lock().unwrap().enabled = enabled;
        let _ = self.tx.send(Msg::SetEnabled(enabled));
    }

    pub fn set_user_exclusions(&self, user: Vec<String>) {
        let _ = self.tx.send(Msg::SetUserExclusions(user));
    }
}

/// Starts the service. It runs for the life of the process; `enabled = false` idles
/// it without tearing down the hooks.
pub fn start(store: StoreHandle, device_id: String, enabled: bool, user_exclusions: Vec<String>) -> Result<CaptureHandle, String> {
    let (tx, rx) = mpsc::channel::<Msg>();
    let status = Arc::new(Mutex::new(CaptureStatus { enabled, running: true, ..Default::default() }));
    let event_tx = tx.clone();
    events::start(move |e| {
        let _ = event_tx.send(Msg::System(e));
    })?;
    let loop_status = status.clone();
    std::thread::Builder::new()
        .name("whimbrel-capture".into())
        .spawn(move || {
            let mut service = match Service::new(store, device_id, enabled, user_exclusions, loop_status.clone()) {
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

struct Service {
    store: StoreHandle,
    device_id: String,
    enabled: bool,
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
        user_exclusions: Vec<String>,
        status: Arc<Mutex<CaptureStatus>>,
    ) -> Result<Self, String> {
        let cadence = Cadence::default();
        Ok(Service {
            store,
            device_id,
            enabled,
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
            let pause = if self.enabled { self.gate.pause_reason(idle_duration()) } else { Some(PauseReason::Disabled) };
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
                        self.status.lock().unwrap().last_error = Some(format!("OCR unavailable: {e}"));
                    }
                }
            }
            if let Some(o) = &self.ocr {
                self.ocr_limiter.ran(&key, now, uia_hash);
                match o.recognize_window(w.hwnd) {
                    Ok(out) if uia::count_chars(&out.text) >= policy::MAC_MIN_CHARS => {
                        text = out.text;
                        source = "ocr";
                        self.status.lock().unwrap().ocr_runs += 1;
                    }
                    Ok(_) => self.status.lock().unwrap().ocr_runs += 1,
                    Err(e) => self.status.lock().unwrap().last_error = Some(format!("OCR: {e}")),
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
        match self.store.call_blocking(move |s| ingest::ingest_record(s, &device_id, &record)) {
            Ok(stats) => {
                let mut st = self.status.lock().unwrap();
                if stats.items_inserted > 0 {
                    st.stored += 1;
                    st.last_stored_at_ms = Some(ts_unix_ms);
                    st.last_app = Some(app);
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "capture insert failed");
                self.status.lock().unwrap().last_error = Some(e.to_string());
            }
        }
    }
}
