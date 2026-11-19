//! The in-meeting window (docs/ui-redesign/screens/live-meeting.md), redesign only: a small always-on-top window with the
//! meeting's title, state, recorded time, level meter, Pause or Resume, Stop and an ask box. Built like `still_here`: its own
//! label, no decorations, not in the taskbar, one UI bundle that picks its view by label.
//!
//! It opens when a recording starts and is destroyed when it stops, only with the redesign switch on. The person can drag it
//! (the page has a drag region), it remembers its place in a device-local setting, and closing it hides it for that recording
//! only; "Show in-meeting window" in the tray menu brings it back. Closing it never stops the recording.
//! The app's own screen capture skips every window of this process (`capture::exclusions`, `pid == own_pid`), and meeting
//! snapshots grab only the meeting app's window, so this window is in neither.

use std::sync::atomic::{AtomicI64, Ordering};

use tauri::{AppHandle, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder, WindowEvent};

use crate::l10n::tr;
use crate::state::AppState;

/// The window's label (also listed in `capabilities/default.json`).
pub const MEETING_BAR: &str = "meeting-bar";
const SIZE: (f64, f64) = (320.0, 260.0);
const MARGIN: f64 = 16.0;
/// How much of the window must be on a screen for a remembered place to be used, in pixels each way.
const MIN_VISIBLE: i32 = 80;

/// Whether the window is hidden from other people in a shared screen (Windows' window content protection: the window is
/// left out of a capture or share, as the Mac's is). On, as the owner decided on 2026-11-18: the ask box holds private text.
pub const HIDE_FROM_SCREEN_SHARING: bool = true;

/// The recording the person closed the window for (its start time), 0 for none.
static CLOSED_FOR: AtomicI64 = AtomicI64::new(0);

/// "x,y" for the setting.
pub fn format_position(x: i32, y: i32) -> String {
    format!("{x},{y}")
}

/// Reads [`format_position`]; anything else is `None`.
pub fn parse_position(text: &str) -> Option<(i32, i32)> {
    let (x, y) = text.split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

/// Whether a window of `size` at `at` has enough on one of the `areas` (x, y, width, height) to be reached, so a place saved on a
/// screen that is gone is not used.
pub fn is_reachable(at: (i32, i32), size: (u32, u32), areas: &[(i32, i32, u32, u32)]) -> bool {
    areas.iter().any(|&(ax, ay, aw, ah)| {
        let across = (at.0 + size.0 as i32).min(ax + aw as i32) - at.0.max(ax);
        let down = (at.1 + size.1 as i32).min(ay + ah as i32) - at.1.max(ay);
        across >= MIN_VISIBLE && down >= MIN_VISIBLE
    })
}

/// Whether the window should exist now: a recording runs, the switch is on, and the person did not close it for this recording.
pub fn wanted(redesign: bool, recording_started: Option<i64>, closed_for: i64) -> bool {
    redesign && recording_started.is_some_and(|started| started != closed_for)
}

/// Opens or destroys the window to match the recording. Called after every recording change (`tray::refresh`); cheap when nothing
/// needs doing, and the window work runs on its own thread (building a window on the main thread inside a command can hang).
pub fn sync(app: &AppHandle, recording_started: Option<i64>) {
    let redesign = app.state::<AppState>().settings.lock().unwrap().redesign_enabled();
    if !redesign && app.get_webview_window(MEETING_BAR).is_none() {
        return;
    }
    let want = wanted(redesign, recording_started, CLOSED_FOR.load(Ordering::Relaxed));
    let exists = app.get_webview_window(MEETING_BAR).is_some();
    if want == exists {
        return;
    }
    let app = app.clone();
    let spawned = std::thread::Builder::new().name("whimbrel-meeting-bar".into()).spawn(move || {
        if want {
            if let Err(e) = open(&app) {
                tracing::error!(error = %e, "could not open the in-meeting window");
            }
        } else {
            hide(&app);
        }
    });
    if let Err(e) = spawned {
        tracing::error!(error = %e, "could not start the in-meeting window's thread");
    }
}

/// "Show in-meeting window" from the tray: forgets a close and opens the window if a meeting records.
pub fn show_from_tray(app: &AppHandle) {
    CLOSED_FOR.store(0, Ordering::Relaxed);
    let started = app.state::<AppState>().meetings().ok().and_then(|m| m.status().started_unix_ms);
    sync(app, started);
}

/// Destroys the window, remembering where it was.
pub fn hide(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MEETING_BAR) {
        remember(app, &window);
        let _ = window.destroy();
    }
}

fn remember(app: &AppHandle, window: &tauri::WebviewWindow) {
    let Ok(position) = window.outer_position() else { return };
    let state = app.state::<AppState>();
    let mut settings = state.settings.lock().unwrap();
    settings.set_meeting_bar_position(&format_position(position.x, position.y));
    if let Err(e) = settings.save() {
        tracing::warn!(error = %e, "could not remember the in-meeting window's place");
    }
}

fn open(app: &AppHandle) -> tauri::Result<()> {
    let (w, h) = SIZE;
    let bar = WebviewWindowBuilder::new(app, MEETING_BAR, WebviewUrl::App("index.html".into()))
        .title(tr("Whimbrel, in-meeting window", &[]))
        .inner_size(w, h)
        .decorations(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .content_protected(HIDE_FROM_SCREEN_SHARING)
        .focused(false)
        .visible(false)
        .build()?;
    // Closing it is for this recording only.
    let handle = app.clone();
    bar.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { .. } = event {
            if let Some(started) = handle.state::<AppState>().meetings().ok().and_then(|m| m.status().started_unix_ms) {
                CLOSED_FOR.store(started, Ordering::Relaxed);
            }
            if let Some(window) = handle.get_webview_window(MEETING_BAR) {
                remember(&handle, &window);
            }
        }
    });
    if let Some(monitor) = bar.primary_monitor()? {
        let scale = monitor.scale_factor();
        let size = ((w * scale).round() as u32, (h * scale).round() as u32);
        let area = monitor.work_area();
        let areas: Vec<(i32, i32, u32, u32)> = app
            .available_monitors()
            .unwrap_or_default()
            .iter()
            .map(|m| {
                let a = m.work_area();
                (a.position.x, a.position.y, a.size.width, a.size.height)
            })
            .collect();
        let saved = app
            .state::<AppState>()
            .settings
            .lock()
            .unwrap()
            .meeting_bar_position()
            .and_then(|p| parse_position(&p))
            .filter(|&at| is_reachable(at, size, &areas));
        let at = saved.unwrap_or_else(|| {
            crate::alert::alert_origin(size, (area.position.x, area.position.y, area.size.width, area.size.height), (MARGIN * scale).round() as i32)
        });
        bar.set_position(PhysicalPosition::new(at.0, at.1))?;
    }
    bar.show()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_place_round_trips_and_junk_is_none() {
        assert_eq!(parse_position(&format_position(-1200, 80)), Some((-1200, 80)));
        assert_eq!(parse_position("12, 34"), Some((12, 34)));
        assert_eq!(parse_position("12"), None);
        assert_eq!(parse_position("a,b"), None);
    }

    #[test]
    fn a_remembered_place_is_used_only_when_it_can_still_be_reached() {
        let areas = [(0, 0, 1920, 1040)];
        assert!(is_reachable((1500, 20), (320, 260), &areas));
        assert!(!is_reachable((1850, 20), (320, 260), &areas), "only 70 px would show");
        assert!(!is_reachable((-3000, 20), (320, 260), &areas), "its screen was unplugged");
        assert!(is_reachable((-3000, 20), (320, 260), &[(-3840, 0, 1920, 1040), (0, 0, 1920, 1040)]));
        assert!(!is_reachable((100, 2000), (320, 260), &areas));
    }

    #[test]
    fn the_window_follows_the_recording_unless_closed_for_it_or_the_switch_is_off() {
        assert!(wanted(true, Some(5_000), 0));
        assert!(!wanted(false, Some(5_000), 0), "never with the switch off");
        assert!(!wanted(true, None, 0), "nothing records");
        assert!(!wanted(true, Some(5_000), 5_000), "closed for this recording");
        assert!(wanted(true, Some(9_000), 5_000), "a later recording opens it again");
    }
}
