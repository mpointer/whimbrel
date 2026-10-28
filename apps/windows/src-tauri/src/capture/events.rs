//! System notifications for the capture loop, on one message-pump thread:
//!
//! - WinEvent hooks for foreground and focus changes (the capture triggers), as in the
//!   M3.1 spike's `ambient` loop;
//! - a message-only window registered for session lock/unlock
//!   (`WTSRegisterSessionNotification`), display on/off
//!   (`RegisterPowerSettingNotification(GUID_CONSOLE_DISPLAY_STATE)`), and suspend/resume
//!   (`WM_POWERBROADCAST`).
//!
//! Hook and window callbacks carry no user data, so events go out through one
//! process-wide sink. There is one capture service per process.

use std::sync::OnceLock;

use windows::core::w;
use windows::Win32::Foundation::{HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::{RegisterPowerSettingNotification, POWERBROADCAST_SETTING};
use windows::Win32::System::RemoteDesktop::{WTSRegisterSessionNotification, NOTIFY_FOR_THIS_SESSION};
use windows::Win32::System::SystemServices::GUID_CONSOLE_DISPLAY_STATE;
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, RegisterClassW, TranslateMessage,
    DEVICE_NOTIFY_WINDOW_HANDLE, EVENT_OBJECT_FOCUS, EVENT_SYSTEM_FOREGROUND, HWND_MESSAGE, MSG, PBT_APMRESUMEAUTOMATIC,
    PBT_APMRESUMESUSPEND, PBT_APMSUSPEND, PBT_POWERSETTINGCHANGE, WINDOW_EX_STYLE, WINDOW_STYLE, WINEVENT_OUTOFCONTEXT,
    WINEVENT_SKIPOWNPROCESS, WM_POWERBROADCAST, WM_WTSSESSION_CHANGE, WNDCLASSW, WTS_SESSION_LOCK, WTS_SESSION_UNLOCK,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemEvent {
    /// Foreground or focus changed.
    Focus,
    SessionLocked(bool),
    DisplayOn(bool),
    Suspending(bool),
}

type Sink = Box<dyn Fn(SystemEvent) + Send + Sync>;
static SINK: OnceLock<Sink> = OnceLock::new();

fn emit(event: SystemEvent) {
    if let Some(sink) = SINK.get() {
        sink(event);
    }
}

/// Starts the pump thread. Returns once the hooks and notifications are registered,
/// or the reason they couldn't be. Call at most once per process.
pub fn start(sink: impl Fn(SystemEvent) + Send + Sync + 'static) -> Result<(), String> {
    SINK.set(Box::new(sink)).map_err(|_| "capture events already started".to_string())?;
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel::<Result<(), String>>(1);
    std::thread::Builder::new()
        .name("whimbrel-capture-events".into())
        .spawn(move || unsafe {
            match register() {
                Ok(()) => {
                    let _ = ready_tx.send(Ok(()));
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            }
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        })
        .map_err(|e| e.to_string())?;
    ready_rx.recv().map_err(|_| "capture events thread exited".to_string())?
}

unsafe fn register() -> Result<(), String> {
    let module = GetModuleHandleW(None).map_err(|e| format!("GetModuleHandleW: {e}"))?;
    let instance = HINSTANCE(module.0);
    let class = w!("WhimbrelCaptureEvents");
    let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: instance, lpszClassName: class, ..Default::default() };
    if RegisterClassW(&wc) == 0 {
        return Err("RegisterClassW failed".into());
    }
    let hwnd = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        class,
        w!(""),
        WINDOW_STYLE(0),
        0,
        0,
        0,
        0,
        Some(HWND_MESSAGE),
        None,
        Some(instance),
        None,
    )
    .map_err(|e| format!("CreateWindowExW: {e}"))?;
    WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION).map_err(|e| format!("WTSRegisterSessionNotification: {e}"))?;
    RegisterPowerSettingNotification(HANDLE(hwnd.0), &GUID_CONSOLE_DISPLAY_STATE, DEVICE_NOTIFY_WINDOW_HANDLE)
        .map_err(|e| format!("RegisterPowerSettingNotification: {e}"))?;
    let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
    let foreground = SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(win_event), 0, 0, flags);
    let focus = SetWinEventHook(EVENT_OBJECT_FOCUS, EVENT_OBJECT_FOCUS, None, Some(win_event), 0, 0, flags);
    if foreground.is_invalid() || focus.is_invalid() {
        return Err("SetWinEventHook failed".into());
    }
    Ok(())
}

unsafe extern "system" fn win_event(_: HWINEVENTHOOK, _event: u32, _hwnd: HWND, _: i32, _: i32, _: u32, _: u32) {
    emit(SystemEvent::Focus);
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_WTSSESSION_CHANGE => match wparam.0 as u32 {
            WTS_SESSION_LOCK => emit(SystemEvent::SessionLocked(true)),
            WTS_SESSION_UNLOCK => emit(SystemEvent::SessionLocked(false)),
            _ => {}
        },
        WM_POWERBROADCAST => {
            match wparam.0 as u32 {
                PBT_POWERSETTINGCHANGE => {
                    let setting = &*(lparam.0 as *const POWERBROADCAST_SETTING);
                    if setting.PowerSetting == GUID_CONSOLE_DISPLAY_STATE && setting.DataLength >= 1 {
                        // 0 = off, 1 = on, 2 = dimmed (still on).
                        emit(SystemEvent::DisplayOn(setting.Data[0] != 0));
                    }
                }
                PBT_APMSUSPEND => emit(SystemEvent::Suspending(true)),
                PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND => emit(SystemEvent::Suspending(false)),
                _ => {}
            }
            return LRESULT(1);
        }
        _ => {}
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}
