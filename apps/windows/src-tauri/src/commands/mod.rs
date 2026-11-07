//! Tauri commands: thin wrappers that send work to the store actor or a blocking
//! thread and return a DTO. Every DTO derives `JsonSchema`; `schema.rs` turns them into
//! the UI's TypeScript types.

pub mod ai;
pub mod approvals;
pub mod capture;
pub mod chat;
pub mod connectors;
pub mod general;
pub mod import;
pub mod language;
pub mod library;
pub mod mcp;
pub mod meetings;
pub mod models;
pub mod prep;
pub mod profile;
pub mod retention;
pub mod routines;
pub mod schedule;
pub mod schema;
pub mod skins;
pub mod snapshots;
pub mod status;
pub mod sync;
pub mod today;
pub mod workspace;

use schemars::JsonSchema;
use serde::Serialize;
use tauri::State;

use crate::capture::service::CaptureStatus;
use crate::error::CommandError;
use crate::settings::RoutingSettings;
use crate::state::AppState;
use crate::store_actor::QueueWaitStats;
use crate::l10n::tr;

/// A file or folder picker titled `title`, over the main window when it's open.
pub(crate) fn parented_dialog(app: &tauri::AppHandle, title: &str) -> tauri_plugin_dialog::FileDialogBuilder<tauri::Wry> {
    use tauri::Manager;
    use tauri_plugin_dialog::DialogExt;
    let dialog = app.dialog().file().set_title(title);
    match app.get_webview_window(crate::app::MAIN_WINDOW) {
        Some(window) => dialog.set_parent(&window),
        None => dialog,
    }
}

/// What Settings → About shows.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AppStatus {
    pub version: String,
    pub data_dir: String,
    /// This device's sync id (base64 public key), if the identity loaded.
    pub device_id: Option<String>,
    pub store_open: bool,
    pub embedding_model_loaded: bool,
    pub store_queue_wait: Option<QueueWaitStats>,
    pub startup_errors: Vec<String>,
}

#[tauri::command]
pub async fn app_status(state: State<'_, AppState>) -> Result<AppStatus, CommandError> {
    let embedding_model_loaded = match &state.store {
        Some(store) => store.call(|s| s.embedding_model_loaded()).await?,
        None => false,
    };
    Ok(AppStatus {
        version: env!("CARGO_PKG_VERSION").to_string(),
        data_dir: state.paths.root.display().to_string(),
        device_id: state.identity.as_ref().map(|i| i.device_id.clone()),
        store_open: state.store.is_some(),
        embedding_model_loaded,
        store_queue_wait: state.store.as_ref().map(|s| s.queue_wait()),
        startup_errors: state.startup_errors.clone(),
    })
}

#[tauri::command]
pub fn capture_status(state: State<'_, AppState>) -> CaptureStatus {
    match &state.capture {
        Some(capture) => capture.status(),
        None => CaptureStatus {
            enabled: state.settings.lock().unwrap().capture_enabled(),
            running: false,
            last_error: Some("capture did not start; see About for the startup error".into()),
            ..Default::default()
        },
    }
}

#[tauri::command]
pub fn set_capture_enabled(state: State<'_, AppState>, enabled: bool) -> Result<CaptureStatus, CommandError> {
    {
        let mut settings = state.settings.lock().unwrap();
        settings.set_capture_enabled(enabled);
        settings.save().map_err(|e| CommandError::invalid_input(tr("Could not save settings: {error}", &[("error", &e.to_string())])))?;
    }
    if let Some(capture) = &state.capture {
        capture.set_enabled(enabled);
    }
    Ok(capture_status(state))
}

#[tauri::command]
pub fn get_routing_settings(state: State<'_, AppState>) -> RoutingSettings {
    state.settings.lock().unwrap().routing()
}

#[tauri::command]
pub fn set_routing_settings(state: State<'_, AppState>, routing: RoutingSettings) -> Result<(), CommandError> {
    {
        let mut settings = state.settings.lock().unwrap();
        crate::syncing::settings::edit(&mut settings, |f| f.set_routing(&routing));
        settings.save().map_err(|e| CommandError::invalid_input(tr("Could not save settings: {error}", &[("error", &e.to_string())])))?;
    }
    // A newly chosen Ollama model: is it installed, or does a stand-in cover it?
    crate::ollama_default::refresh_soon(state.settings.clone());
    Ok(())
}
