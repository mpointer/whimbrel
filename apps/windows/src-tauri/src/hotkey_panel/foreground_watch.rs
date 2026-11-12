//! The last window the user was working in, for the tray's "Ask about this window…".
//!
//! Clicking the tray icon makes the taskbar the foreground window before the menu opens,
//! and choosing the item makes Whimbrel's menu the foreground. Neither is the window
//! meant. A `EVENT_SYSTEM_FOREGROUND` hook records every foreground change except to the
//! taskbar, its overflow, menus and Whimbrel itself, so the window before the click is
//! still known. Nothing but the handle is kept; nothing is read until the panel asks.

use std::sync::atomic::{AtomicIsize, Ordering};

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, IsWindow, EVENT_SYSTEM_FOREGROUND, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
};

/// The last foreground window that was not the shell or Whimbrel; 0 for none.
static LAST: AtomicIsize = AtomicIsize::new(0);

/// Window classes that take the foreground on the way to the tray menu.
const SHELL_CLASSES: &[&str] = &[
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
    "NotifyIconOverflowWindow",
    "TopLevelWindowForOverflowXamlIsland",
    "Xaml_WindowedPopupClass",
    // Start, search and other shell flyouts.
    "Windows.UI.Core.CoreWindow",
    "XamlExplorerHostIslandWindow",
    // Alt+Tab and Task View.
    "MultitaskingViewFrame",
    // The desktop.
    "Progman",
    "WorkerW",
    // Menus, Whimbrel's tray menu included.
    "#32768",
];

/// Whether a window of `class` is the taskbar or a menu rather than a window to ask about.
pub fn is_shell_surface(class: &str) -> bool {
    SHELL_CLASSES.contains(&class)
}

/// Starts recording. Call once, on the main thread (the hook is delivered through its
/// message loop).
pub fn start() {
    let hook: HWINEVENTHOOK = unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(on_foreground),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        )
    };
    if hook.is_invalid() {
        tracing::warn!("could not watch the foreground window; the tray's Ask about this window will have none");
    }
    // The hook lives as long as the process.
}

unsafe extern "system" fn on_foreground(
    _hook: HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _thread: u32,
    _time: u32,
) {
    if hwnd.0.is_null() {
        return;
    }
    if crate::capture::window::pid_of(hwnd) == std::process::id() {
        return;
    }
    let mut buf = [0u16; 256];
    let n = unsafe { GetClassNameW(hwnd, &mut buf) };
    let class = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
    if !is_shell_surface(&class) {
        LAST.store(hwnd.0 as isize, Ordering::Relaxed);
    }
}

/// The last window the user was working in, if it still exists.
pub fn last_window() -> Option<HWND> {
    let raw = LAST.load(Ordering::Relaxed);
    let hwnd = HWND(raw as *mut _);
    (raw != 0 && unsafe { IsWindow(Some(hwnd)) }.as_bool()).then_some(hwnd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_taskbar_and_menus_are_not_windows_to_ask_about() {
        assert!(is_shell_surface("Shell_TrayWnd"));
        assert!(is_shell_surface("#32768"));
        for class in ["Windows.UI.Core.CoreWindow", "XamlExplorerHostIslandWindow", "MultitaskingViewFrame", "Progman", "WorkerW"] {
            assert!(is_shell_surface(class), "{class}");
        }
        assert!(!is_shell_surface("OpusApp"));
        assert!(!is_shell_surface("Chrome_WidgetWin_1"));
    }
}
