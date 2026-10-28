//! The "meeting detected" alert (M3.3 Decision 10): a small window at the top-right of
//! the primary monitor's work area.
//!
//! Not a toast: toasts from an unpackaged app need an AppUserModelID and a Start Menu
//! shortcut, which the M3.5 installer creates. Unlike the tray popup, this window must
//! survive losing focus — the user is in a call when it appears — so it closes only on a
//! button, on the countdown, or when the call is gone.

use tauri::{AppHandle, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};

use crate::error::CommandError;
use crate::meetings::service::{alert_view, AlertView};
use crate::state::AppState;

/// The alert window's label (also listed in `capabilities/default.json`).
pub const ALERT: &str = "alert";
/// Its size in logical pixels.
const SIZE: (f64, f64) = (360.0, 190.0);
/// The gap from the work area's top-right corner.
const MARGIN: f64 = 12.0;

/// Where the alert sits, in physical pixels: the work area's top-right corner, inset by
/// `margin`. Pure, so the placement is testable without a monitor.
pub fn alert_origin(size: (u32, u32), work_area: (i32, i32, u32, u32), margin: i32) -> (i32, i32) {
    let (area_x, area_y, area_w, _) = work_area;
    let x = area_x + area_w as i32 - size.0 as i32 - margin;
    (x.max(area_x + margin), area_y + margin)
}

/// Opens the alert if it isn't already up.
pub fn show(app: &AppHandle) {
    if app.get_webview_window(ALERT).is_some() {
        return;
    }
    if let Err(e) = open(app) {
        tracing::error!(error = %e, "could not open the meeting alert");
    }
}

/// Closes it, however it was answered.
pub fn hide(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(ALERT) {
        let _ = window.destroy();
    }
}

fn open(app: &AppHandle) -> tauri::Result<()> {
    let (w, h) = SIZE;
    let alert = WebviewWindowBuilder::new(app, ALERT, WebviewUrl::App("index.html".into()))
        .title("Meeting detected")
        .inner_size(w, h)
        .decorations(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .visible(false)
        .build()?;
    if let Some(monitor) = alert.primary_monitor()? {
        let scale = monitor.scale_factor();
        let area = monitor.work_area();
        let (x, y) = alert_origin(
            ((w * scale).round() as u32, (h * scale).round() as u32),
            (area.position.x, area.position.y, area.size.width, area.size.height),
            (MARGIN * scale).round() as i32,
        );
        alert.set_position(PhysicalPosition::new(x, y))?;
    }
    alert.show()?;
    Ok(())
}

/// What the alert shows: the detected call and its countdown.
#[tauri::command]
pub fn alert_status(state: tauri::State<'_, AppState>) -> AlertView {
    let settings = state.settings.lock().unwrap();
    alert_view(&state.detected, &settings, chrono::Utc::now().timestamp_millis())
}

/// Accept: records the detected call. `open_app` also brings the main window up.
#[tauri::command]
pub async fn alert_accept(app: AppHandle, open_app: bool) -> Result<(), CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let Some(detected) = state.detected.take() else {
            crate::alert::hide(&app);
            return Ok(());
        };
        crate::alert::hide(&app);
        let meetings = state.meetings()?;
        let started = if detected.source == "calendar" {
            // Record the call's app when detection has spotted one; otherwise everything
            // the PC plays, which the workspace notes (docs/12 §4.4).
            meetings.start_calendar(detected.title.clone(), detected.attendees.clone(), detected.start_unix_ms, state.detected.candidate_pid())
        } else {
            meetings.start_detected(detected.title.clone(), detected.root_pid)
        };
        crate::tray::refresh(&app);
        if open_app {
            crate::app::show_main_window(&app);
        }
        started.map(|_| ())
    })
    .await
    .map_err(|e| CommandError::store(format!("accepting the meeting failed: {e}")))?
}

/// Dismiss: nothing is written, as on the Mac. The detector won't offer the same app
/// again until it stops matching.
#[tauri::command]
pub fn alert_dismiss(app: AppHandle, state: tauri::State<'_, AppState>) {
    if let Some(detected) = state.detected.take() {
        tracing::info!(id = %detected.id, "meeting prompt dismissed");
    }
    hide(&app);
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: (i32, i32, u32, u32) = (0, 0, 1920, 1032);

    #[test]
    fn the_alert_sits_in_the_work_areas_top_right_corner() {
        assert_eq!(alert_origin((360, 190), WORK, 12), (1548, 12));
    }

    #[test]
    fn it_follows_a_second_monitors_offset_work_area() {
        assert_eq!(alert_origin((360, 190), (-1920, 100, 1920, 1000), 12), (-372, 112));
    }

    #[test]
    fn a_narrow_work_area_keeps_it_on_screen() {
        assert_eq!(alert_origin((360, 190), (0, 0, 300, 800), 12), (12, 12));
    }
}
