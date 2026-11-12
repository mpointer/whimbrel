//! The hotkey panel's window (spec §3): a small always-on-top window with no taskbar
//! button, built like the meeting alert (`alert.rs`) but focused, since the user types in
//! it. The React UI picks its view by the window label.

use tauri::{AppHandle, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};

/// The panel window's label (also listed in `capabilities/default.json`).
pub const HOTKEY_PANEL: &str = "hotkey-panel";
/// Its size in logical pixels.
const SIZE: (f64, f64) = (560.0, 360.0);

/// Where the panel sits, in physical pixels: centred across the work area, a fifth of the
/// way down. Pure, so the placement is testable without a monitor.
pub fn panel_origin(size: (u32, u32), work_area: (i32, i32, u32, u32)) -> (i32, i32) {
    let (area_x, area_y, area_w, area_h) = work_area;
    let x = area_x + (area_w as i32 - size.0 as i32) / 2;
    let y = area_y + (area_h as i32 - size.1 as i32).max(0) / 5;
    (x.max(area_x), y)
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
    let (w, h) = SIZE;
    let panel = WebviewWindowBuilder::new(app, HOTKEY_PANEL, WebviewUrl::App("index.html".into()))
        .title("Whimbrel")
        .inner_size(w, h)
        .decorations(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
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
        let (x, y) = panel_origin(
            ((w * scale).round() as u32, (h * scale).round() as u32),
            (area.position.x, area.position.y, area.size.width, area.size.height),
        );
        panel.set_position(PhysicalPosition::new(x, y))?;
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
