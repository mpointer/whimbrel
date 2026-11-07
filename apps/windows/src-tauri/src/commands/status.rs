//! The Settings status page (#595, the Mac's `StatusOverviewView`): the two things it shows
//! that no other command reports, the sync outbox backlog and the media on disk. Everything
//! else on the page comes from the commands its rows link to.

use schemars::JsonSchema;
use serde::Serialize;
use tauri::State;

use crate::error::CommandError;
use crate::state::AppState;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StatusView {
    /// Sync operations not yet delivered.
    pub outbox_pending: i64,
    /// When the oldest of them was made, if there are any.
    pub outbox_oldest_ms: Option<i64>,
    /// Meeting audio on disk, counted from the stored paths (both tracks of each part).
    pub audio_bytes: i64,
    /// Meeting snapshot images on disk, counted from the stored paths.
    pub image_bytes: i64,
}

#[tauri::command]
pub async fn status_overview(state: State<'_, AppState>) -> Result<StatusView, CommandError> {
    let root = state.paths.root.clone();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
    // The media count reads every meeting's files, so it runs on the slow lane, off the
    // main lane the rest of the UI waits on.
    state
        .slow_store()?
        .call(move |s| {
            let backlog = s.outbox_backlog().map_err(|e| e.to_string())?;
            let usage = api::media_retention::plan(s, &api::media_retention::MediaPolicy::default(), now, &root, &[])
                .map_err(|e| e.to_string())?
                .usage;
            Ok::<_, String>(StatusView {
                outbox_pending: backlog.pending,
                outbox_oldest_ms: backlog.oldest_unix_ms,
                audio_bytes: usage.audio_full_bytes + usage.audio_compressed_bytes,
                image_bytes: usage.image_bytes,
            })
        })
        .await?
        .map_err(CommandError::store)
}
