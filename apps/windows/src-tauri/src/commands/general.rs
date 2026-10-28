//! Settings → General (plan Task 10): start at login, through `tauri-plugin-autostart`'s
//! HKCU `Run` entry (docs/12 §4).

use schemars::JsonSchema;
use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_autostart::ManagerExt;

use crate::error::CommandError;
use crate::state::AppState;

/// Only release builds register themselves: a debug build would point Windows at the
/// dev exe.
pub const AUTOSTART_AVAILABLE: bool = !cfg!(debug_assertions);

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GeneralSettings {
    /// Windows starts Whimbrel at sign-in (the `Run` entry exists).
    pub autostart: bool,
    /// False in debug builds, where the toggle is read-only.
    pub autostart_available: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchAction {
    Enable,
    Disable,
    Nothing,
}

/// What a release launch does with the `Run` entry. Start at login is on by default after
/// the first launch, so an unrecorded choice counts as on. Enabling again rewrites the
/// entry, which also follows the exe to a new path.
pub fn launch_action(recorded: Option<bool>, registered: bool) -> LaunchAction {
    match (recorded.unwrap_or(true), registered) {
        (true, _) => LaunchAction::Enable,
        (false, true) => LaunchAction::Disable,
        (false, false) => LaunchAction::Nothing,
    }
}

/// Applies the recorded choice at launch, recording "on" the first time.
pub fn apply_autostart_at_launch(app: &AppHandle, state: &AppState) {
    if !AUTOSTART_AVAILABLE {
        return;
    }
    let recorded = {
        let mut settings = state.settings.lock().unwrap();
        let recorded = settings.autostart();
        if recorded.is_none() {
            settings.set_autostart(true);
            if let Err(e) = settings.save() {
                tracing::warn!(error = %e, "could not record the start-at-login default");
            }
        }
        recorded
    };
    let manager = app.autolaunch();
    let registered = manager.is_enabled().unwrap_or(false);
    let result = match launch_action(recorded, registered) {
        LaunchAction::Enable => manager.enable(),
        LaunchAction::Disable => manager.disable(),
        LaunchAction::Nothing => Ok(()),
    };
    match result {
        Ok(()) => tracing::info!(recorded = ?recorded, "start at login applied"),
        Err(e) => tracing::warn!(error = %e, "could not apply start at login"),
    }
}

#[tauri::command]
pub fn general_settings(app: AppHandle) -> GeneralSettings {
    GeneralSettings {
        autostart: AUTOSTART_AVAILABLE && app.autolaunch().is_enabled().unwrap_or(false),
        autostart_available: AUTOSTART_AVAILABLE,
    }
}

#[tauri::command]
pub fn set_autostart(app: AppHandle, state: State<'_, AppState>, enabled: bool) -> Result<GeneralSettings, CommandError> {
    if !AUTOSTART_AVAILABLE {
        return Err(CommandError::invalid_input("Start at login is only available in release builds."));
    }
    let manager = app.autolaunch();
    let result = match launch_action(Some(enabled), manager.is_enabled().unwrap_or(false)) {
        LaunchAction::Enable => manager.enable(),
        LaunchAction::Disable => manager.disable(),
        LaunchAction::Nothing => Ok(()),
    };
    result.map_err(|e| CommandError::invalid_input(format!("could not change start at login: {e}")))?;
    {
        let mut settings = state.settings.lock().unwrap();
        settings.set_autostart(enabled);
        settings.save().map_err(|e| CommandError::invalid_input(format!("could not save settings: {e}")))?;
    }
    Ok(general_settings(app))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_launch_turns_it_on_and_a_recorded_choice_sticks() {
        assert_eq!(launch_action(None, false), LaunchAction::Enable);
        assert_eq!(launch_action(Some(true), true), LaunchAction::Enable, "rewrite, in case the exe moved");
        assert_eq!(launch_action(Some(false), true), LaunchAction::Disable);
        assert_eq!(launch_action(Some(false), false), LaunchAction::Nothing);
    }
}
