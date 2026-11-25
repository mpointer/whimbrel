//! The hotkey panel's window (spec §3): a small always-on-top window with no taskbar
//! button, built like the meeting alert (`alert.rs`) but focused, since the user types in
//! it. The React UI picks its view by the window label.

use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder};

use crate::state::AppState;

/// The panel window's label (also listed in `capabilities/default.json`).
pub const HOTKEY_PANEL: &str = "hotkey-panel";
/// Its size in logical pixels.
const SIZE: (f64, f64) = (560.0, 360.0);
/// The gap kept to the work area's edges when the redesigned panel is capped to it, in logical pixels.
const PANEL_MARGIN: f64 = 12.0;

/// Where the panel sits, in physical pixels: centred across the work area, a fifth of the
/// way down. Pure, so the placement is testable without a monitor.
pub fn panel_origin(size: (u32, u32), work_area: (i32, i32, u32, u32)) -> (i32, i32) {
    let (area_x, area_y, area_w, area_h) = work_area;
    let x = area_x + (area_w as i32 - size.0 as i32) / 2;
    let y = area_y + (area_h as i32 - size.1 as i32).max(0) / 5;
    (x.max(area_x), y)
}

/// Whether the panel is left out of a shared screen: with the redesign on, as the in-meeting window is
/// (`meetings::meeting_bar::HIDE_FROM_SCREEN_SHARING`), since it shows text read from the window in front and the answer
/// about it. With the redesign off it is drawn into a share as it always was.
pub fn hidden_from_sharing(redesign: bool) -> bool {
    redesign && crate::meetings::meeting_bar::HIDE_FROM_SCREEN_SHARING
}

pub fn is_open(app: &AppHandle) -> bool {
    app.get_webview_window(HOTKEY_PANEL).is_some()
}

/// Opens the panel on the monitor holding the point `near` (the recorded window's centre,
/// physical pixels), else the primary monitor, and gives it focus.
pub fn open(app: &AppHandle, near: Option<(f64, f64)>) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window(HOTKEY_PANEL) {
        window.show()?;
        window.set_focus()?;
        return Ok(());
    }
    let redesign = app.state::<AppState>().settings.lock().unwrap().redesign_enabled();
    // With the redesign on the panel grows with the Windows text scale (`text_scale.rs`); with it off this is `SIZE`, no read.
    let (w, h) = crate::text_scale::size_for_look(SIZE, redesign);
    let panel = WebviewWindowBuilder::new(app, HOTKEY_PANEL, WebviewUrl::App("index.html".into()))
        .title("Whimbrel")
        .inner_size(w, h)
        .decorations(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .content_protected(hidden_from_sharing(redesign))
        .focused(true)
        .visible(false)
        .build()?;
    let monitor = match near {
        Some((x, y)) => app.monitor_from_point(x, y)?,
        None => None,
    };
    if let Some(monitor) = monitor.or(panel.primary_monitor()?) {
        let scale = monitor.scale_factor();
        let area = monitor.work_area();
        let work = (area.position.x, area.position.y, area.size.width, area.size.height);
        let wanted = ((w * scale).round() as u32, (h * scale).round() as u32);
        if redesign {
            // Capped to the work area less a margin, moved to the monitor first and sized there (as the tray popup does, so
            // Windows does not rescale a size set on the monitor it was created on), then placed from the size applied.
            let size = crate::text_scale::fit_to_work_area(wanted, work, (PANEL_MARGIN * scale).round() as i32);
            let (x, y) = panel_origin(size, work);
            panel.set_position(PhysicalPosition::new(x, y))?;
            if size != wanted {
                panel.set_size(PhysicalSize::new(size.0, size.1))?;
                let applied = panel.outer_size().map(|s| (s.width, s.height)).unwrap_or(size);
                let (x, y) = panel_origin(applied, work);
                panel.set_position(PhysicalPosition::new(x, y))?;
            }
        } else {
            let (x, y) = panel_origin(wanted, work);
            panel.set_position(PhysicalPosition::new(x, y))?;
        }
    }
    panel.show()?;
    panel.set_focus()?;
    Ok(())
}

pub fn close(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(HOTKEY_PANEL) {
        let _ = window.destroy();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panel_is_hidden_from_a_shared_screen_only_with_the_redesign() {
        assert!(hidden_from_sharing(true));
        assert!(!hidden_from_sharing(false));
    }

    #[test]
    fn the_panel_is_centred_a_fifth_of_the_way_down() {
        assert_eq!(panel_origin((560, 360), (0, 0, 1920, 1032)), (680, 134));
    }

    #[test]
    fn it_follows_a_second_monitors_work_area() {
        assert_eq!(panel_origin((560, 360), (-1920, 100, 1920, 1000)), (-1240, 228));
    }

    #[test]
    fn a_small_work_area_keeps_it_on_screen() {
        assert_eq!(panel_origin((560, 360), (0, 0, 400, 300)), (0, 0));
    }
}
