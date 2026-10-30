//! The tray (plan Task 10). Its icon follows capture and sync state, and turns red while a
//! meeting records (M3.3 Task 3). Left-click toggles a quick-glance popup, and right-click
//! shows the menu.
//!
//! The popup is a small undecorated window. Like the main window, it is destroyed rather
//! than hidden when it loses focus, so no second WebView2 renderer idles all day
//! (docs/12 W2).

use std::sync::Mutex;
use std::time::{Duration, Instant};

use schemars::JsonSchema;
use serde::Serialize;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, PhysicalPosition, Rect, WebviewUrl, WebviewWindowBuilder, WindowEvent};

use crate::state::AppState;
use crate::syncing::controller::SyncState;

pub const TRAY_ID: &str = "whimbrel";
/// The popup window's label (also listed in `capabilities/default.json`).
pub const POPUP: &str = "popup";
/// The popup's size, in logical pixels.
const POPUP_SIZE: (f64, f64) = (300.0, 228.0);
/// The gap between the popup and the taskbar, in logical pixels.
const POPUP_MARGIN: f64 = 8.0;
/// How often the icon and tooltip are recomputed: two status reads and one registry
/// read. The icon is replaced only when it changes.
const REFRESH_EVERY: Duration = Duration::from_secs(3);
/// A click this soon after the popup closed on focus loss is the click that closed it.
const REOPEN_GUARD: Duration = Duration::from_millis(400);
/// Focus lost this soon after opening is the tray click finishing, not the user leaving.
const OPEN_GRACE: Duration = Duration::from_millis(600);

static POPUP_CLOSED_AT: Mutex<Option<Instant>> = Mutex::new(None);
/// What the tray shows now: (icon, light taskbar, status line).
static SHOWN: Mutex<Option<(IconState, bool, String)>> = Mutex::new(None);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureLine {
    On,
    /// Enabled, but paused for now (e.g. the screen is locked).
    Paused,
    Off,
    /// Enabled, but the service couldn't start.
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncLine {
    Off,
    SettingUp,
    /// When content last synced, if it has this session.
    Synced(Option<i64>),
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconState {
    /// A meeting is recording.
    Recording,
    Idle,
    Paused,
    SyncError,
}

/// The popup's view of the tray state.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TrayStatus {
    /// "Capturing · Synced 2:05 PM", also the tray tooltip.
    pub line: String,
    pub capture_enabled: bool,
    /// When the recording meeting started, if one is recording.
    pub recording_started_unix_ms: Option<i64>,
}

pub fn sync_line(state: SyncState, last_synced_ms: Option<i64>, content_error: bool) -> SyncLine {
    match state {
        SyncState::NotConfigured | SyncState::NotSignedIn => SyncLine::Off,
        SyncState::SigningIn | SyncState::WaitingForApproval | SyncState::WaitingForGroupKey => SyncLine::SettingUp,
        SyncState::Ready if content_error => SyncLine::Error,
        SyncState::Ready => SyncLine::Synced(last_synced_ms),
        SyncState::Error => SyncLine::Error,
    }
}

/// Recording wins: it's the one state the user must never miss. A sync error needs
/// attention, so it wins over paused capture.
pub fn icon_state(capture: CaptureLine, sync: SyncLine, recording: bool) -> IconState {
    if recording {
        return IconState::Recording;
    }
    match (capture, sync) {
        (_, SyncLine::Error) => IconState::SyncError,
        (CaptureLine::On, _) => IconState::Idle,
        _ => IconState::Paused,
    }
}

pub fn status_line(capture: CaptureLine, sync: SyncLine, time: impl Fn(i64) -> String) -> String {
    let capture = match capture {
        CaptureLine::On => "Capturing",
        CaptureLine::Paused => "Capture paused",
        CaptureLine::Off => "Capture off",
        CaptureLine::Stopped => "Capture stopped",
    };
    let sync = match sync {
        SyncLine::Off => "Sync off".to_string(),
        SyncLine::SettingUp => "Sync setting up".to_string(),
        SyncLine::Synced(Some(ms)) => format!("Synced {}", time(ms)),
        SyncLine::Synced(None) => "Sync on".to_string(),
        SyncLine::Error => "Sync error".to_string(),
    };
    format!("{capture} · {sync}")
}

/// Where the popup's top-left corner goes, in physical pixels. It is centered over the
/// tray icon, against the taskbar's edge of the work area (bottom or top), and kept inside
/// the work area. `icon` and `work` are (x, y, width, height).
pub fn popup_origin(icon: (i32, i32, u32, u32), popup: (u32, u32), work: (i32, i32, u32, u32), margin: i32) -> (i32, i32) {
    let (ix, iy, iw, ih) = (icon.0, icon.1, icon.2 as i32, icon.3 as i32);
    let (pw, ph) = (popup.0 as i32, popup.1 as i32);
    let (wx, wy, ww, wh) = (work.0, work.1, work.2 as i32, work.3 as i32);
    let min_x = wx + margin;
    let x = (ix + iw / 2 - pw / 2).clamp(min_x, (wx + ww - pw - margin).max(min_x));
    let y = if iy + ih / 2 >= wy + wh / 2 { wy + wh - ph - margin } else { wy + margin };
    (x, y)
}

fn current(state: &AppState) -> (CaptureLine, SyncLine) {
    let capture = match &state.capture {
        Some(c) => {
            let s = c.status();
            if !s.enabled {
                CaptureLine::Off
            } else if !s.running {
                CaptureLine::Stopped
            } else if s.paused.is_some() {
                CaptureLine::Paused
            } else {
                CaptureLine::On
            }
        }
        None => CaptureLine::Stopped,
    };
    let sync = match &state.sync {
        Some(c) => {
            let v = c.view();
            sync_line(v.state, v.content_last_synced_ms, v.content_error.is_some())
        }
        None => SyncLine::Off,
    };
    (capture, sync)
}

fn clock_time(ms: i64) -> String {
    use chrono::TimeZone;
    chrono::Local.timestamp_millis_opt(ms).single().map(|t| t.format("%-I:%M %p").to_string()).unwrap_or_default()
}

/// Whether the taskbar uses the light theme (`SystemUsesLightTheme`). Unset means dark,
/// the Windows 11 default.
fn taskbar_is_light() -> bool {
    use windows::core::w;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    let mut value: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    result.is_ok() && value == 1
}

/// The rendered icons (`apps/windows/scripts/render-icons.mjs`, from `design/tray-glyph.svg`). The Mac's glyph is a
/// template image the OS tints; Windows has none, so the glyph follows the taskbar theme.
fn icon_bytes(icon: IconState, light_taskbar: bool) -> &'static [u8] {
    match (icon, light_taskbar) {
        (IconState::Idle, true) => &include_bytes!("../icons/tray/idle-light.ico")[..],
        (IconState::Idle, false) => &include_bytes!("../icons/tray/idle-dark.ico")[..],
        (IconState::Paused, true) => &include_bytes!("../icons/tray/paused-light.ico")[..],
        (IconState::Paused, false) => &include_bytes!("../icons/tray/paused-dark.ico")[..],
        (IconState::SyncError, _) => &include_bytes!("../icons/tray/sync-error.ico")[..],
        (IconState::Recording, _) => &include_bytes!("../icons/tray/recording.ico")[..],
    }
}

/// Recomputes the tray state and updates the icon and tooltip if they changed.
pub fn refresh(app: &AppHandle) -> TrayStatus {
    let state = app.state::<AppState>();
    let (capture, sync) = current(&state);
    let recording_started_unix_ms = state.meetings.as_ref().and_then(|m| m.status().started_unix_ms);
    let line = status_line(capture, sync, clock_time);
    let tooltip = match recording_started_unix_ms {
        Some(ms) => format!("Whimbrel · Recording since {} · {line}", clock_time(ms)),
        None => format!("Whimbrel · {line}"),
    };
    let shown = (icon_state(capture, sync, recording_started_unix_ms.is_some()), taskbar_is_light(), tooltip.clone());
    // Decide under the lock, but never call the tray setters holding it. Off the main
    // thread they dispatch to the main thread and wait; a command running `refresh` on the
    // main thread would then wait for this lock, and neither returns (M3.3 Task 3's hang).
    let changed = {
        let mut last = SHOWN.lock().unwrap();
        let changed = last.as_ref() != Some(&shown);
        if changed {
            *last = Some(shown.clone());
        }
        changed
    };
    if changed {
        if let Some(tray) = app.tray_by_id(TRAY_ID) {
            match Image::from_bytes(icon_bytes(shown.0, shown.1)) {
                Ok(image) => {
                    let _ = tray.set_icon(Some(image));
                }
                Err(e) => tracing::error!("could not load a tray icon: {e}"),
            }
            let _ = tray.set_tooltip(Some(tooltip));
        }
    }
    TrayStatus { line, capture_enabled: capture != CaptureLine::Off, recording_started_unix_ms }
}

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Whimbrel", true, None::<&str>)?;
    let feedback = MenuItem::with_id(app, "feedback", "Send Feedback…", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Whimbrel", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &feedback, &quit])?;
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(Image::from_bytes(icon_bytes(IconState::Idle, taskbar_is_light()))?)
        .tooltip("Whimbrel")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => crate::app::show_main_window(app),
            "feedback" => {
                if let Err(e) = crate::feedback::open() {
                    tracing::warn!(error = %e, "send feedback from the tray");
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, rect, .. } = event {
                toggle_popup(tray.app_handle(), rect);
            }
        })
        .build(app)?;
    refresh(app);
    let handle = app.clone();
    std::thread::Builder::new().name("whimbrel-tray".into()).spawn(move || loop {
        std::thread::sleep(REFRESH_EVERY);
        refresh(&handle);
    })?;
    Ok(())
}

fn toggle_popup(app: &AppHandle, anchor: Rect) {
    if let Some(popup) = app.get_webview_window(POPUP) {
        let _ = popup.destroy();
        return;
    }
    if POPUP_CLOSED_AT.lock().unwrap().is_some_and(|t| t.elapsed() < REOPEN_GUARD) {
        return;
    }
    if let Err(e) = open_popup(app, anchor) {
        tracing::error!("could not open the tray popup: {e}");
    }
}

fn open_popup(app: &AppHandle, anchor: Rect) -> tauri::Result<()> {
    let (w, h) = POPUP_SIZE;
    let popup = WebviewWindowBuilder::new(app, POPUP, WebviewUrl::App("index.html".into()))
        .title("Whimbrel")
        .inner_size(w, h)
        .decorations(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .build()?;
    // The tray reports physical pixels; the monitor's scale turns the logical popup size
    // and margin into them.
    let icon_pos: PhysicalPosition<i32> = anchor.position.to_physical(1.0);
    let icon_size = anchor.size.to_physical::<u32>(1.0);
    if let Some(monitor) = app.monitor_from_point(icon_pos.x as f64, icon_pos.y as f64)? {
        let scale = monitor.scale_factor();
        let area = monitor.work_area();
        let (x, y) = popup_origin(
            (icon_pos.x, icon_pos.y, icon_size.width, icon_size.height),
            ((w * scale).round() as u32, (h * scale).round() as u32),
            (area.position.x, area.position.y, area.size.width, area.size.height),
            (POPUP_MARGIN * scale).round() as i32,
        );
        popup.set_position(PhysicalPosition::new(x, y))?;
    }
    let closer = popup.clone();
    let opened = Instant::now();
    popup.on_window_event(move |event| {
        if let WindowEvent::Focused(focused) = *event {
            let elapsed_ms = opened.elapsed().as_millis() as u64;
            tracing::info!(focused, elapsed_ms, "tray popup focus");
            if focused {
                return;
            }
            if opened.elapsed() < OPEN_GRACE {
                // The shell takes the foreground back as the tray click finishes, just
                // after the popup appears; take it again rather than close at once.
                let _ = closer.set_focus();
                return;
            }
            *POPUP_CLOSED_AT.lock().unwrap() = Some(Instant::now());
            let _ = closer.destroy();
        }
    });
    popup.show()?;
    popup.set_focus()?;
    Ok(())
}

/// The popup's status. It also updates the tray icon at once (after the popup toggles
/// capture, say) rather than on the next tick. Async, so it runs off the main thread: the
/// tray setters wait on the main thread.
#[tauri::command]
pub async fn tray_status(app: AppHandle) -> Result<TrayStatus, crate::error::CommandError> {
    tauri::async_runtime::spawn_blocking(move || refresh(&app))
        .await
        .map_err(|e| crate::error::CommandError::store(format!("tray status failed: {e}")))
}

/// Opens the main window; the popup closes as it loses focus.
#[tauri::command]
pub fn popup_open_main(app: AppHandle) {
    crate::app::show_main_window(&app);
}

#[tauri::command]
pub fn popup_quit(app: AppHandle) {
    app.exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: (i32, i32, u32, u32) = (0, 0, 1920, 1032);
    const POPUP_PX: (u32, u32) = (300, 184);

    #[test]
    fn the_line_reads_capture_then_sync() {
        let t = |_| "2:05 PM".to_string();
        assert_eq!(status_line(CaptureLine::On, SyncLine::Synced(Some(1)), t), "Capturing · Synced 2:05 PM");
        assert_eq!(status_line(CaptureLine::Off, SyncLine::Off, t), "Capture off · Sync off");
        assert_eq!(status_line(CaptureLine::Paused, SyncLine::Synced(None), t), "Capture paused · Sync on");
        assert_eq!(status_line(CaptureLine::Stopped, SyncLine::Error, t), "Capture stopped · Sync error");
    }

    #[test]
    fn a_sync_error_wins_the_icon_and_anything_but_capturing_dims_it() {
        assert_eq!(icon_state(CaptureLine::On, SyncLine::Synced(None), false), IconState::Idle);
        assert_eq!(icon_state(CaptureLine::On, SyncLine::Off, false), IconState::Idle);
        assert_eq!(icon_state(CaptureLine::Off, SyncLine::Synced(None), false), IconState::Paused);
        assert_eq!(icon_state(CaptureLine::Stopped, SyncLine::SettingUp, false), IconState::Paused);
        assert_eq!(icon_state(CaptureLine::Off, SyncLine::Error, false), IconState::SyncError);
    }

    #[test]
    fn recording_wins_the_icon_over_everything() {
        assert_eq!(icon_state(CaptureLine::Off, SyncLine::Error, true), IconState::Recording);
        assert_eq!(icon_state(CaptureLine::On, SyncLine::Synced(None), true), IconState::Recording);
    }

    #[test]
    fn sync_states_map_to_lines() {
        assert_eq!(sync_line(SyncState::Ready, Some(5), false), SyncLine::Synced(Some(5)));
        assert_eq!(sync_line(SyncState::Ready, Some(5), true), SyncLine::Error);
        assert_eq!(sync_line(SyncState::WaitingForApproval, None, false), SyncLine::SettingUp);
        assert_eq!(sync_line(SyncState::NotSignedIn, None, false), SyncLine::Off);
        assert_eq!(sync_line(SyncState::Error, None, false), SyncLine::Error);
    }

    #[test]
    fn the_popup_sits_above_a_bottom_taskbar_centered_on_its_icon() {
        assert_eq!(popup_origin((1000, 1040, 24, 32), POPUP_PX, WORK, 8), (862, 840));
    }

    #[test]
    fn the_popup_stays_inside_the_work_area() {
        // An icon near the right edge: the popup is pushed left to fit.
        assert_eq!(popup_origin((1880, 1040, 24, 32), POPUP_PX, WORK, 8), (1612, 840));
        // A work area narrower than the popup pins it to the left margin.
        assert_eq!(popup_origin((10, 1040, 24, 32), POPUP_PX, (0, 0, 200, 1032), 8), (8, 840));
    }

    #[test]
    fn the_popup_hangs_below_a_top_taskbar() {
        assert_eq!(popup_origin((1000, 8, 24, 32), POPUP_PX, (0, 48, 1920, 1032), 8), (862, 56));
    }
}
