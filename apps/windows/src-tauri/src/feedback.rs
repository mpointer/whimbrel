//! Send Feedback (docs/superpowers/plans/2026-10-29-windows-send-feedback.md): a new email to Whimbrel
//! with the app version and the Windows build filled in, so every report says what it ran on. The
//! Mac's Help → "Send Feedback…" (`WhimbrelApp.feedbackMailURL`) does the same.

use crate::error::CommandError;
use crate::l10n::tr;

pub const ADDRESS: &str = "hello@whimbrelapp.com";

/// Percent-encodes everything but RFC 3986's unreserved characters, as a `mailto:` query value needs
/// (RFC 6068): spaces, line breaks and "·" all become `%XX`.
fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len() * 3);
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// The `mailto:` link for `version` of the app on `windows` (as [`describe_windows`] writes it).
/// `attachment` is the line about the diagnostics bundle, if there is one to mention.
pub fn mail_url(version: &str, windows: &str, attachment: Option<&str>) -> String {
    let subject = format!("Whimbrel feedback (Windows {version})");
    let attach = attachment.map(|line| format!("\n\n{line}")).unwrap_or_default();
    let body = format!("{attach}\n\n---\nWhimbrel {version} \u{00B7} {windows}");
    format!("mailto:{ADDRESS}?subject={}&body={}", encode(&subject), encode(&body))
}

/// What Send feedback did with the diagnostics bundle, for the About card.
#[derive(Debug, Clone, PartialEq, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackResult {
    /// Where the bundle was saved, for the user to attach.
    pub bundle_path: Option<String>,
    /// Why there is no bundle, when making it failed.
    pub bundle_error: Option<String>,
}

/// "Windows 11 24H2 (build 26100.2314)". The registry's `ProductName` still says Windows 10 on Windows 11,
/// so the version comes from the build: 22000 and above is 11.
pub fn describe_windows(build: Option<u32>, display_version: Option<&str>, ubr: Option<u32>) -> String {
    let Some(build) = build else { return "Windows".to_string() };
    let major = if build >= 22000 { "11" } else { "10" };
    let release = display_version.map(str::trim).filter(|v| !v.is_empty()).map(|v| format!(" {v}")).unwrap_or_default();
    let revision = ubr.map(|u| format!(".{u}")).unwrap_or_default();
    format!("Windows {major}{release} (build {build}{revision})")
}

const CURRENT_VERSION: windows::core::PCWSTR = windows::core::w!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion");

fn registry_string(name: windows::core::PCWSTR) -> Option<String> {
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};
    let mut buffer = [0u16; 128];
    let mut size = std::mem::size_of_val(&buffer) as u32;
    let result = unsafe {
        RegGetValueW(HKEY_LOCAL_MACHINE, CURRENT_VERSION, name, RRF_RT_REG_SZ, None, Some(buffer.as_mut_ptr().cast()), Some(&mut size))
    };
    if result.is_err() {
        return None;
    }
    let len = (size as usize / 2).saturating_sub(1).min(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..len]))
}

fn registry_dword(name: windows::core::PCWSTR) -> Option<u32> {
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD};
    let mut value: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    let result = unsafe {
        RegGetValueW(HKEY_LOCAL_MACHINE, CURRENT_VERSION, name, RRF_RT_REG_DWORD, None, Some((&mut value as *mut u32).cast()), Some(&mut size))
    };
    result.is_ok().then_some(value)
}

/// This PC's Windows, as the feedback email names it.
pub fn this_windows() -> String {
    use windows::core::w;
    let build = registry_string(w!("CurrentBuild")).and_then(|b| b.trim().parse().ok());
    describe_windows(build, registry_string(w!("DisplayVersion")).as_deref(), registry_dword(w!("UBR")))
}

/// Opens a new feedback email in the user's mail app.
pub fn open(attachment: Option<&str>) -> Result<(), String> {
    use windows::core::{w, HSTRING, PCWSTR};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let url = mail_url(env!("CARGO_PKG_VERSION"), &this_windows(), attachment);
    let result = unsafe { ShellExecuteW(None, w!("open"), &HSTRING::from(url), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
    if result.0 as isize > 32 {
        Ok(())
    } else {
        Err(format!("no mail app opened the feedback email (ShellExecute returned {})", result.0 as isize))
    }
}

/// Settings → About this PC: "Send feedback…". Saves a diagnostics bundle, shows it in File
/// Explorer, then opens the email, which asks for it to be attached. Off the main thread: the
/// bundle reads up to 15 MB of logs and the store.
#[tauri::command]
pub async fn send_feedback(app: tauri::AppHandle) -> Result<FeedbackResult, CommandError> {
    tauri::async_runtime::spawn_blocking(move || send_with_bundle(&app))
        .await
        .map_err(|e| CommandError::not_configured(e.to_string()))?
}

/// The bundle, File Explorer, then the email: the About card's button and the tray's
/// Send feedback. Blocking.
pub fn send_with_bundle(app: &tauri::AppHandle) -> Result<FeedbackResult, CommandError> {
    use tauri::Manager;
    let made = crate::diagnostics::bundle(&app.state::<crate::state::AppState>());
    let (result, line) = match made {
        Ok(path) => {
            if let Err(e) = crate::diagnostics::reveal(&path) {
                tracing::warn!(error = %e, "could not show the diagnostics bundle in File Explorer");
            }
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let line = tr(
                "Please attach {file}, the diagnostics file Whimbrel just saved (File Explorer has it selected). It has recent logs and settings status, with email addresses and meeting titles removed.",
                &[("file", &name)],
            );
            (FeedbackResult { bundle_path: Some(path.display().to_string()), bundle_error: None }, line)
        }
        Err(e) => {
            tracing::warn!(error = %e, "could not make the diagnostics bundle");
            let line = tr("Whimbrel couldn't save a diagnostics file: {error}", &[("error", &e)]);
            (FeedbackResult { bundle_path: None, bundle_error: Some(e) }, line)
        }
    };
    open(Some(&line)).map_err(|e| {
        tracing::warn!(error = %e, "send feedback");
        CommandError::not_configured(tr("Couldn't open your mail app. Email {address} instead.", &[("address", &ADDRESS.to_string())]))
    })?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_link_has_the_address_subject_and_body() {
        let url = mail_url("0.9.39", "Windows 11 24H2 (build 26100.2314)", None);
        assert_eq!(
            url,
            "mailto:hello@whimbrelapp.com?subject=Whimbrel%20feedback%20%28Windows%200.9.39%29\
             &body=%0A%0A---%0AWhimbrel%200.9.39%20%C2%B7%20Windows%2011%2024H2%20%28build%2026100.2314%29"
        );
    }

    #[test]
    fn nothing_in_the_values_is_left_unencoded() {
        let url = mail_url("1.0 & more?", "Windows #1 = x", Some("Attach Whimbrel-diagnostics-1.zip & send?"));
        let query = url.split_once('?').unwrap().1;
        for pair in query.split('&') {
            let (_, value) = pair.split_once('=').unwrap();
            assert!(value.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._~%".contains(&b)), "{value}");
        }
    }

    #[test]
    fn windows_is_named_by_its_build() {
        assert_eq!(describe_windows(Some(26100), Some("24H2"), Some(2314)), "Windows 11 24H2 (build 26100.2314)");
        assert_eq!(describe_windows(Some(19045), Some("22H2"), Some(5011)), "Windows 10 22H2 (build 19045.5011)");
        assert_eq!(describe_windows(Some(22000), None, None), "Windows 11 (build 22000)");
        assert_eq!(describe_windows(Some(26100), Some("  "), None), "Windows 11 (build 26100)");
        assert_eq!(describe_windows(None, Some("24H2"), Some(1)), "Windows");
    }

    #[test]
    fn this_pc_reads_as_windows_10_or_11() {
        let described = this_windows();
        assert!(described.starts_with("Windows 1"), "{described}");
        assert!(described.contains("(build "), "{described}");
    }
}
