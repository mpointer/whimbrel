//! Settings → Capture & Privacy: "Keep an Obsidian vault up to date" (port of the Mac's PR 681). The
//! writing happens in `crate::topic_vault`'s loop; this is the switch, the folder and what it last did.

use schemars::JsonSchema;
use serde::Serialize;
use tauri::{AppHandle, State};

use crate::error::CommandError;
use crate::l10n::tr;
use crate::settings::SettingsFile;
use crate::state::AppState;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VaultView {
    pub enabled: bool,
    /// The chosen folder; empty until one is.
    pub path: String,
    /// When the vault was last written, after this folder was chosen.
    pub last_updated_unix_ms: Option<i64>,
    /// The last write failed: "Couldn't update the vault…".
    pub failed: bool,
}

fn view(file: &SettingsFile) -> VaultView {
    VaultView { enabled: file.vault_enabled(), path: file.vault_path(), last_updated_unix_ms: file.vault_last_updated(), failed: file.vault_failed() }
}

fn save(file: &SettingsFile) -> Result<(), CommandError> {
    file.save().map_err(|e| CommandError::invalid_input(tr("Could not save settings: {error}", &[("error", &e.to_string())])))
}

/// A new folder: no "Last updated" or failure carried over from the old one.
pub fn choose(file: &mut SettingsFile, path: &str) {
    if path != file.vault_path() {
        file.clear_vault_last_updated();
    }
    file.set_vault_path(path);
    file.set_vault_failed(false);
}

#[tauri::command]
pub fn vault_settings(state: State<'_, AppState>) -> VaultView {
    view(&state.settings.lock().unwrap())
}

/// The switch. The vault is written on the next tick (within 5 minutes).
#[tauri::command]
pub fn set_vault_enabled(state: State<'_, AppState>, enabled: bool) -> Result<VaultView, CommandError> {
    let mut file = state.settings.lock().unwrap();
    file.set_vault_enabled(enabled);
    save(&file)?;
    Ok(view(&file))
}

/// "Choose folder…": the native folder picker. Unchanged when cancelled.
#[tauri::command]
pub async fn choose_vault_folder(app: AppHandle, state: State<'_, AppState>) -> Result<VaultView, CommandError> {
    let picked = tauri::async_runtime::spawn_blocking(move || super::parented_dialog(&app, &tr("Choose folder…", &[])).blocking_pick_folder())
        .await
        .map_err(|e| CommandError::invalid_input(e.to_string()))?;
    let mut file = state.settings.lock().unwrap();
    if let Some(path) = picked.and_then(|p| p.into_path().ok()) {
        choose(&mut file, &path.display().to_string());
        save(&file)?;
    }
    Ok(view(&file))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_folder_starts_fresh_and_the_same_one_keeps_its_time() {
        let path = std::env::temp_dir().join(format!("whimbrel-vault-cmd-{}-{:?}.json", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_file(&path);
        let mut f = SettingsFile::load(&path);
        choose(&mut f, "C:\\Notes");
        f.set_vault_last_updated(5);
        f.set_vault_failed(true);
        choose(&mut f, "C:\\Notes");
        assert_eq!((f.vault_last_updated(), f.vault_failed()), (Some(5), false), "same folder: keeps its time, clears the failure");
        choose(&mut f, "D:\\Vault");
        assert_eq!((f.vault_path(), f.vault_last_updated()), ("D:\\Vault".to_string(), None));
        let _ = std::fs::remove_file(path);
    }
}
