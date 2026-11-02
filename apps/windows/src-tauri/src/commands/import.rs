//! Import (Release 1 item 1.6, W-1.6): transcript files and exports as meetings, a folder of
//! Markdown as searchable notes. The parsing, repeats and overlaps are core's (`api::import`);
//! this file picks the files and starts the summaries.

use schemars::JsonSchema;
use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::FilePath;

use crate::error::CommandError;
use crate::l10n::tr;
use crate::meetings::pipeline::IMPORT_SUMMARY_LIMIT;
use crate::state::AppState;

/// What one import did, for the line above the Meetings list (the Mac's `ImportSummary`).
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportView {
    pub meetings: usize,
    pub notes: usize,
    /// Already here: the same meeting, or the same note unchanged.
    pub skipped: usize,
    /// Export rows skipped because a meeting recorded here ran at that time.
    pub overlapping: usize,
    /// The files that couldn't be read.
    pub failed: Vec<String>,
    /// How many of the new meetings get a summary now.
    pub summary_limit: usize,
}

fn view_of(outcome: &api::import::ImportOutcome) -> ImportView {
    ImportView {
        meetings: outcome.meeting_ids.len(),
        notes: outcome.notes,
        skipped: outcome.skipped,
        overlapping: outcome.overlapping,
        failed: outcome.failed.clone(),
        summary_limit: IMPORT_SUMMARY_LIMIT,
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn device_id(state: &AppState) -> Result<String, CommandError> {
    state
        .identity
        .as_ref()
        .map(|i| i.device_id.clone())
        .ok_or_else(|| CommandError::store(tr("Import failed.", &[])))
}

fn on_this_pc(path: FilePath) -> Result<std::path::PathBuf, CommandError> {
    path.into_path().map_err(|e| CommandError::invalid_input(tr("Not a file on this PC: {error}", &[("error", &e.to_string())])))
}

fn picker_failed(e: impl std::fmt::Display) -> CommandError {
    tracing::error!(error = %e, "the import picker stopped unexpectedly");
    CommandError::store(tr("Import failed.", &[]))
}

/// "Import → Transcripts…": `.vtt`, `.srt`, `.txt` or a Granola CSV, several at once.
/// `None` when the picker is cancelled.
#[tauri::command]
pub async fn import_transcripts(app: AppHandle, state: State<'_, AppState>) -> Result<Option<ImportView>, CommandError> {
    let device_id = device_id(&state)?;
    let picked = tauri::async_runtime::spawn_blocking(move || {
        super::parented_dialog(&app, &tr("Choose transcripts (.vtt, .srt, .txt) or a Granola CSV", &[]))
            .add_filter("Transcripts", &["vtt", "srt", "txt", "csv"])
            .blocking_pick_files()
    })
    .await
    .map_err(picker_failed)?;
    let Some(picked) = picked else { return Ok(None) };
    let paths = picked.into_iter().map(on_this_pc).collect::<Result<Vec<_>, _>>()?;
    let now = now_ms();
    let outcome = state.store()?.call(move |s| api::import::import_files(s, &device_id, &paths, now)).await?;
    if let Some(pipeline) = &state.pipeline {
        pipeline.summarize_imported(outcome.meeting_ids.clone());
    }
    Ok(Some(view_of(&outcome)))
}

/// "Import → Markdown Folder…": each note as searchable items. `None` when cancelled.
#[tauri::command]
pub async fn import_markdown_folder(app: AppHandle, state: State<'_, AppState>) -> Result<Option<ImportView>, CommandError> {
    let device_id = device_id(&state)?;
    let picked = tauri::async_runtime::spawn_blocking(move || {
        super::parented_dialog(&app, &tr("Choose a folder of Markdown notes", &[])).blocking_pick_folder()
    })
    .await
    .map_err(picker_failed)?;
    let Some(picked) = picked else { return Ok(None) };
    let folder = on_this_pc(picked)?;
    let now = now_ms();
    let outcome = state.store()?.call(move |s| api::import::import_markdown_folder(s, &device_id, &folder, now)).await?;
    Ok(Some(view_of(&outcome)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_view_counts_what_the_import_did() {
        let outcome = api::import::ImportOutcome {
            meeting_ids: vec![9, 4, 2],
            notes: 0,
            skipped: 1,
            overlapping: 2,
            failed: vec!["broken.vtt".into()],
        };
        let view = view_of(&outcome);
        assert_eq!((view.meetings, view.skipped, view.overlapping), (3, 1, 2));
        assert_eq!(view.failed, vec!["broken.vtt".to_string()]);
        assert_eq!(view.summary_limit, 25);
    }
}
