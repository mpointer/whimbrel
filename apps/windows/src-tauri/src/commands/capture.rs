//! Settings → Capture (plan Task 7): user exclusions (chosen from the apps with a window
//! open right now) and whether this PC has an OCR language pack for the user's language.

use schemars::JsonSchema;
use serde::Serialize;
use tauri::State;

use crate::capture::{app_id, window};
use crate::error::CommandError;
use crate::settings::keys;
use crate::state::AppState;

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunningApp {
    /// The §4.3 identity capture stores (normalized); what an exclusion matches.
    pub app_id: String,
    pub exe: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OcrLanguages {
    /// Languages Windows can OCR (installed language packs with OCR).
    pub available: Vec<String>,
    /// The user's preferred languages, most preferred first.
    pub profile: Vec<String>,
    /// Whether the most preferred language has an OCR pack.
    pub covered: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CaptureSettings {
    pub excluded: Vec<String>,
    pub running_apps: Vec<RunningApp>,
    pub ocr: OcrLanguages,
}

/// `en-US` is covered by `en-US` or by a bare `en`, case-insensitively.
pub fn language_covered(profile_tag: &str, available: &[String]) -> bool {
    let primary = profile_tag.split('-').next().unwrap_or(profile_tag);
    available
        .iter()
        .any(|a| a.eq_ignore_ascii_case(profile_tag) || a.eq_ignore_ascii_case(primary) || a.split('-').next().is_some_and(|p| p.eq_ignore_ascii_case(primary) && profile_tag.eq_ignore_ascii_case(primary)))
}

fn ocr_languages() -> OcrLanguages {
    use windows::Media::Ocr::OcrEngine;
    use windows::System::UserProfile::GlobalizationPreferences;
    let available: Vec<String> = OcrEngine::AvailableRecognizerLanguages()
        .map(|langs| langs.into_iter().filter_map(|l| l.LanguageTag().ok().map(|t| t.to_string())).collect())
        .unwrap_or_default();
    let profile: Vec<String> = GlobalizationPreferences::Languages()
        .map(|langs| langs.into_iter().map(|t| t.to_string()).collect())
        .unwrap_or_default();
    let covered = profile.first().is_some_and(|p| language_covered(p, &available));
    OcrLanguages { available, profile, covered }
}

/// Distinct apps with a visible window, by normalized identity, Whimbrel excluded.
pub fn distinct_running_apps(windows: Vec<window::WindowInfo>, own_pid: u32) -> Vec<RunningApp> {
    let mut apps: Vec<RunningApp> = Vec::new();
    for w in windows.into_iter().filter(|w| w.pid != own_pid && !w.exe.is_empty()) {
        let id = app_id::normalize(&w.app_id, &w.exe);
        if !apps.iter().any(|a| a.app_id == id) {
            apps.push(RunningApp { app_id: id, exe: w.exe, title: w.title });
        }
    }
    apps.sort_by(|a, b| a.exe.cmp(&b.exe));
    apps
}

#[tauri::command]
pub async fn capture_settings(state: State<'_, AppState>) -> Result<CaptureSettings, CommandError> {
    let excluded = state.settings.lock().unwrap().excluded_bundle_ids();
    let (running_apps, ocr) = tokio::task::spawn_blocking(|| (distinct_running_apps(window::top_level_windows(), std::process::id()), ocr_languages()))
        .await
        .map_err(|e| CommandError::store(e.to_string()))?;
    Ok(CaptureSettings { excluded, running_apps, ocr })
}

/// Replaces the user exclusion list and applies it to the running capture service.
#[tauri::command]
pub fn set_excluded_apps(state: State<'_, AppState>, excluded: Vec<String>) -> Result<Vec<String>, CommandError> {
    let mut list: Vec<String> = excluded.into_iter().map(|e| e.trim().to_string()).filter(|e| !e.is_empty()).collect();
    list.sort_by_key(|e| e.to_lowercase());
    list.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    {
        let mut settings = state.settings.lock().unwrap();
        settings.set(keys::EXCLUDED_BUNDLE_IDS, Some(&list));
        settings.save().map_err(|e| CommandError::invalid_input(format!("could not save settings: {e}")))?;
    }
    if let Some(capture) = &state.capture {
        capture.set_user_exclusions(list.clone());
    }
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Foundation::HWND;

    fn win(pid: u32, exe: &str, app_id: &str, title: &str) -> window::WindowInfo {
        window::WindowInfo { hwnd: HWND(std::ptr::null_mut()), pid, title: title.into(), exe: exe.into(), app_id: app_id.into(), minimized: false }
    }

    #[test]
    fn running_apps_are_distinct_normalized_and_exclude_whimbrel() {
        let apps = distinct_running_apps(
            vec![
                win(1, "msedge.exe", "MSEdge", "Gmail"),
                win(2, "msedge.exe", "MSEdge.profile2.Default", "Docs"),
                win(3, "code.exe", "code.exe", "main.rs"),
                win(99, "whimbrel-windows.exe", "whimbrel-windows.exe", "Whimbrel"),
            ],
            99,
        );
        assert_eq!(apps.iter().map(|a| a.app_id.as_str()).collect::<Vec<_>>(), ["code.exe", "MSEdge"]);
    }

    #[test]
    fn a_profile_language_is_covered_by_its_region_or_its_bare_tag() {
        let available = vec!["en-US".to_string(), "fr".to_string()];
        assert!(language_covered("en-US", &available));
        assert!(language_covered("fr-CA", &available), "a bare `fr` pack covers fr-CA");
        assert!(!language_covered("de-DE", &available));
    }

    /// `cargo test --lib -- --ignored ocr_languages`. Needs an OCR language pack, which a
    /// bare CI runner may not have.
    #[test]
    #[ignore = "reads this PC's installed OCR language packs"]
    fn this_pc_reports_its_ocr_languages() {
        // M3.1 found en-US installed on this laptop; any real reading must list something.
        let langs = ocr_languages();
        assert!(!langs.available.is_empty(), "{langs:?}");
    }
}
