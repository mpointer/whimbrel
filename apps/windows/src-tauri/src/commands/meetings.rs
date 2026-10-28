//! Meeting recording commands (M3.3 Task 3).
//!
//! Async, with the work on a blocking thread. A synchronous Tauri command runs on the main
//! thread, and starting (a row write that can queue behind sync) or stopping (waiting for
//! the WAVs to finalize) there froze the window.

use tauri::{AppHandle, Manager};

use crate::error::CommandError;
use crate::meetings::controller::{MeetingController, MeetingRecordingView};
use crate::state::AppState;

async fn off_main<F>(app: AppHandle, f: F) -> Result<MeetingRecordingView, CommandError>
where
    F: FnOnce(&MeetingController) -> Result<MeetingRecordingView, CommandError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || {
        let view = f(app.state::<AppState>().meetings()?);
        crate::tray::refresh(&app);
        view
    })
    .await
    .map_err(|e| CommandError::store(format!("the meeting command failed: {e}")))?
}

#[tauri::command]
pub async fn meeting_record_start(app: AppHandle) -> Result<MeetingRecordingView, CommandError> {
    off_main(app, |m| m.start_manual()).await
}

#[tauri::command]
pub async fn meeting_record_stop(app: AppHandle) -> Result<MeetingRecordingView, CommandError> {
    off_main(app, |m| m.stop()).await
}

#[tauri::command]
pub async fn meeting_recording_status(app: AppHandle) -> Result<MeetingRecordingView, CommandError> {
    tauri::async_runtime::spawn_blocking(move || Ok(app.state::<AppState>().meetings()?.status()))
        .await
        .map_err(|e| CommandError::store(format!("the meeting command failed: {e}")))?
}

/// Settings → Meetings: how calls are detected and started.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MeetingSettings {
    /// Watch for calls at all (FR-MTG-2). On by default.
    pub heuristic_detection: bool,
    /// Start recording when the alert's countdown ends, instead of waiting. Off by default.
    pub auto_start: bool,
    /// Minutes before a calendar meeting the alert appears, 1–10 (FR-MTG-1).
    pub lead_minutes: f64,
    /// Minutes before a calendar meeting its Meeting Prep brief is made, 5-120. Kept on this PC.
    pub prep_lead_minutes: f64,
}

fn read_settings(settings: &crate::settings::SettingsFile) -> MeetingSettings {
    MeetingSettings {
        heuristic_detection: settings.heuristic_detection_enabled(),
        auto_start: settings.auto_start_enabled(),
        lead_minutes: settings.lead_minutes(),
        prep_lead_minutes: settings.prep_lead_minutes(),
    }
}

fn apply_settings(file: &mut crate::settings::SettingsFile, settings: MeetingSettings) {
    file.set_meeting_settings(settings.heuristic_detection, settings.auto_start, settings.lead_minutes);
    file.set_prep_lead_minutes(settings.prep_lead_minutes);
}

#[tauri::command]
pub fn meeting_settings(state: tauri::State<'_, AppState>) -> MeetingSettings {
    read_settings(&state.settings.lock().unwrap())
}

#[tauri::command]
pub fn set_meeting_settings(state: tauri::State<'_, AppState>, settings: MeetingSettings) -> Result<MeetingSettings, CommandError> {
    {
        let mut file = state.settings.lock().unwrap();
        apply_settings(&mut file, settings);
        file.save().map_err(|e| CommandError::invalid_input(format!("could not save settings: {e}")))?;
    }
    Ok(meeting_settings(state))
}

#[cfg(test)]
mod settings_tests {
    use super::*;
    use crate::settings::SettingsFile;

    #[test]
    fn saving_the_meeting_settings_keeps_the_prep_lead_apart_from_the_alert_lead() {
        let dir = std::env::temp_dir().join(format!("whimbrel-meeting-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut file = SettingsFile::load(&dir.join("settings.json"));
        assert_eq!(read_settings(&file).prep_lead_minutes, 30.0);
        apply_settings(&mut file, MeetingSettings { heuristic_detection: true, auto_start: false, lead_minutes: 3.0, prep_lead_minutes: 90.0 });
        let back = read_settings(&file);
        assert_eq!((back.lead_minutes, back.prep_lead_minutes), (3.0, 90.0));
        apply_settings(&mut file, MeetingSettings { prep_lead_minutes: 1.0, ..back });
        assert_eq!(read_settings(&file).prep_lead_minutes, 5.0, "clamped on the way in");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
