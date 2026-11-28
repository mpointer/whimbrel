//! The Windows "Text size" setting (Settings, Accessibility, Text size), and what it means for the small windows.
//!
//! Measured on a real PC (2026-11-23): WebView2 multiplies `devicePixelRatio` by the text scale. The root font stays 16 px and the
//! CSS viewport shrinks, so a fixed logical-size window gets 1/scale of the CSS space (a 360x230 alert at 200% lays out in 180x115
//! CSS px, everything drawn twice as large). Every small window therefore multiplies its logical size by the
//! scale when it is created, so it has its full design size in CSS px at any text size, and is then capped to the work area
//! (`fit_to_work_area`). The UI's compact layout is the fallback for when that cap bit.

use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

/// The smallest and largest value Windows stores: 100% to 225%.
const MIN_PERCENT: u32 = 100;
const MAX_PERCENT: u32 = 225;

/// The scale from the stored value: `None` (no value, or it could not be read) is 1.0; anything else is kept between 100 and 225
/// percent. Pure, so the parsing and clamping are testable.
pub fn parse_text_scale(raw: Option<u32>) -> f64 {
    match raw {
        None => 1.0,
        Some(percent) => f64::from(percent.clamp(MIN_PERCENT, MAX_PERCENT)) / 100.0,
    }
}

/// The Windows text scale now, 1.0 to 2.25: `HKCU\Software\Microsoft\Accessibility\TextScaleFactor` (a DWORD, percent). The value is
/// absent until the person moves the slider, which means 100%. Any error is 1.0.
pub fn text_scale() -> f64 {
    use windows::core::w;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    let mut value: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Accessibility"),
            w!("TextScaleFactor"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    parse_text_scale(if result.is_ok() { Some(value) } else { None })
}

/// A logical size multiplied by `scale`. Pure.
pub fn scale_size(base: (f64, f64), scale: f64) -> (f64, f64) {
    (base.0 * scale, base.1 * scale)
}

/// A small window's logical size: the design size times the text scale (the registry is read here and only here).
pub fn scaled_size(base: (f64, f64)) -> (f64, f64) {
    scale_size(base, text_scale())
}

/// A size in physical pixels capped to the work area less a margin on each side, so a window never needs more room than the
/// screen has. `work` is (x, y, width, height). A size that already fits is returned as it was.
pub fn fit_to_work_area(size: (u32, u32), work: (i32, i32, u32, u32), margin: i32) -> (u32, u32) {
    let room = |extent: u32| (extent as i32 - 2 * margin).max(1) as u32;
    (size.0.min(room(work.2)), size.1.min(room(work.3)))
}

/// `y` kept inside the work area for a window of `height` physical pixels: no lower than the bottom margin allows, no higher than
/// the top margin. A window taller than the room pins to the top margin. Pure.
pub fn clamp_y(y: i32, height: u32, work: (i32, i32, u32, u32), margin: i32) -> i32 {
    let min_y = work.1 + margin;
    y.clamp(min_y, (work.1 + work.3 as i32 - height as i32 - margin).max(min_y))
}

/// The top-left of a window of `size` kept fully inside the work area (of `areas`) that holds most of it, `margin` from its edges
/// (0 keeps a place the person chose as it is when it already fits). A window larger than the area pins to the top-left margin.
/// With no area the place is returned as it was. Pure.
pub fn clamp_into_work_areas(at: (i32, i32), size: (u32, u32), areas: &[(i32, i32, u32, u32)], margin: i32) -> (i32, i32) {
    let overlap = |a: &(i32, i32, u32, u32)| {
        let across = ((at.0 as i64 + size.0 as i64).min(a.0 as i64 + a.2 as i64) - (at.0 as i64).max(a.0 as i64)).max(0);
        let down = ((at.1 as i64 + size.1 as i64).min(a.1 as i64 + a.3 as i64) - (at.1 as i64).max(a.1 as i64)).max(0);
        across * down
    };
    let Some(work) = areas.iter().copied().reduce(|best, a| if overlap(&a) > overlap(&best) { a } else { best }) else { return at };
    let min_x = work.0 + margin;
    let x = at.0.clamp(min_x, (work.0 + work.2 as i32 - size.0 as i32 - margin).max(min_x));
    (x, clamp_y(at.1, size.1, work, margin))
}

/// Every corner alert's window label. Each one opens clear of all the others that are up, whatever order they arrive in.
pub const CORNER_ALERTS: [&str; 5] = [
    crate::alert::ALERT,
    crate::question_alert::QUESTION_ALERT,
    crate::budget_alert::BUDGET_ALERT,
    crate::disk_alert::DISK_ALERT,
    crate::meetings::still_here::STILL_HERE,
];

/// The top edge for a new corner alert of `height`: the highest place at or below `top` where it clears every window in
/// `taken` (each a top edge and a height) by `gap`. A space left between two windows is used when the new one fits in it;
/// otherwise it goes below the lowest. Nothing already up is moved. Pure.
pub fn stack_slot(top: i32, height: u32, taken: &[(i32, u32)], gap: i32) -> i32 {
    let mut taken = taken.to_vec();
    taken.sort_by_key(|t| t.0);
    let mut y = top;
    for (other_y, other_height) in taken {
        if y + height as i32 + gap <= other_y {
            break;
        }
        y = y.max(other_y + other_height as i32 + gap);
    }
    y
}

/// Places a corner alert (meeting alert, question, budget, disk, still-here) on the primary monitor: its top-right
/// corner, clear of every other corner alert that is up (by their real places and heights, which may have been capped), the
/// size capped to the work area and the position kept on screen. `logical` is the size the window was built with (already
/// scaled by the text scale); `margin` is the gap in logical pixels.
pub fn place_corner_alert(app: &AppHandle, window: &WebviewWindow, logical: (f64, f64), margin: f64) -> tauri::Result<()> {
    let Some(monitor) = window.primary_monitor()? else { return Ok(()) };
    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    let work = (area.position.x, area.position.y, area.size.width, area.size.height);
    let m = (margin * scale).round() as i32;
    let wanted = ((logical.0 * scale).round() as u32, (logical.1 * scale).round() as u32);
    let size = fit_to_work_area(wanted, work, m);
    let (x, top) = crate::alert::alert_origin(size, work, m);
    let mut taken = Vec::new();
    for label in CORNER_ALERTS {
        if label == window.label() {
            continue;
        }
        if let Some(other) = app.get_webview_window(label) {
            taken.push((other.outer_position()?.y, other.outer_size()?.height));
        }
    }
    let y = clamp_y(stack_slot(top, size.1, &taken, m), size.1, work, m);
    window.set_position(PhysicalPosition::new(x, y))?;
    if size != wanted {
        window.set_size(PhysicalSize::new(size.0, size.1))?;
    }
    Ok(())
}

/// Whether a corner alert opens active and focused: only when a screen reader is running. Narrator speaks a focus change and
/// stays silent for a window that appears without one (measured on a real PC, 2026-11-25), so for that person the alert takes
/// the keyboard, which also lets them reach its buttons. Everyone else keeps the no-activate show. Pure.
pub fn corner_alert_activates(screen_reader_running: bool) -> bool {
    screen_reader_running
}

/// Whether Windows says a screen reader is running (`SPI_GETSCREENREADER`, which Narrator and the other readers set). Read each
/// time an alert opens, so turning Narrator on or off takes effect on the next alert. Any error counts as not running.
pub fn screen_reader_running() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{SystemParametersInfoW, SPI_GETSCREENREADER, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS};
    // A Win32 BOOL is a 32-bit integer.
    let mut on: i32 = 0;
    let read = unsafe { SystemParametersInfoW(SPI_GETSCREENREADER, 0, Some(&mut on as *mut i32 as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)) };
    read.is_ok() && on != 0
}

/// Shows a corner alert. Without a screen reader it does not become the active window, so it does not close an open
/// tray menu or take the keyboard from the app in use (`WebviewWindow::show` activates the window even when it was built
/// unfocused); a click or Alt+Tab still gives it the keyboard. With a screen reader running (`corner_alert_activates`) it is shown
/// active and brought to the front, and the page puts focus on the alert's own content, so the reader says it as a focus change.
pub fn show_corner_alert(window: &WebviewWindow) -> tauri::Result<()> {
    use windows::Win32::UI::WindowsAndMessaging::{SetForegroundWindow, ShowWindow, SW_SHOW, SW_SHOWNOACTIVATE};
    let hwnd = windows::Win32::Foundation::HWND(window.hwnd()?.0);
    if corner_alert_activates(screen_reader_running()) {
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOW);
            // Windows may refuse to move the foreground to a background process; the window is then still shown, and
            // `set_focus` asks the shell's own way as well. Neither failure is an error to the caller.
            let _ = SetForegroundWindow(hwnd);
        }
        let _ = window.set_focus();
    } else {
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: (i32, i32, u32, u32) = (0, 0, 1920, 1032);

    #[test]
    fn a_screen_reader_makes_the_alert_take_focus_and_no_screen_reader_does_not() {
        assert!(corner_alert_activates(true));
        assert!(!corner_alert_activates(false));
    }

    #[test]
    fn no_value_means_100_percent() {
        assert_eq!(parse_text_scale(None), 1.0);
    }

    #[test]
    fn the_stored_percent_is_the_scale() {
        assert_eq!(parse_text_scale(Some(100)), 1.0);
        assert_eq!(parse_text_scale(Some(150)), 1.5);
        assert_eq!(parse_text_scale(Some(200)), 2.0);
        assert_eq!(parse_text_scale(Some(225)), 2.25);
    }

    #[test]
    fn a_value_outside_100_to_225_is_clamped() {
        assert_eq!(parse_text_scale(Some(0)), 1.0);
        assert_eq!(parse_text_scale(Some(99)), 1.0);
        assert_eq!(parse_text_scale(Some(226)), 2.25);
        assert_eq!(parse_text_scale(Some(u32::MAX)), 2.25);
    }

    #[test]
    fn a_size_scales_in_both_directions() {
        assert_eq!(scale_size((360.0, 230.0), 2.0), (720.0, 460.0));
        assert_eq!(scale_size((360.0, 230.0), 1.0), (360.0, 230.0));
    }

    #[test]
    fn a_size_that_fits_is_unchanged() {
        assert_eq!(fit_to_work_area((720, 460), WORK, 12), (720, 460));
    }

    #[test]
    fn a_size_is_capped_to_the_work_area_less_a_margin_each_side() {
        assert_eq!(fit_to_work_area((810, 1300), WORK, 12), (810, 1008));
        assert_eq!(fit_to_work_area((2000, 500), WORK, 12), (1896, 500));
        assert_eq!(fit_to_work_area((480, 560), (0, 0, 400, 1000), 12), (376, 560));
    }

    #[test]
    fn a_tiny_work_area_still_leaves_one_pixel() {
        assert_eq!(fit_to_work_area((300, 300), (0, 0, 10, 10), 12), (1, 1));
    }

    #[test]
    fn a_stacked_window_that_would_leave_the_screen_is_pulled_up() {
        // 1032 high, margin 12: the lowest top for a 400 high window is 620.
        assert_eq!(clamp_y(900, 400, WORK, 12), 620);
        assert_eq!(clamp_y(300, 400, WORK, 12), 300);
    }

    #[test]
    fn a_remembered_place_is_moved_in_so_a_larger_window_is_on_its_screen() {
        // Dragged to the right edge at 100%: 320 wide at x 1580 fits; at 200% (640 wide) it would hang off, so it moves left.
        assert_eq!(clamp_into_work_areas((1580, 20), (320, 260), &[WORK], 0), (1580, 20));
        assert_eq!(clamp_into_work_areas((1580, 20), (640, 520), &[WORK], 0), (1280, 20));
        assert_eq!(clamp_into_work_areas((100, 900), (640, 520), &[WORK], 0), (100, 512));
        // On a second screen: the area with most of the window is used.
        let two = [(-1920, 0, 1920, 1032), WORK];
        assert_eq!(clamp_into_work_areas((-700, 20), (640, 520), &two, 0), (-700, 20));
        assert_eq!(clamp_into_work_areas((-500, 20), (640, 520), &two, 0), (-640, 20));
        // Larger than the area: pinned to the top-left margin. No areas: as it was.
        assert_eq!(clamp_into_work_areas((500, 500), (2000, 2000), &[WORK], 12), (12, 12));
        assert_eq!(clamp_into_work_areas((500, 500), (640, 520), &[], 12), (500, 500));
    }

    #[test]
    fn the_first_corner_alert_takes_the_top() {
        assert_eq!(stack_slot(12, 230, &[], 12), 12);
    }

    #[test]
    fn a_later_alert_goes_below_the_one_that_is_up_whichever_came_first() {
        // The question alert (211 high) is up at the top; the meeting alert (231 high) then opens below it, not over it.
        assert_eq!(stack_slot(13, 231, &[(13, 211)], 12), 236);
        // And the other way round.
        assert_eq!(stack_slot(13, 211, &[(13, 231)], 12), 256);
    }

    #[test]
    fn a_third_alert_goes_below_both_in_any_order() {
        assert_eq!(stack_slot(12, 200, &[(254, 210), (12, 230)], 12), 476);
    }

    #[test]
    fn a_freed_space_is_used_only_when_the_new_alert_fits_in_it() {
        // The top alert closed and left 12 to 254 free above one at 254: a 230 high alert fits there, a 240 high one does not.
        assert_eq!(stack_slot(12, 230, &[(254, 210)], 12), 12);
        assert_eq!(stack_slot(12, 240, &[(254, 210)], 12), 476);
    }

    #[test]
    fn a_window_taller_than_the_room_pins_to_the_top_margin() {
        assert_eq!(clamp_y(500, 2000, WORK, 12), 12);
        assert_eq!(clamp_y(500, 400, (0, 100, 1920, 1000), 12), 500);
        assert_eq!(clamp_y(5, 400, (0, 100, 1920, 1000), 12), 112);
    }
}
