//! Settings → Retention (M3.3 Task 8).

use schemars::JsonSchema;
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::error::CommandError;
use crate::retention::{RetentionPreview, RetentionRun, RetentionSettings};
use crate::state::AppState;

/// The windows, what the last sweep removed, and whether the first-run notice is behind
/// us — until it is, the schedule counts but never deletes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RetentionView {
    pub settings: RetentionSettings,
    pub last_run: Option<RetentionRun>,
    pub acknowledged: bool,
}

fn view(state: &AppState) -> RetentionView {
    let (settings, acknowledged) = {
        let file = state.settings.lock().unwrap();
        (file.retention(), file.retention_acknowledged())
    };
    RetentionView { settings, acknowledged, last_run: state.retention.as_ref().and_then(|r| r.last_run()) }
}

#[tauri::command]
pub fn retention_settings(state: tauri::State<'_, AppState>) -> RetentionView {
    view(&state)
}

/// Saving windows deliberately does *not* end the first-run hold. The card saves on
/// every keystroke, so a `3` typed on the way to `365` would otherwise arm a sweep at
/// three days. Only the two buttons that mean "delete now" end it.
#[tauri::command]
pub fn set_retention_settings(state: tauri::State<'_, AppState>, settings: RetentionSettings) -> Result<RetentionView, CommandError> {
    {
        let mut file = state.settings.lock().unwrap();
        file.set_retention(settings);
        file.save().map_err(|e| CommandError::invalid_input(format!("could not save settings: {e}")))?;
    }
    Ok(view(&state))
}

/// What a sweep would remove right now. Off the main thread: a count over the capture
/// table plus a scan of the recordings folder.
#[tauri::command]
pub async fn retention_preview(app: AppHandle) -> Result<RetentionPreview, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let retention = state.retention.as_ref().ok_or_else(|| CommandError::store("retention isn't running"))?;
        retention.preview()
    })
    .await
    .map_err(|e| CommandError::store(format!("the count failed: {e}")))?
}

/// "Delete old data now", and the first-run notice's "Keep current settings": sweeps
/// without waiting for the daily run, which also ends the first-run hold. Off the main
/// thread — it deletes rows and files.
#[tauri::command]
pub async fn retention_run_now(app: AppHandle) -> Result<RetentionView, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let retention = state.retention.as_ref().ok_or_else(|| CommandError::store("retention isn't running"))?;
        retention.run_now()?;
        Ok(view(&state))
    })
    .await
    .map_err(|e| CommandError::store(format!("the sweep failed: {e}")))?
}
