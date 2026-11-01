//! Settings → Search model (plan Task 7, Decision 12): the multilingual embedding model is
//! downloaded, not bundled, into its own folder under `%LOCALAPPDATA%\Whimbrel\data\models\minilm`,
//! the store's `models_dir` (`memory::embedding_model_dir`).
//!
//! Settings → Meeting models (M3.3 Task 2, Decision 11): whisper and the two diarization
//! models, from core's manifest, into `models\`; with "I have meetings in other languages" on,
//! the multilingual whisper model too (localization W7). Each meeting loads them fresh, so no
//! restart is needed. Downloads of both kinds share the poll and cancel commands.
//!
//! `memory`'s loader doesn't remember a missing model, so the first search or embed after
//! the download loads it: no restart.

use std::path::Path;
use std::sync::Mutex;

use schemars::JsonSchema;
use serde::Serialize;
use tauri::{AppHandle, State};

use crate::error::CommandError;
use crate::state::AppState;
use crate::l10n::tr;

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    /// Every file in core's embedding manifest is on disk.
    pub installed: bool,
    /// The running store has the model loaded (it loads on the first search or embed after install).
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
    /// "I have meetings in other languages" (`transcription.multilingual`), device-local.
    pub multilingual: bool,
}

/// The multilingual model joins the manifest only while the setting is on (the Mac's
/// `ModelDownloadService.downloadAll`); off, the three files are exactly as before.
fn meeting_destinations(paths: &crate::paths::Paths, multilingual: bool) -> api::models::ModelDestinations {
    api::models::ModelDestinations {
        whisper_model_path: paths.whisper_model(),
        segmentation_model_path: paths.segmentation_model(),
        embedding_model_path: paths.speaker_embedding_model(),
        whisper_multilingual_path: multilingual.then(|| paths.whisper_multilingual_model()),
    }
}

/// The manifest's display names for the files not on disk. The downloader's own size
/// floors reject a truncated file at download time, so presence is enough here.
fn missing_models(specs: &[api::models::ModelSpec]) -> Vec<String> {
    specs.iter().filter(|s| !s.destination.is_file()).map(|s| s.name.to_string()).collect()
}

#[tauri::command]
pub fn meeting_models_status(state: State<'_, AppState>) -> MeetingModelsStatus {
    let multilingual = state.settings.lock().unwrap().multilingual();
    let missing = missing_models(&api::models::manifest(&meeting_destinations(&state.paths, multilingual)));
    MeetingModelsStatus { installed: missing.is_empty(), missing, dir: state.paths.models.display().to_string(), multilingual }
}

/// Turns "I have meetings in other languages" on or off. On, the multilingual model joins the
/// missing list until it's downloaded; meetings stay English until then. It also changes the
/// output tag (`l10n::output_tag`), as on the Mac.
#[tauri::command]
pub fn set_meeting_multilingual(state: State<'_, AppState>, on: bool) -> Result<MeetingModelsStatus, CommandError> {
    {
        let mut settings = state.settings.lock().unwrap();
        settings.set_multilingual(on);
        settings.save().map_err(|e| CommandError::invalid_input(format!("could not save settings: {e}")))?;
    }
    Ok(meeting_models_status(state))
}

/// The meeting models' download, while one runs: the launch download and Settings share it.
static MEETING_JOB: Mutex<Option<String>> = Mutex::new(None);

/// Downloads whichever meeting models are missing; poll with `model_download_poll`. A second
/// start while one runs (the launch download and Settings) joins it.
#[tauri::command]
pub fn meeting_models_download_start(state: State<'_, AppState>) -> String {
    let multilingual = state.settings.lock().unwrap().multilingual();
    running_or_start(&MEETING_JOB, still_running, || api::models::download_all_start(&meeting_destinations(&state.paths, multilingual)))
}

/// From core's manifest, so the files and their folder follow the model version.
pub fn model_installed(dir: &Path) -> bool {
    api::models::embedding_manifest(dir).iter().all(|spec| spec.destination.is_file())
}

#[tauri::command]
pub async fn model_status(state: State<'_, AppState>) -> Result<ModelStatus, CommandError> {
    let loaded = match &state.store {
        Some(store) => store.call(|s| s.embedding_model_loaded()).await?,
        None => false,
    };
    Ok(ModelStatus { installed: model_installed(&state.paths.minilm), loaded, dir: state.paths.minilm.display().to_string() })
}

/// The search model's download, while one runs: the launch download and Settings share it.
static SEARCH_JOB: Mutex<Option<String>> = Mutex::new(None);

fn still_running(id: &str) -> bool {
    api::models::download_poll(id).is_some_and(|p| !p.done)
}

/// The running job in `slot`, if `running` says it still is, else a new one from `start`.
fn running_or_start(slot: &Mutex<Option<String>>, running: impl Fn(&str) -> bool, start: impl FnOnce() -> String) -> String {
    let mut slot = slot.lock().unwrap();
    if let Some(id) = slot.as_deref().filter(|id| running(id)) {
        return id.to_string();
    }
    let id = start();
    *slot = Some(id.clone());
    id
}

#[tauri::command]
pub fn model_download_start(state: State<'_, AppState>) -> String {
    // A second start while one runs (two clicks, or the launch download and Settings) joins it.
    running_or_start(&SEARCH_JOB, still_running, || api::models::start_with_manifest(api::models::embedding_manifest(&state.paths.minilm)))
}

/// The job's progress; the job is released once it reports `done`.
#[tauri::command]
pub fn model_download_poll(job_id: String) -> Result<DownloadView, CommandError> {
    let p = api::models::download_poll(&job_id).ok_or_else(|| CommandError::invalid_input(tr("That download is no longer running.", &[])))?;
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
    fn a_second_start_joins_the_running_download() {
        let slot = Mutex::new(None);
        let mut started = 0;
        let first = running_or_start(&slot, |_| true, || {
            started += 1;
            "job-1".to_string()
        });
        let again = running_or_start(&slot, |_| true, || unreachable!("joins the running one"));
        assert_eq!((first.as_str(), again.as_str(), started), ("job-1", "job-1", 1));
        // Finished (or released): a new start is a new job.
        let next = running_or_start(&slot, |_| false, || "job-2".to_string());
        assert_eq!(next, "job-2");
    }

    #[test]
    fn meeting_models_report_each_missing_file_by_name() {
        let root = std::env::temp_dir().join(format!("whimbrel-meeting-models-{}", std::process::id()));
        let paths = crate::paths::Paths::under(&root);
        paths.ensure_dirs().unwrap();
        let specs = api::models::manifest(&meeting_destinations(&paths, false));
        assert_eq!(missing_models(&specs).len(), 3);

        std::fs::write(paths.whisper_model(), b"x").unwrap();
        std::fs::write(paths.speaker_embedding_model(), b"x").unwrap();
        assert_eq!(missing_models(&specs), ["Speaker segmentation model"]);

        std::fs::write(paths.segmentation_model(), b"x").unwrap();
        assert!(missing_models(&specs).is_empty());

        // Multilingual on: the larger whisper model is the one thing left to fetch.
        let specs = api::models::manifest(&meeting_destinations(&paths, true));
        assert_eq!(missing_models(&specs), ["Multilingual transcription model"]);
        assert_eq!(specs.last().unwrap().destination, paths.whisper_multilingual_model());
        std::fs::write(paths.whisper_multilingual_model(), b"x").unwrap();
        assert!(missing_models(&specs).is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn installed_needs_every_manifest_file_in_the_model_folder() {
        let dir = std::env::temp_dir().join(format!("whimbrel-minilm-{}", std::process::id()));
        let model = memory::embedding_model_dir(&dir);
        std::fs::create_dir_all(&model).unwrap();
        // The pre-multilingual layout, loose in models_dir, doesn't count.
        for f in memory::EMBEDDING_MODEL_FILES {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        assert!(!model_installed(&dir));
        for f in &memory::EMBEDDING_MODEL_FILES[..2] {
            std::fs::write(model.join(f), b"x").unwrap();
        }
        assert!(!model_installed(&dir), "the weights are missing");
        std::fs::write(model.join("model.safetensors"), b"x").unwrap();
        assert!(model_installed(&dir));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
