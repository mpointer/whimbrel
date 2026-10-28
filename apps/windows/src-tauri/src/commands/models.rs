//! Settings → Search model (plan Task 7, Decision 12): the MiniLM embedding model is
//! downloaded, not bundled, into `%LOCALAPPDATA%\Whimbrel\data\models\minilm`, the store's
//! `models_dir`.
//!
//! Settings → Meeting models (M3.3 Task 2, Decision 11): whisper and the two diarization
//! models, from core's manifest, into `models\`. Each meeting loads them fresh, so no
//! restart is needed. Downloads of both kinds share the poll and cancel commands.
//!
//! `memory`'s loader caches a failed load for the life of the process, so a store that
//! opened before the download keeps reporting "no model". The UI therefore offers a
//! restart once the download finishes.

use std::path::Path;

use schemars::JsonSchema;
use serde::Serialize;
use tauri::{AppHandle, State};

use crate::error::CommandError;
use crate::state::AppState;

const MODEL_FILES: [&str; 3] = ["config.json", "tokenizer.json", "model.safetensors"];

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    /// All three files are on disk.
    pub installed: bool,
    /// The running store has the model loaded (false until a restart after install).
    pub loaded: bool,
    pub dir: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DownloadView {
    pub job_id: String,
    pub fraction: f64,
    pub status: String,
    pub done: bool,
    pub cancelled: bool,
    pub error: Option<String>,
}

/// Which meeting models are on disk.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MeetingModelsStatus {
    pub installed: bool,
    /// Display names of the models still missing.
    pub missing: Vec<String>,
    pub dir: String,
}

fn meeting_destinations(paths: &crate::paths::Paths) -> api::models::ModelDestinations {
    api::models::ModelDestinations {
        whisper_model_path: paths.whisper_model(),
        segmentation_model_path: paths.segmentation_model(),
        embedding_model_path: paths.speaker_embedding_model(),
    }
}

/// The manifest's display names for the files not on disk. The downloader's own size
/// floors reject a truncated file at download time, so presence is enough here.
fn missing_models(specs: &[api::models::ModelSpec]) -> Vec<String> {
    specs.iter().filter(|s| !s.destination.is_file()).map(|s| s.name.to_string()).collect()
}

#[tauri::command]
pub fn meeting_models_status(state: State<'_, AppState>) -> MeetingModelsStatus {
    let missing = missing_models(&api::models::manifest(&meeting_destinations(&state.paths)));
    MeetingModelsStatus { installed: missing.is_empty(), missing, dir: state.paths.models.display().to_string() }
}

/// Downloads whichever meeting models are missing; poll with `model_download_poll`.
#[tauri::command]
pub fn meeting_models_download_start(state: State<'_, AppState>) -> String {
    api::models::download_all_start(&meeting_destinations(&state.paths))
}

pub fn model_installed(dir: &Path) -> bool {
    MODEL_FILES.iter().all(|f| dir.join(f).is_file())
}

#[tauri::command]
pub async fn model_status(state: State<'_, AppState>) -> Result<ModelStatus, CommandError> {
    let loaded = match &state.store {
        Some(store) => store.call(|s| s.embedding_model_loaded()).await?,
        None => false,
    };
    Ok(ModelStatus { installed: model_installed(&state.paths.minilm), loaded, dir: state.paths.minilm.display().to_string() })
}

#[tauri::command]
pub fn model_download_start(state: State<'_, AppState>) -> String {
    api::models::start_with_manifest(api::models::embedding_manifest(&state.paths.minilm))
}

/// The job's progress; the job is released once it reports `done`.
#[tauri::command]
pub fn model_download_poll(job_id: String) -> Result<DownloadView, CommandError> {
    let p = api::models::download_poll(&job_id).ok_or_else(|| CommandError::invalid_input("That download is no longer running."))?;
    if p.done {
        api::models::download_release(&job_id);
    }
    Ok(DownloadView { job_id, fraction: p.fraction, status: p.status, done: p.done, cancelled: p.cancelled, error: p.error })
}

#[tauri::command]
pub fn model_download_cancel(job_id: String) -> bool {
    api::models::download_cancel(&job_id)
}

/// Relaunches Whimbrel so the store loads the newly installed model. Under the supervisor
/// that means exiting with `RESTART_REQUESTED`: Tauri's own restart would start a new
/// process the supervisor doesn't know about.
#[tauri::command]
pub fn restart_app(app: AppHandle) {
    tracing::info!("restarting to load the search model");
    if crate::restart::is_supervised() {
        crate::restart::exit_with(&app, crate::restart::RESTART_REQUESTED);
    } else {
        app.restart();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meeting_models_report_each_missing_file_by_name() {
        let root = std::env::temp_dir().join(format!("whimbrel-meeting-models-{}", std::process::id()));
        let paths = crate::paths::Paths::under(&root);
        paths.ensure_dirs().unwrap();
        let specs = api::models::manifest(&meeting_destinations(&paths));
        assert_eq!(missing_models(&specs).len(), 3);

        std::fs::write(paths.whisper_model(), b"x").unwrap();
        std::fs::write(paths.speaker_embedding_model(), b"x").unwrap();
        assert_eq!(missing_models(&specs), ["Speaker segmentation model"]);

        std::fs::write(paths.segmentation_model(), b"x").unwrap();
        assert!(missing_models(&specs).is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn installed_needs_all_three_files() {
        let dir = std::env::temp_dir().join(format!("whimbrel-minilm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!model_installed(&dir));
        for f in &MODEL_FILES[..2] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        assert!(!model_installed(&dir), "the weights are missing");
        std::fs::write(dir.join("model.safetensors"), b"x").unwrap();
        assert!(model_installed(&dir));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
