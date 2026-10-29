//! Top-level window enumeration and app identity — docs/12 §4.3.
//!
//! `bundle_id` on Windows is, in order of preference: the window's AppUserModelID
//! (packaged apps, new Teams, Store apps), else the process's package AUMID, else the
//! lowercased executable basename.

use windows::core::{BOOL, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
use windows::Win32::Storage::Packaging::Appx::GetApplicationUserModelId;
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetForegroundWindow, GetWindow, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsIconic, IsWindowVisible, GW_OWNER,
};

#[derive(Debug, Clone, serde::Serialize)]
pub struct WindowInfo {
    #[serde(serialize_with = "ser_hwnd")]
    pub hwnd: HWND,
    pub pid: u32,
    pub title: String,
    pub exe: String,
    /// §4.3 identity — the string that goes in `ScreenTextRecord.bundle_id`.
    pub app_id: String,
    pub minimized: bool,
}

fn ser_hwnd<S: serde::Serializer>(h: &HWND, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&format!("0x{:x}", h.0 as usize))
}

pub fn hwnd_from_str(s: &str) -> Option<HWND> {
    let s = s.trim_start_matches("0x");
    usize::from_str_radix(s, 16).ok().map(|v| HWND(v as *mut _))
}

pub fn title(hwnd: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; len as usize + 1];
        let n = GetWindowTextW(hwnd, &mut buf);
        String::from_utf16_lossy(&buf[..n as usize])
    }
}

pub fn pid_of(hwnd: HWND) -> u32 {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    pid
}

/// Lowercased executable basename, e.g. `msedge.exe`.
pub fn exe_of(pid: u32) -> Option<String> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = CloseHandle(h);
        ok.ok()?;
        let full = String::from_utf16_lossy(&buf[..len as usize]);
        Some(basename_lower(&full))
    }
}

fn basename_lower(path: &str) -> String {
    path.rsplit(['\\', '/']).next().unwrap_or(path).to_lowercase()
}

fn window_aumid(hwnd: HWND) -> Option<String> {
    unsafe {
        let store: IPropertyStore = SHGetPropertyStoreForWindow(hwnd).ok()?;
        let v = store.GetValue(&PKEY_AppUserModel_ID).ok()?;
        let s = v.to_string();
        (!s.is_empty()).then_some(s)
    }
}

fn process_aumid(pid: u32) -> Option<String> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut len = 0u32;
        // First call sizes the buffer (fails with ERROR_INSUFFICIENT_BUFFER, or
        // APPMODEL_ERROR_NO_APPLICATION for unpackaged processes).
        let _ = GetApplicationUserModelId(h, &mut len, None);
        let out = if len > 0 {
            let mut buf = vec![0u16; len as usize];
            let rc = GetApplicationUserModelId(h, &mut len, Some(PWSTR(buf.as_mut_ptr())));
            (rc.is_ok()).then(|| String::from_utf16_lossy(&buf[..(len as usize).saturating_sub(1)]))
        } else {
            None
        };
        let _ = CloseHandle(h);
        out.filter(|s| !s.is_empty())
    }
}

/// §4.3: window AUMID → process package AUMID → lowercased exe basename.
pub fn app_identity(hwnd: HWND, pid: u32, exe: &str) -> String {
    pick_identity(window_aumid(hwnd), process_aumid(pid), exe)
}

fn pick_identity(window_aumid: Option<String>, process_aumid: Option<String>, exe: &str) -> String {
    window_aumid.or(process_aumid).unwrap_or_else(|| exe.to_lowercase())
}

pub fn info(hwnd: HWND) -> WindowInfo {
    let pid = pid_of(hwnd);
    let exe = exe_of(pid).unwrap_or_default();
    WindowInfo {
        hwnd,
        pid,
        title: title(hwnd),
        app_id: app_identity(hwnd, pid, &exe),
        exe,
        minimized: unsafe { IsIconic(hwnd).as_bool() },
    }
}

/// The window's on-screen area in pixels, from DWM's frame bounds (no drop shadow), else
/// `GetWindowRect`. 0 if Windows can't say.
pub fn area(hwnd: HWND) -> u64 {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
    let mut r = RECT::default();
    let size = std::mem::size_of::<RECT>() as u32;
    let found = unsafe { DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, &mut r as *mut _ as *mut _, size) }.is_ok()
        || unsafe { GetWindowRect(hwnd, &mut r) }.is_ok();
    if !found {
        return 0;
    }
    (r.right - r.left).max(0) as u64 * (r.bottom - r.top).max(0) as u64
}

/// Whether DWM is hiding the window: on another virtual desktop, a suspended Store app, or
/// a shell window that only looks visible. Capturing one yields no frame.
pub fn is_cloaked(hwnd: HWND) -> bool {
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
    let mut cloaked = 0u32;
    let size = std::mem::size_of::<u32>() as u32;
    unsafe { DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut _, size) }.is_ok() && cloaked != 0
}

pub fn foreground() -> Option<WindowInfo> {
    let h = unsafe { GetForegroundWindow() };
    (!h.0.is_null()).then(|| info(h))
}

/// Visible, unowned, titled top-level windows — the ones a user would call "a window".
pub fn top_level_windows() -> Vec<WindowInfo> {
    unsafe extern "system" fn cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let out = &mut *(lparam.0 as *mut Vec<HWND>);
        let visible = IsWindowVisible(hwnd).as_bool();
        let owned = GetWindow(hwnd, GW_OWNER).map(|o| !o.0.is_null()).unwrap_or(false);
        if visible && !owned && GetWindowTextLengthW(hwnd) > 0 {
            out.push(hwnd);
        }
        true.into()
    }
    let mut hwnds: Vec<HWND> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut hwnds as *mut _ as isize));
    }
    hwnds.into_iter().map(info).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_prefers_window_aumid_then_package_then_exe() {
        assert_eq!(
            pick_identity(Some("MSTeams_8wekyb3d8bbwe!MSTeams".into()), Some("x".into()), "ms-teams.exe"),
            "MSTeams_8wekyb3d8bbwe!MSTeams"
        );
        assert_eq!(pick_identity(None, Some("pkg!App".into()), "a.exe"), "pkg!App");
        assert_eq!(pick_identity(None, None, "Code.EXE"), "code.exe");
    }

    #[test]
    fn basename_is_lowercased_last_component() {
        assert_eq!(basename_lower(r"C:\Program Files\Microsoft\Edge\Application\msedge.exe"), "msedge.exe");
        assert_eq!(basename_lower("Code.exe"), "code.exe");
    }
}
