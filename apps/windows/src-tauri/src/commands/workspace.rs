//! The Meetings tab's commands (M3.3 Task 5): the meeting list, the Before / During /
//! After workspace, its edits, and Forget.
//!
//! The state itself is core's (`api::meeting_workspace::derive_state`), so this file only
//! shapes it for the UI and carries the shell's own signals — which meeting is recording
//! and which one the pipeline is working on.

use std::path::Path;

use memory::MeetingStatus;
use schemars::JsonSchema;
use serde::Serialize;
use tauri::State;

use super::library::{meeting_row, MeetingRow};
use crate::error::CommandError;
use crate::state::AppState;

/// The workspace's phase, matching core's `WorkspaceState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkspacePhase {
    Before,
    During,
    After,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ActionItemView {
    pub id: i64,
    pub text: String,
    pub completed: bool,
    /// "generated" (from the summary) or "user".
    pub source: String,
}

/// One meeting's workspace: what to show, and which parts the user has edited.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceView {
    pub phase: WorkspacePhase,
    pub row: MeetingRow,
    pub attendees: Vec<String>,
    pub trigger: String,
    /// The meeting app or conference link, when the row has one.
    pub conference_link: Option<String>,
    /// Seconds recorded so far, while this process is the one recording.
    pub elapsed_seconds: Option<i64>,
    /// "Transcribing…" and friends, while the pipeline is on this meeting.
    pub processing_stage: Option<String>,
    pub summary: Option<String>,
    pub summary_is_edited: bool,
    pub decisions: Vec<String>,
    pub decisions_are_edited: bool,
    pub open_questions: Vec<String>,
    pub open_questions_are_edited: bool,
    pub personal_notes: String,
    pub action_items: Vec<ActionItemView>,
    pub has_transcript: bool,
    /// A meeting recorded on another device: its audio isn't here, so it can't be
    /// re-summarized, and Forget would only remove this device's copy.
    pub read_only: bool,
}

/// A meeting's transcript for display: turns from the file when it's on this device,
/// else from the stored segments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptTurn {
    pub speaker: String,
    pub timestamp: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptView {
    pub turns: Vec<TranscriptTurn>,
    /// "file" or "segments" — the segments are what a synced meeting has.
    pub source: &'static str,
    /// More segments remain beyond this page.
    pub more: bool,
}

/// How many transcript segments one page fetches (the Mac's page size).
pub const TRANSCRIPT_PAGE: usize = 500;

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// How the meeting started, for the list's icon and the workspace's header.
pub(crate) fn trigger_text(trigger: memory::MeetingTrigger) -> String {
    match trigger {
        memory::MeetingTrigger::Calendar => "calendar",
        memory::MeetingTrigger::Detected => "detected",
        memory::MeetingTrigger::Manual => "manual",
        // Away Mode records on the Mac only, and those meetings don't sync.
        memory::MeetingTrigger::Away => "away",
    }
    .to_string()
}

fn turns(raw: &str) -> Vec<TranscriptTurn> {
    transcribe::postprocess::parse_turns(raw)
        .into_iter()
        .map(|t| TranscriptTurn { speaker: t.speaker, timestamp: t.timestamp, text: t.text })
        .collect()
}

/// The meeting list: newest first, optionally filtered by title (the Mac's search box).
#[tauri::command]
pub async fn meeting_list(state: State<'_, AppState>, filter: Option<String>) -> Result<Vec<MeetingRow>, CommandError> {
    let labeler = state.device_labeler();
    let needle = filter.unwrap_or_default().trim().to_lowercase();
    let meetings = state
        .store()?
        .call(move |s| s.recent_meetings(crate::meetings::controller::RECONCILE_WINDOW).map_err(|e| e.to_string()))
        .await?
        .map_err(CommandError::store)?;
    Ok(meetings
        .iter()
        .filter(|m| needle.is_empty() || m.title.to_lowercase().contains(&needle))
        .map(|m| meeting_row(m, &labeler))
        .collect())
}

#[tauri::command]
pub async fn meeting_workspace(state: State<'_, AppState>, id: i64) -> Result<Option<WorkspaceView>, CommandError> {
    let labeler = state.device_labeler();
    let own_device_id = state.identity.as_ref().map(|i| i.device_id.clone());
    let recording = state.meetings.as_ref().map(|m| m.status());
    let processing = state.pipeline.as_ref().and_then(|p| p.processing());
    let signals = api::meeting_workspace::Signals {
        active_meeting_id: recording.as_ref().and_then(|r| r.meeting_id),
        processing_meeting_id: processing.as_ref().map(|p| p.meeting_id),
        processing_stage: processing.as_ref().map(|p| p.label.clone()),
        now_unix_ms: now_ms(),
    };
    let state_and_items = state
        .store()?
        .call(move |s| api::meeting_workspace::load_state(s, id, &signals).map_err(|e| e.to_string()))
        .await?
        .map_err(CommandError::store)?;
    let Some(workspace) = state_and_items else { return Ok(None) };
    Ok(Some(view_of(workspace, &labeler, own_device_id.as_deref())))
}

fn view_of(
    state: api::meeting_workspace::WorkspaceState,
    labeler: &crate::device_labels::DeviceLabeler,
    own_device_id: Option<&str>,
) -> WorkspaceView {
    use api::meeting_workspace::WorkspaceState;
    let (phase, meeting, elapsed_seconds, stage, note) = match state {
        WorkspaceState::Before { meeting } => (WorkspacePhase::Before, meeting, None, None, None),
        WorkspaceState::Cancelled { meeting } => (WorkspacePhase::Cancelled, meeting, None, None, None),
        WorkspaceState::During { meeting, elapsed_seconds, processing_stage, .. } => {
            (WorkspacePhase::During, meeting, Some(elapsed_seconds), processing_stage, None)
        }
        WorkspaceState::After(note) => (
            WorkspacePhase::After,
            note.meeting.clone(),
            None,
            note.processing_stage.clone(),
            Some(note),
        ),
    };
    let read_only = own_device_id.is_some_and(|own| meeting.device_id != own);
    let action_items = note
        .as_ref()
        .map(|n| {
            n.action_items
                .iter()
                .map(|a| ActionItemView { id: a.id, text: a.text.clone(), completed: a.completed, source: a.source.clone() })
                .collect()
        })
        .unwrap_or_default();
    WorkspaceView {
        phase,
        row: meeting_row(&meeting, labeler),
        attendees: meeting.attendees.clone(),
        trigger: trigger_text(meeting.trigger),
        conference_link: api::connectors::calendar::detect_conference_link(meeting.notes.as_deref().unwrap_or("")).map(|l| l.url),
        elapsed_seconds,
        processing_stage: stage,
        summary: note.as_ref().and_then(|n| n.display_summary().map(str::to_string)),
        summary_is_edited: note.as_ref().is_some_and(|n| n.summary_is_user_edited()),
        decisions: note.as_ref().map(|n| n.display_decisions().to_vec()).unwrap_or_default(),
        decisions_are_edited: note.as_ref().is_some_and(|n| n.decisions_are_user_edited()),
        open_questions: note.as_ref().map(|n| n.display_open_questions().to_vec()).unwrap_or_default(),
        open_questions_are_edited: note.as_ref().is_some_and(|n| n.open_questions_are_user_edited()),
        personal_notes: note.as_ref().map(|n| n.personal_notes().to_string()).unwrap_or_default(),
        action_items,
        has_transcript: meeting.transcript_path.is_some(),
        read_only,
    }
}

/// The meeting's transcript: its file if this device has it, else the stored segments
/// (all a synced meeting has).
#[tauri::command]
pub async fn meeting_transcript(state: State<'_, AppState>, id: i64) -> Result<TranscriptView, CommandError> {
    let meeting = state
        .store()?
        .call(move |s| s.get_meeting(id).map_err(|e| e.to_string()))
        .await?
        .map_err(CommandError::store)?;
    if let Some(path) = meeting.as_ref().and_then(|m| m.transcript_path.clone()) {
        if let Ok(raw) = std::fs::read_to_string(Path::new(&path)) {
            return Ok(TranscriptView { turns: turns(&raw), source: "file", more: false });
        }
    }
    let (segments, total) = state
        .store()?
        .call(move |s| s.get_transcript_segments_page(id, 0, TRANSCRIPT_PAGE).map_err(|e| e.to_string()))
        .await?
        .map_err(CommandError::store)?;
    let more = total > segments.len();
    let raw = segments
        .iter()
        .map(|s| format!("[{}] {}: {}", centiseconds_label(s.start_cs), s.speaker_label, s.text))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(TranscriptView { turns: turns(&raw), source: "segments", more })
}

/// `HH:MM:SS` from a segment's offset — a synced meeting has no file to take wall-clock
/// times from, so its transcript shows time into the meeting.
fn centiseconds_label(start_cs: i64) -> String {
    let seconds = start_cs / 100;
    format!("{:02}:{:02}:{:02}", seconds / 3600, (seconds % 3600) / 60, seconds % 60)
}

#[tauri::command]
pub async fn meeting_save_notes(state: State<'_, AppState>, id: i64, notes: String) -> Result<(), CommandError> {
    state
        .store()?
        .call(move |s| s.save_meeting_personal_notes(id, &notes, now_ms()).map_err(|e| e.to_string()))
        .await?
        .map_err(CommandError::store)
}

/// Saves the user's edits to the generated summary. `None` for a field means "no
/// override": the generated value shows again.
#[tauri::command]
pub async fn meeting_save_overrides(
    state: State<'_, AppState>,
    id: i64,
    summary: Option<String>,
    decisions: Option<Vec<String>>,
    open_questions: Option<Vec<String>>,
) -> Result<(), CommandError> {
    state
        .store()?
        .call(move |s| {
            s.save_meeting_note_overrides(id, summary.as_deref(), decisions.as_deref(), open_questions.as_deref(), now_ms())
                .map_err(|e| e.to_string())
        })
        .await?
        .map_err(CommandError::store)
}

/// Drops the user's summary overrides, so the generated text shows again. Core keeps an
/// override until it is reset (its save COALESCEs a `None` to what's already there), so
/// this is the only way back to the generated summary.
#[tauri::command]
pub async fn meeting_reset_overrides(state: State<'_, AppState>, id: i64) -> Result<(), CommandError> {
    state
        .store()?
        .call(move |s| s.reset_meeting_note_draft(id, now_ms()).map_err(|e| e.to_string()))
        .await?
        .map_err(CommandError::store)
}

#[tauri::command]
pub async fn meeting_seed_action_items(state: State<'_, AppState>, id: i64) -> Result<Vec<ActionItemView>, CommandError> {
    let store = state.store()?.clone();
    store
        .call(move |s| {
            let meeting = s.get_meeting(id).map_err(|e| e.to_string())?;
            let generated = meeting.map(|m| m.summary_action_items).unwrap_or_default();
            // Idempotent in core: it seeds only when the meeting has no items yet.
            s.seed_meeting_action_items_from_generated(id, &generated, now_ms()).map_err(|e| e.to_string())?;
            action_items(s, id)
        })
        .await?
        .map_err(CommandError::store)
}

fn action_items(s: &memory::Store, id: i64) -> Result<Vec<ActionItemView>, String> {
    Ok(s.list_meeting_action_items(id)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|a| ActionItemView { id: a.id, text: a.text, completed: a.completed, source: a.source })
        .collect())
}

#[tauri::command]
pub async fn meeting_add_action_item(state: State<'_, AppState>, id: i64, text: String) -> Result<Vec<ActionItemView>, CommandError> {
    state
        .store()?
        .call(move |s| {
            let existing = s.list_meeting_action_items(id).map_err(|e| e.to_string())?;
            // Never `existing.len()`: a mid-list delete leaves gaps, and two rows sharing
            // an order index sort undefined (the Mac's `nextActionItemOrderIndex`).
            let next = existing.iter().map(|a| a.order_index).max().unwrap_or(-1) + 1;
            s.insert_meeting_action_item(id, text.trim(), next, now_ms()).map_err(|e| e.to_string())?;
            action_items(s, id)
        })
        .await?
        .map_err(CommandError::store)
}

#[tauri::command]
pub async fn meeting_update_action_item(
    state: State<'_, AppState>,
    id: i64,
    item_id: i64,
    text: Option<String>,
    completed: Option<bool>,
) -> Result<Vec<ActionItemView>, CommandError> {
    state
        .store()?
        .call(move |s| {
            if let Some(text) = text {
                s.update_meeting_action_item_text(item_id, text.trim(), now_ms()).map_err(|e| e.to_string())?;
            }
            if let Some(completed) = completed {
                s.set_meeting_action_item_completed(item_id, completed, now_ms()).map_err(|e| e.to_string())?;
            }
            action_items(s, id)
        })
        .await?
        .map_err(CommandError::store)
}

#[tauri::command]
pub async fn meeting_delete_action_item(state: State<'_, AppState>, id: i64, item_id: i64) -> Result<Vec<ActionItemView>, CommandError> {
    state
        .store()?
        .call(move |s| {
            s.delete_meeting_action_item(item_id).map_err(|e| e.to_string())?;
            action_items(s, id)
        })
        .await?
        .map_err(CommandError::store)
}

#[tauri::command]
pub async fn meeting_reorder_action_items(
    state: State<'_, AppState>,
    id: i64,
    ordered_ids: Vec<i64>,
) -> Result<Vec<ActionItemView>, CommandError> {
    state
        .store()?
        .call(move |s| {
            s.reorder_meeting_action_items(id, &ordered_ids, now_ms()).map_err(|e| e.to_string())?;
            action_items(s, id)
        })
        .await?
        .map_err(CommandError::store)
}

/// Regenerates the summary from the transcript. `keep_edits` decides what happens to the
/// user's overrides: keep them and mark the new generation seen, or drop them and take
/// the new summary and its action items (the Mac's conflict sheet).
#[tauri::command]
pub async fn meeting_regenerate(state: State<'_, AppState>, id: i64, keep_edits: bool) -> Result<(), CommandError> {
    let pipeline = state.pipeline.as_ref().ok_or_else(|| CommandError::store("the pipeline isn't running"))?.clone();
    tauri::async_runtime::spawn_blocking(move || pipeline.regenerate(id, keep_edits))
        .await
        .map_err(|e| CommandError::store(format!("regenerating failed: {e}")))?
}

/// Whether a regeneration would overwrite the user's edits, so the UI can ask first.
/// The Mac's `regenerationNeedsConflictPrompt` plus `actionItemsHaveUserInteraction`.
#[tauri::command]
pub async fn meeting_regeneration_conflict(state: State<'_, AppState>, id: i64) -> Result<bool, CommandError> {
    state
        .store()?
        .call(move |s| {
            let draft = s.get_meeting_note_draft(id).map_err(|e| e.to_string())?;
            let items = s.list_meeting_action_items(id).map_err(|e| e.to_string())?;
            let has_override = draft.as_ref().is_some_and(|d| {
                d.summary_edited.is_some() || d.decisions_edited.is_some() || d.open_questions_edited.is_some()
            });
            let touched_items = items
                .iter()
                .any(|a| a.source == "user" || a.completed || a.updated_unix_ms != a.created_unix_ms);
            Ok::<bool, String>(has_override || touched_items)
        })
        .await?
        .map_err(CommandError::store)
}

/// "Forget meeting": the row and its device-local files. Refused while it records.
/// Copies already synced to other devices stay (M3.3 Decision 13).
#[tauri::command]
pub async fn meeting_forget(state: State<'_, AppState>, id: i64) -> Result<(), CommandError> {
    if state.meetings.as_ref().and_then(|m| m.status().meeting_id) == Some(id) {
        return Err(CommandError::invalid_input("Stop the recording before forgetting this meeting."));
    }
    let recordings = state.paths.recordings.clone();
    // Files only after the row is gone, the Mac's order: a failed delete must never
    // leave a meeting pointing at files that aren't there.
    let deleted = state
        .store()?
        .call(move |s| s.delete_meeting_returning_files(id).map_err(|e| e.to_string()))
        .await?
        .map_err(CommandError::store)?;
    // Every part's audio and transcript, and the snapshot folder unless the meeting is
    // kept (core leaves `snapshots_dir` unset then, and only removes a folder it marked).
    if let Some(files) = deleted {
        api::retention::delete_all_meeting_files(&files, &recordings);
    }
    Ok(())
}

/// Which meeting the Today tab shows: the one being recorded, else today's next
/// scheduled one, else the most recent (the Mac's `TodayView` choice). Pure.
pub fn most_relevant(meetings: &[memory::Meeting], active_id: Option<i64>, now_unix_ms: i64) -> Option<i64> {
    if let Some(active) = active_id {
        if meetings.iter().any(|m| m.id == active) {
            return Some(active);
        }
    }
    let day_end = now_unix_ms + 86_400_000;
    let next_scheduled = meetings
        .iter()
        .filter(|m| m.status == MeetingStatus::Scheduled && m.start_unix_ms >= now_unix_ms && m.start_unix_ms < day_end)
        .min_by_key(|m| m.start_unix_ms);
    next_scheduled.or_else(|| meetings.iter().max_by_key(|m| m.start_unix_ms)).map(|m| m.id)
}

#[tauri::command]
pub async fn meeting_most_relevant(state: State<'_, AppState>) -> Result<Option<i64>, CommandError> {
    let active = state.meetings.as_ref().and_then(|m| m.status().meeting_id);
    state
        .store()?
        .call(move |s| {
            let meetings = s.recent_meetings(50).map_err(|e| e.to_string())?;
            Ok::<_, String>(most_relevant(&meetings, active, now_ms()))
        })
        .await?
        .map_err(CommandError::store)
}

#[cfg(test)]
mod tests {
    use super::*;
    use memory::{MeetingTrigger, NewMeeting, Store};

    fn store_with(rows: &[(MeetingStatus, i64)]) -> (Store, Vec<i64>) {
        let store = Store::open_in_memory().unwrap();
        let ids = rows
            .iter()
            .map(|(status, start)| {
                store
                    .insert_meeting(&NewMeeting {
                        device_id: "this-pc".into(),
                        start_unix_ms: *start,
                        end_unix_ms: None,
                        title: "Standup".into(),
                        attendees: Vec::new(),
                        trigger: MeetingTrigger::Manual,
                        audio_path: None,
                        status: *status,
                    })
                    .unwrap()
            })
            .collect();
        (store, ids)
    }

    #[test]
    fn today_shows_the_recording_meeting_first() {
        let (store, ids) = store_with(&[(MeetingStatus::Recording, 1_000), (MeetingStatus::Completed, 9_000)]);
        let meetings = store.recent_meetings(10).unwrap();
        assert_eq!(most_relevant(&meetings, Some(ids[0]), 5_000), Some(ids[0]));
    }

    #[test]
    fn then_todays_next_scheduled_meeting_then_the_latest() {
        let now = 1_000_000;
        let (store, ids) = store_with(&[
            (MeetingStatus::Completed, now - 10_000),
            (MeetingStatus::Scheduled, now + 3_600_000),
            (MeetingStatus::Scheduled, now + 600_000),
            (MeetingStatus::Scheduled, now + 200_000_000),
        ]);
        let meetings = store.recent_meetings(10).unwrap();
        assert_eq!(most_relevant(&meetings, None, now), Some(ids[2]), "the soonest one still to come today");

        let (store, ids) = store_with(&[(MeetingStatus::Completed, now - 10_000), (MeetingStatus::Completed, now - 5_000)]);
        let meetings = store.recent_meetings(10).unwrap();
        assert_eq!(most_relevant(&meetings, None, now), Some(ids[1]), "otherwise the most recent");
        assert_eq!(most_relevant(&[], None, now), None);
    }

    #[test]
    fn a_stale_active_id_is_ignored() {
        let (store, ids) = store_with(&[(MeetingStatus::Completed, 9_000)]);
        let meetings = store.recent_meetings(10).unwrap();
        assert_eq!(most_relevant(&meetings, Some(404), 10_000), Some(ids[0]));
    }

    #[test]
    fn segment_offsets_become_clock_labels() {
        assert_eq!(centiseconds_label(0), "00:00:00");
        assert_eq!(centiseconds_label(9_050), "00:01:30");
        assert_eq!(centiseconds_label(3_600 * 100 + 100), "01:00:01");
    }

    #[test]
    fn a_transcript_parses_into_turns() {
        let view = turns("[08:24:45] Me: morning\n[08:24:50] Speaker 1: morning");
        assert_eq!(view.len(), 2);
        assert_eq!((view[0].speaker.as_str(), view[0].text.as_str()), ("Me", "morning"));
    }
}
