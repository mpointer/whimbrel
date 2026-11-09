//! Settings → Search model (plan Task 7, Decision 12): the multilingual embedding model is
//! downloaded, not bundled, into its own folder under `%LOCALAPPDATA%\Whimbrel\data\models\minilm`,
//! the store's `models_dir` (`memory::embedding_model_dir`).
//!
//! Settings → Meeting models (M3.3 Task 2, Decision 11): whisper and the two diarization
//! models, from core's manifest, into `models\`; with "I have meetings in other languages" on,
//! the multilingual whisper model too (localization W7). Each meeting loads them fresh, so no
//! restart is needed. Downloads of both kinds share the poll and cancel commands.
//!
//! Settings → Meeting models → Quality (W-1.2): Automatic, Better or Fast, core's chooser with
//! `speech_analyzer: false`. `base` joins the meeting models for Automatic, as the floor model;
//! large-v3-turbo downloads only on request (Better, or the one offer).
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

/// The multilingual model (`base`) joins the manifest while "I have meetings in other languages"
/// is on or quality is Automatic, where it's the floor model (the Mac's
/// `ModelDownloadService.needsBase`). Otherwise the three files are exactly as before.
fn meeting_destinations(paths: &crate::paths::Paths, needs_base: bool) -> api::models::ModelDestinations {
    api::models::ModelDestinations {
        whisper_model_path: paths.whisper_model(),
        segmentation_model_path: paths.segmentation_model(),
        embedding_model_path: paths.speaker_embedding_model(),
        whisper_multilingual_path: needs_base.then(|| paths.whisper_multilingual_model()),
    }
}

fn needs_base(settings: &crate::settings::SettingsFile) -> bool {
    settings.multilingual() || settings.transcription_quality() == transcribe::engine::Quality::Automatic
}

/// The manifest's display names for the files not on disk. The downloader's own size
/// floors reject a truncated file at download time, so presence is enough here.
fn missing_models(specs: &[api::models::ModelSpec]) -> Vec<String> {
    specs.iter().filter(|s| !s.destination.is_file()).map(|s| s.name.to_string()).collect()
}

#[tauri::command]
pub fn meeting_models_status(state: State<'_, AppState>) -> MeetingModelsStatus {
    let (multilingual, base) = {
        let settings = state.settings.lock().unwrap();
        (settings.multilingual(), needs_base(&settings))
    };
    let missing = missing_models(&api::models::manifest(&meeting_destinations(&state.paths, base)));
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
    let base = needs_base(&state.settings.lock().unwrap());
    running_or_start(&MEETING_JOB, still_running, || api::models::download_all_start(&meeting_destinations(&state.paths, base)))
}

/// The Quality picker, as the UI sees it (core's `transcribe::engine::Quality`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum QualityChoice {
    Automatic,
    Better,
    Fast,
}

impl From<transcribe::engine::Quality> for QualityChoice {
    fn from(q: transcribe::engine::Quality) -> Self {
        use transcribe::engine::Quality;
        match q {
            Quality::Automatic => QualityChoice::Automatic,
            Quality::Better => QualityChoice::Better,
            Quality::Fast => QualityChoice::Fast,
        }
    }
}

impl From<QualityChoice> for transcribe::engine::Quality {
    fn from(q: QualityChoice) -> Self {
        use transcribe::engine::Quality;
        match q {
            QualityChoice::Automatic => Quality::Automatic,
            QualityChoice::Better => Quality::Better,
            QualityChoice::Fast => Quality::Fast,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionQualityStatus {
    pub quality: QualityChoice,
    /// Show the one offer of better transcripts: this PC is fast enough for large-v3-turbo,
    /// which isn't downloaded, and the offer wasn't dismissed (the Mac's `shouldOffer`).
    pub offer: bool,
    pub turbo_installed: bool,
    /// large-v3-turbo's estimated realtime speed here; None until the benchmark has run.
    pub turbo_speed: Option<f64>,
}

fn quality_status(settings: &crate::settings::SettingsFile, paths: &crate::paths::Paths) -> TranscriptionQualityStatus {
    let choice = crate::meetings::quality::choice(settings, paths, false);
    TranscriptionQualityStatus {
        quality: settings.transcription_quality().into(),
        offer: !settings.better_offer_dismissed() && choice.offer_turbo,
        turbo_installed: paths.whisper_turbo_model().is_file(),
        turbo_speed: settings.turbo_speed(),
    }
}

#[tauri::command]
pub fn transcription_quality_status(state: State<'_, AppState>) -> TranscriptionQualityStatus {
    quality_status(&state.settings.lock().unwrap(), &state.paths)
}

/// The picker. Better needs large-v3-turbo, which the UI downloads with `turbo_download_start`.
#[tauri::command]
pub fn set_transcription_quality(state: State<'_, AppState>, quality: QualityChoice) -> Result<TranscriptionQualityStatus, CommandError> {
    let mut settings = state.settings.lock().unwrap();
    settings.set_transcription_quality(quality.into());
    settings.save().map_err(|e| CommandError::invalid_input(format!("could not save settings: {e}")))?;
    Ok(quality_status(&settings, &state.paths))
}

/// "Not now", or accepting: either ends the offer for good.
#[tauri::command]
pub fn dismiss_better_offer(state: State<'_, AppState>) -> Result<TranscriptionQualityStatus, CommandError> {
    let mut settings = state.settings.lock().unwrap();
    settings.dismiss_better_offer();
    settings.save().map_err(|e| CommandError::invalid_input(format!("could not save settings: {e}")))?;
    Ok(quality_status(&settings, &state.paths))
}

/// large-v3-turbo's download, while one runs: the offer and Settings share it.
static TURBO_JOB: Mutex<Option<String>> = Mutex::new(None);

/// Downloads large-v3-turbo (about 574 MB); poll with `model_download_poll`.
#[tauri::command]
pub fn turbo_download_start(state: State<'_, AppState>) -> String {
    running_or_start(&TURBO_JOB, still_running, || api::models::start_with_manifest(vec![api::models::turbo_spec(state.paths.whisper_turbo_model())]))
}

/// Times a small model once per PC to see whether it can run large-v3-turbo (`meetings::benchmark`).
/// Does nothing once measured, with no small model on disk, or while a meeting records or is being
/// transcribed, so it never competes with one; the launch download asks again next time.
#[tauri::command]
pub async fn transcription_benchmark(state: State<'_, AppState>) -> Result<TranscriptionQualityStatus, CommandError> {
    let busy = state.meetings.as_ref().is_some_and(|m| m.is_recording()) || state.pipeline.as_ref().is_some_and(|p| p.processing().is_some());
    if state.settings.lock().unwrap().turbo_speed().is_none() && !busy {
        let paths = state.paths.clone();
        let marker = crate::meetings::benchmark::marker(&paths);
        let measured = match crate::meetings::benchmark::attempt(marker.is_file()) {
            crate::meetings::benchmark::Attempt::GiveUp => {
                // 0 keeps the turbo offer and Automatic's turbo off, and stops the launch asking again.
                tracing::warn!("the last engine benchmark never finished (the app closed during it); not running it again");
                let _ = std::fs::remove_file(&marker);
                Some(0.0)
            }
            crate::meetings::benchmark::Attempt::Run => {
                if let Err(e) = std::fs::write(&marker, b"") {
                    tracing::warn!(error = %e, "could not mark the engine benchmark as running");
                }
                let speed = tauri::async_runtime::spawn_blocking(move || crate::meetings::benchmark::estimate(&paths)).await.ok().flatten();
                let _ = std::fs::remove_file(&marker);
                speed
            }
        };
        if let Some(speed) = measured {
            let mut settings = state.settings.lock().unwrap();
            settings.set_turbo_speed(speed);
            if let Err(e) = settings.save() {
                tracing::warn!(error = %e, "could not save the benchmark");
            }
        }
    }
    Ok(quality_status(&state.settings.lock().unwrap(), &state.paths))
}

/// The small local model behind Ask (W-1.3).
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LocalModelStatus {
    /// The helper and the model are both here, and every chain ends with it.
    pub installed: bool,
    /// The helper ships in this build and the PC has the memory and the disk for it.
    pub can_offer: bool,
}

static LOCAL_MODEL_JOB: Mutex<Option<String>> = Mutex::new(None);

#[tauri::command]
pub async fn local_model_status(state: State<'_, AppState>) -> Result<LocalModelStatus, CommandError> {
    let paths = state.paths.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let installed = crate::local_model::is_installed(&paths);
        // A download that just finished: core learns of it here, as the UI asks after each job.
        if installed && !api::local_model::is_configured() {
            crate::local_model::configure(&paths);
        }
        LocalModelStatus { installed, can_offer: crate::local_model::can_offer_here(&paths) }
    })
    .await
    .map_err(|e| CommandError::store(e.to_string()))
}

/// Downloads the local model (about 1.9 GB); poll with `model_download_poll`. Never automatic.
#[tauri::command]
pub fn local_model_download_start(state: State<'_, AppState>) -> String {
    running_or_start(&LOCAL_MODEL_JOB, still_running, || api::models::start_with_manifest(vec![api::models::local_model_spec(state.paths.local_model())]))
}

/// Settings → AI → "Remove the Small Local Model".
#[tauri::command]
pub async fn local_model_remove(state: State<'_, AppState>) -> Result<LocalModelStatus, CommandError> {
    crate::local_model::remove(&state.paths).map_err(|e| CommandError::store(e.to_string()))?;
    local_model_status(state).await
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
    fn base_is_fetched_for_automatic_or_other_languages_and_not_for_fast() {
        use transcribe::engine::Quality;
        let dir = std::env::temp_dir().join(format!("whimbrel-needs-base-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = crate::settings::SettingsFile::load(&dir.join("settings.json"));
        s.set_multilingual(false);
        s.set_transcription_quality(Quality::Fast);
        assert!(!needs_base(&s), "an existing install on Fast downloads nothing new");
        s.set_transcription_quality(Quality::Automatic);
        assert!(needs_base(&s), "the floor model for Automatic");
        s.set_transcription_quality(Quality::Better);
        assert!(!needs_base(&s));
        s.set_multilingual(true);
        assert!(needs_base(&s));
        std::fs::remove_dir_all(&dir).unwrap();
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
