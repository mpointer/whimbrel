//! The low-disk notice (plan 2026-11-12-storage-visibility): when the data folder's drive has
//! under 10 GB free, a small popup says so at most once a day, and its button opens Settings
//! where Storage and Free up space are. Recording is never blocked. A popup, not a toast, for the
//! reason in `alert.rs`.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};

use crate::l10n::tr;
use crate::paths::Paths;
use crate::storage::{self, LowDisk};

pub const DISK_ALERT: &str = "disk-alert";
const SIZE: (f64, f64) = (380.0, 170.0);
const MARGIN: f64 = 12.0;
/// The first look, a minute after launch, then hourly; the popup itself at most daily.
const FIRST_CHECK: Duration = Duration::from_secs(60);
const CHECK_EVERY: Duration = Duration::from_secs(60 * 60);
pub const AT_MOST_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

struct Shared {
    pending: Vec<LowDisk>,
    last_shown: Option<Instant>,
}

fn shared() -> &'static Mutex<Shared> {
    static SHARED: OnceLock<Mutex<Shared>> = OnceLock::new();
    SHARED.get_or_init(|| Mutex::new(Shared { pending: Vec::new(), last_shown: None }))
}

/// Whether a low disk may be shown now: never shown, or last shown a day or more ago.
pub fn due(last_shown: Option<Instant>, now: Instant) -> bool {
    last_shown.is_none_or(|t| now.saturating_duration_since(t) >= AT_MOST_EVERY)
}

/// One look: queues the low drives and opens the popup when it's due.
fn check(app: &AppHandle, paths: &Paths) {
    let low = storage::low_disks(&[paths.root.as_path(), paths.recordings.as_path()], storage::volume_free_bytes, storage::LOW_DISK_BYTES);
    if low.is_empty() {
        return;
    }
    for d in &low {
        tracing::warn!(volume = %d.volume, free_bytes = d.free_bytes, "disk is almost full");
    }
    let now = Instant::now();
    {
        let mut s = shared().lock().unwrap();
        if !due(s.last_shown, now) {
            return;
        }
        s.pending = low;
        s.last_shown = Some(now);
    }
    if app.get_webview_window(DISK_ALERT).is_none() {
        if let Err(e) = open(app) {
            tracing::error!(error = %e, "could not open the low-disk notice");
        }
    }
}

/// Starts the hourly look.
pub fn watch(app: AppHandle, paths: Paths) {
    let spawned = std::thread::Builder::new().name("whimbrel-disk-alert".into()).spawn(move || {
        std::thread::sleep(FIRST_CHECK);
        loop {
            check(&app, &paths);
            std::thread::sleep(CHECK_EVERY);
        }
    });
    if let Err(e) = spawned {
        tracing::error!(error = %e, "could not start the low-disk watcher");
    }
}

fn hide(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(DISK_ALERT) {
        let _ = window.destroy();
    }
}

fn open(app: &AppHandle) -> tauri::Result<()> {
    let (w, h) = SIZE;
    let popup = WebviewWindowBuilder::new(app, DISK_ALERT, WebviewUrl::App("index.html".into()))
        .title(tr("Your disk is almost full", &[]))
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
        // Below whichever alerts are up.
        for (label, height) in [(crate::alert::ALERT, 190.0), (crate::question_alert::QUESTION_ALERT, 210.0), (crate::budget_alert::BUDGET_ALERT, 170.0)] {
            if app.get_webview_window(label).is_some() {
                y += ((height + MARGIN) * scale).round() as i32;
            }
        }
        popup.set_position(PhysicalPosition::new(x, y))?;
    }
    popup.show()?;
    Ok(())
}

/// The drives the notice is about.
#[tauri::command]
pub fn disk_alert_status() -> Vec<LowDisk> {
    shared().lock().unwrap().pending.clone()
}

/// Free up space: the main window opens Settings at Capture, where Storage is.
#[tauri::command]
pub fn disk_alert_open(app: AppHandle) {
    shared().lock().unwrap().pending.clear();
    hide(&app);
    crate::app::show_main_window(&app);
    let _ = app.emit_to(crate::app::MAIN_WINDOW, crate::budget_alert::OPEN_SETTINGS_EVENT, "capture");
}

#[tauri::command]
pub fn disk_alert_dismiss(app: AppHandle) {
    shared().lock().unwrap().pending.clear();
    hide(&app);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shows_at_most_once_a_day() {
        let now = Instant::now();
        assert!(due(None, now));
        assert!(!due(Some(now), now + Duration::from_secs(60 * 60)));
        assert!(due(Some(now), now + AT_MOST_EVERY));
    }
}
