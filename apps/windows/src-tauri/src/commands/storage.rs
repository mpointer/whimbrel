//! Settings → Storage (plan 2026-11-12-storage-visibility, Windows half): what Whimbrel keeps on
//! this PC, compacting the database, the low-disk line, and Free up space. The work is in
//! `crate::storage`.

use schemars::JsonSchema;
use serde::Serialize;
use tauri::State;

use crate::error::CommandError;
use crate::state::AppState;
use crate::storage::{self, CompactView, Footprint, FreeUpOptions, LowDisk};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StorageView {
    pub footprint: Footprint,
    /// Free space inside the database file, which compacting gives back.
    pub db_free_bytes: i64,
    /// Drives under the warning line (10 GB free).
    pub low_disks: Vec<LowDisk>,
    /// The day choices for Free up space, and the default.
    pub days: Vec<u32>,
    pub default_days: u32,
}

/// The data folder's disk free space, or 0 when Windows can't say (compaction then refuses).
fn free_here(state: &AppState) -> i64 {
    storage::volume_free_bytes(&state.paths.root).unwrap_or(0)
}

/// The meeting being recorded now, which no cleanup may touch.
fn busy(state: &AppState) -> Vec<i64> {
    state.meetings.as_ref().and_then(|m| m.status().meeting_id).into_iter().collect()
}

#[tauri::command]
pub async fn storage_overview(state: State<'_, AppState>) -> Result<StorageView, CommandError> {
    let paths = state.paths.clone();
    // Walking the folders takes a moment on a big data folder: off the UI thread.
    let (footprint, low_disks) = tauri::async_runtime::spawn_blocking(move || {
        let folders = [paths.root.as_path(), paths.recordings.as_path()];
        (storage::measure(&paths), storage::low_disks(&folders, storage::volume_free_bytes, storage::LOW_DISK_BYTES))
    })
    .await
    .map_err(|e| CommandError::store(e.to_string()))?;
    let db_free_bytes = state
        .slow_store()?
        .call(|s| s.db_space().map(|d| d.free_bytes()).map_err(|e| e.to_string()))
        .await?
        .map_err(CommandError::store)?;
    Ok(StorageView { footprint, db_free_bytes, low_disks, days: storage::FREE_UP_DAYS.to_vec(), default_days: storage::FREE_UP_DEFAULT_DAYS })
}

/// "Compact database": runs whenever the disk can hold the rewrite. Holds the store until done.
#[tauri::command]
pub async fn storage_compact(state: State<'_, AppState>) -> Result<CompactView, CommandError> {
    let free = free_here(&state);
    state.store()?.call(move |s| storage::compact(s, free, true)).await?.map_err(CommandError::store)
}

/// What each Free up space row would free at `days`.
#[tauri::command]
pub async fn storage_free_up_options(state: State<'_, AppState>, days: u32) -> Result<FreeUpOptions, CommandError> {
    let (recordings, busy) = (state.paths.recordings.clone(), busy(&state));
    state.slow_store()?.call(move |s| storage::options(s, days, &recordings, &busy)).await?.map_err(CommandError::store)
}

/// After a delete: compact when it now pays off. A refusal is fine; the delete stands.
async fn compact_after(state: &AppState) -> Result<(), CommandError> {
    let free = free_here(state);
    state.store()?.call(move |s| storage::compact(s, free, false)).await?.map(|_| ()).map_err(CommandError::store)
}

/// Deletes the audio of meetings older than `days`; transcripts, notes and summaries stay.
#[tauri::command]
pub async fn storage_delete_old_recordings(state: State<'_, AppState>, days: u32) -> Result<usize, CommandError> {
    let (recordings, busy) = (state.paths.recordings.clone(), busy(&state));
    let deleted = state.store()?.call(move |s| storage::delete_old_recordings(s, days, &recordings, &busy)).await?.map_err(CommandError::store)?;
    compact_after(&state).await?;
    Ok(deleted)
}

/// Deletes screen captures older than `days` here and on the other devices.
#[tauri::command]
pub async fn storage_delete_old_captures(state: State<'_, AppState>, days: u32) -> Result<usize, CommandError> {
    let recordings = state.paths.recordings.clone();
    let deleted = state.store()?.call(move |s| storage::delete_old_captures(s, days, &recordings)).await?.map_err(CommandError::store)?;
    compact_after(&state).await?;
    Ok(deleted)
}
