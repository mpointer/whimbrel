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
use crate::meetings::pipeline::{transcription_notice, TranscriptionNotice};
use crate::error::CommandError;
use crate::state::AppState;
use crate::l10n::tr;

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
    /// The language the meeting was transcribed in (`meeting.spoken_language`), once known.
    pub spoken_language: Option<String>,
    /// "Transcript language" is offered: multilingual mode is on, and this PC still has the
    /// meeting's audio to transcribe again (the Mac's `MeetingOptionsMenu`).
    pub can_choose_language: bool,
    /// Why this finished meeting has no transcript, when its last run said so: it failed, or
    /// it heard nothing. The page shows it with Transcribe again (the Mac's #530).
    pub transcription: Option<TranscriptionNotice>,
    /// Why the last summary of this device's transcribed meeting failed, shown beside
    /// Regenerate. `None` once a summary succeeds.
    pub summary_failure: Option<String>,
}

/// A meeting's transcript for display: turns from the file when it's on this device,
/// else from the stored segments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptTurn {
    /// The label as stored: "Me", "Others", "Speaker 2".
    pub speaker: String,
    /// What to show: the speaker's name when one is assigned, else `speaker`.
    pub label: String,
    /// The name is a guess, shown as "Name?".
    pub guess: bool,
    /// The diarized speaker ("Speaker 2" is 1), the ones the speaker menu can name.
    pub speaker_id: Option<i32>,
    /// The named person, for "Yes, that's {name}".
    pub person_key: Option<String>,
    pub timestamp: String,
    pub text: String,
}

/// A calendar invitee, for the speaker menu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InviteeView {
    pub name: String,
    pub email: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptView {
    pub turns: Vec<TranscriptTurn>,
    /// "file" or "segments" — the segments are what a synced meeting has.
    pub source: &'static str,
    /// More segments remain beyond this page.
    pub more: bool,
    /// The speaker menu is offered: see [`can_name_speakers`].
    pub can_name: bool,
    pub invitees: Vec<InviteeView>,
}

/// A person with a remembered voice (Settings → Meetings → Remembered voices…).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VoicePersonView {
    pub person_key: String,
    pub name: String,
    pub confirmed_count: i64,
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

fn turns(raw: &str, speakers: &[memory::speaker_names::MeetingSpeaker]) -> Vec<TranscriptTurn> {
    transcribe::postprocess::parse_turns(raw)
        .into_iter()
        .map(|t| {
            let (label, guess) = memory::speaker_names::display_label(&t.speaker, speakers);
            let person_key = memory::speaker_names::labelled_speaker(&t.speaker, speakers)
                .filter(|s| s.name.is_some())
                .and_then(|s| s.person_key.clone());
            TranscriptTurn {
                speaker_id: diarized_speaker_id(&t.speaker),
                speaker: t.speaker,
                label,
                guess,
                person_key,
                timestamp: t.timestamp,
                text: t.text,
            }
        })
        .collect()
}

/// "Speaker N" (written as `Speaker {id + 1}`) is diarized speaker N-1; "Me" and "Others" aren't.
fn diarized_speaker_id(label: &str) -> Option<i32> {
    label.strip_prefix("Speaker ").and_then(|n| n.parse::<i32>().ok()).filter(|n| *n >= 1).map(|n| n - 1)
}

/// Speakers can be named in a meeting of one part: diarization restarts in each part, so
/// "Speaker 2" of a merged meeting is no one person. Parts don't sync, so a meeting from
/// another device can be named only once its recorder has named it (it then has speaker
/// rows); a multi-part one there never gets them.
fn can_name_speakers(own: bool, parts: usize, named_speakers: usize) -> bool {
    if own {
        parts <= 1
    } else {
        named_speakers > 0
    }
}

/// The meeting's speakers, invitees, and whether the menu is offered, in one store call.
async fn speaker_context(
    state: &State<'_, AppState>,
    id: i64,
    own: bool,
) -> Result<(Vec<memory::speaker_names::MeetingSpeaker>, Vec<InviteeView>, bool), CommandError> {
    state
        .store()?
        .call(move |s| -> Result<_, String> {
            let speakers = s.meeting_speakers(id).map_err(|e| e.to_string())?;
            let invitees = s
                .meeting_attendee_details(id)
                .map_err(|e| e.to_string())?
                .into_iter()
                .filter(|a| !a.name.trim().is_empty())
                .map(|a| InviteeView { name: a.name, email: a.email })
                .collect();
            let parts = s.meeting_parts(id).map_err(|e| e.to_string())?.len();
            let offered = can_name_speakers(own, parts, speakers.len());
            Ok((speakers, invitees, offered))
        })
        .await?
        .map_err(CommandError::store)
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
    let spoken_language = state
        .store()?
        .call(move |s| s.meeting_spoken_language(id).map_err(|e| e.to_string()))
        .await?
        .map_err(CommandError::store)?;
    let multilingual = state.settings.lock().unwrap().multilingual();
    let audio = match &workspace {
        api::meeting_workspace::WorkspaceState::After(note) => note.meeting.audio_path.clone(),
        _ => None,
    };
    let mut view = view_of(workspace, &labeler, own_device_id.as_deref());
    // An imported or synced meeting has its transcript as stored segments, with no file here.
    if !view.has_transcript {
        view.has_transcript = state
            .store()?
            .call(move |s| s.get_transcript_segments_page(id, 0, 0).map(|(_, total)| total > 0).unwrap_or(false))
            .await?;
    }
    view.can_choose_language = can_choose_language(multilingual, &view, has_audio(&state.paths.recordings, audio.as_deref()));
    view.spoken_language = spoken_language;
    let shows_notice = view.phase == WorkspacePhase::After && !view.read_only && !view.has_transcript && view.processing_stage.is_none();
    view.transcription = notice_for(shows_notice, audio.as_deref().map(Path::new));
    let shows_summary_failure = !view.read_only && view.has_transcript && view.processing_stage.is_none();
    view.summary_failure = summary_failure_for(shows_summary_failure, audio.as_deref().map(Path::new));
    Ok(Some(view))
}

/// The summary failure applies (`applies`) to this device's transcribed meeting that isn't
/// being worked on; its marker holds the message.
fn summary_failure_for(applies: bool, audio: Option<&Path>) -> Option<String> {
    if !applies {
        return None;
    }
    crate::meetings::pipeline::summary_failure(audio?)
}

/// The notice applies (`applies`) to this device's finished meeting that has no transcript
/// and isn't being worked on; its markers say which one. Reads only the markers.
fn notice_for(applies: bool, audio: Option<&Path>) -> Option<TranscriptionNotice> {
    if !applies {
        return None;
    }
    transcription_notice(audio?)
}

/// Whether this PC still has the recording `audio_path` names, in a form the pipeline reads.
fn has_audio(recordings: &Path, audio_path: Option<&str>) -> bool {
    audio_path
        .and_then(|audio| crate::meetings::pipeline::stamp_of(Path::new(audio)))
        .is_some_and(|stamp| crate::meetings::recorder::RecordingFiles::unfinished(recordings, &stamp, None).audio_path().is_some())
}

/// The Mac shows "Transcript language" only in multilingual mode, for a completed meeting
/// with audio. A meeting from another device has its audio there.
fn can_choose_language(multilingual: bool, view: &WorkspaceView, has_audio: bool) -> bool {
    multilingual && view.phase == WorkspacePhase::After && !view.read_only && has_audio
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
        spoken_language: None,
        can_choose_language: false,
        transcription: None,
        summary_failure: None,
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
    let own_device_id = state.identity.as_ref().map(|i| i.device_id.clone());
    let own = meeting.as_ref().is_some_and(|m| own_device_id.as_deref().is_none_or(|own| m.device_id == own));
    let (speakers, invitees, can_name) = speaker_context(&state, id, own).await?;
    let view = |turns, source, more| TranscriptView { turns, source, more, can_name, invitees: invitees.clone() };
    if let Some(path) = meeting.as_ref().and_then(|m| m.transcript_path.clone()) {
        if let Ok(raw) = std::fs::read_to_string(Path::new(&path)) {
            return Ok(view(turns(&raw, &speakers), "file", false));
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
    Ok(view(turns(&raw, &speakers), "segments", more))
}

/// The speaker menu's pick: an invitee (with their email) or a typed name, confirmed and
/// learned. Returns the transcript with the new name.
#[tauri::command]
pub async fn set_meeting_speaker(
    state: State<'_, AppState>,
    id: i64,
    speaker_id: i32,
    name: String,
    email: Option<String>,
) -> Result<TranscriptView, CommandError> {
    // The menu's Save is off for a blank name; one arriving anyway changes nothing.
    if name.trim().is_empty() {
        return meeting_transcript(state, id).await;
    }
    state
        .store()?
        .call(move |s| api::speaker_names::confirm_speaker(s, id, speaker_id, &name, email.as_deref()).map_err(|e| e.to_string()))
        .await?
        .map_err(CommandError::store)?;
    meeting_transcript(state, id).await
}

/// Whether "Trim recording…" is offered for a meeting, and its length on the transcript clock.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TrimInfo {
    pub offered: bool,
    pub length_ms: i64,
}

/// Trim is offered on the PC that recorded the meeting (it owns the row and the audio), once the
/// recording has stopped, for a meeting over a minute long (the sheet's smallest step).
pub fn trim_offered(own: bool, recording: bool, length_ms: i64) -> bool {
    own && !recording && length_ms > 60_000
}

#[tauri::command]
pub async fn meeting_trim_info(state: State<'_, AppState>, id: i64) -> Result<TrimInfo, CommandError> {
    let own_device_id = state.identity.as_ref().map(|i| i.device_id.clone());
    state
        .store()?
        .call(move |s| -> Result<TrimInfo, String> {
            let Some(meeting) = s.get_meeting(id).map_err(|e| e.to_string())? else {
                return Ok(TrimInfo { offered: false, length_ms: 0 });
            };
            let length_ms = api::meetings::meeting_length_ms(s, id).map_err(|e| e.to_string())?;
            let own = own_device_id.as_deref() == Some(meeting.device_id.as_str());
            let recording = meeting.status == memory::MeetingStatus::Recording;
            Ok(TrimInfo { offered: trim_offered(own, recording, length_ms), length_ms })
        })
        .await?
        .map_err(CommandError::store)
}

/// Trims the meeting to its first `keep_ms`: core cuts the transcript (on every device), this PC
/// shortens the audio, deletes the cut snapshots and rewrites the transcript file. False is
/// "Nothing to trim." The page then regenerates the summary as Regenerate does.
#[tauri::command]
pub async fn meeting_trim(state: State<'_, AppState>, id: i64, keep_ms: i64) -> Result<bool, CommandError> {
    let recordings = state.paths.recordings.clone();
    let trimmed = state
        .store()?
        .call(move |s| crate::meetings::trim::trim(s, &recordings, id, keep_ms))
        .await?
        .map_err(|e| CommandError::store(format!("{}: {e}", tr("Couldn't trim the recording", &[]))))?;
    Ok(trimmed.is_some())
}

/// "Yes, that's {name}": confirms the guess as the person it was shown as. A guess that
/// changed meanwhile (a sync, a rerun) is left as it now is.
#[tauri::command]
pub async fn confirm_meeting_speaker(state: State<'_, AppState>, id: i64, speaker_id: i32, person_key: String) -> Result<TranscriptView, CommandError> {
    state
        .store()?
        .call(move |s| s.confirm_meeting_speaker(id, speaker_id, &person_key).map_err(|e| e.to_string()))
        .await?
        .map_err(CommandError::store)?;
    meeting_transcript(state, id).await
}

fn voice_people_view(people: Vec<memory::speaker_names::VoicePerson>) -> Vec<VoicePersonView> {
    people
        .into_iter()
        .map(|p| VoicePersonView { person_key: p.person_key, name: p.name, confirmed_count: p.confirmed_count })
        .collect()
}

#[tauri::command]
pub async fn voice_people(state: State<'_, AppState>) -> Result<Vec<VoicePersonView>, CommandError> {
    let people = state.store()?.call(|s| s.voice_people().map_err(|e| e.to_string())).await?.map_err(CommandError::store)?;
    Ok(voice_people_view(people))
}

/// Forgets one person's voice everywhere (through sync); names in past transcripts stay.
#[tauri::command]
pub async fn forget_voice(state: State<'_, AppState>, person_key: String) -> Result<Vec<VoicePersonView>, CommandError> {
    let people = state
        .store()?
        .call(move |s| s.forget_voice(&person_key).and_then(|()| s.voice_people()).map_err(|e| e.to_string()))
        .await?
        .map_err(CommandError::store)?;
    Ok(voice_people_view(people))
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
    let pipeline = state.pipeline.as_ref().ok_or_else(|| CommandError::store(tr("The pipeline isn't running", &[])))?.clone();
    tauri::async_runtime::spawn_blocking(move || pipeline.regenerate(id, keep_edits))
        .await
        .map_err(|e| CommandError::store(tr("Regenerating failed: {error}", &[("error", &e.to_string())])))?
}

/// The per-meeting "Transcript language" (the Mac's `MeetingOptionsMenu`): stores the choice
/// (`None` is Automatic: detect again) and transcribes the meeting again in it.
#[tauri::command]
pub async fn meeting_set_spoken_language(state: State<'_, AppState>, id: i64, language: Option<String>) -> Result<(), CommandError> {
    if let Some(code) = language.as_deref() {
        if !crate::meetings::spoken::OVERRIDE_LANGUAGES.contains(&code) {
            return Err(CommandError::invalid_input(format!("not a transcript language: {code}")));
        }
    }
    let pipeline = state.pipeline.as_ref().ok_or_else(|| CommandError::store("the pipeline isn't running"))?.clone();
    if pipeline.processing().is_some() {
        return Err(CommandError::invalid_input(crate::l10n::tr(
            "Whimbrel is still working on a meeting. Try again when it's done.",
            &[],
        )));
    }
    state
        .store()?
        .call(move |s| s.set_meeting_spoken_language(id, language.as_deref()).map_err(|e| e.to_string()))
        .await?
        .map_err(CommandError::store)?;
    tauri::async_runtime::spawn_blocking(move || pipeline.transcribe_again(id))
        .await
        .map_err(|e| CommandError::store(format!("transcribing again failed: {e}")))?
}

/// Transcribe again from the meeting's notice (the Mac's #530): the same run as a language
/// change, in the meeting's stored language. `transcribe_again` refuses while another
/// meeting is being worked on, and clears the old markers.
#[tauri::command]
pub async fn meeting_transcribe_again(state: State<'_, AppState>, id: i64) -> Result<(), CommandError> {
    let pipeline = state.pipeline.as_ref().ok_or_else(|| CommandError::store(tr("The pipeline isn't running", &[])))?.clone();
    tauri::async_runtime::spawn_blocking(move || pipeline.transcribe_again(id))
        .await
        .map_err(|e| CommandError::store(format!("transcribing again failed: {e}")))?
}

/// "Questions for you": the meeting's live question alerts, in meeting order (parity item a).
#[tauri::command]
pub async fn meeting_live_alerts(state: State<'_, AppState>, id: i64) -> Result<Vec<crate::live_alerts::LiveAlertView>, CommandError> {
    state
        .store()?
        .call(move |s| s.list_live_alerts(id).map(|rows| rows.iter().map(|a| crate::live_alerts::alert_view(s, a)).collect()).map_err(|e| e.to_string()))
        .await?
        .map_err(CommandError::store)
}

/// Whether a regeneration would overwrite the user's edits, so the UI can ask first.
/// The Mac's `regenerationNeedsConflictPrompt` plus `actionItemsHaveUserInteraction`.
#[tauri::command]
pub async fn meeting_regeneration_conflict(state: State<'_, AppState>, id: i64) -> Result<bool, CommandError> {
    state.store()?.call(move |s| regeneration_conflict(s, id)).await?.map_err(CommandError::store)
}

/// Whether the user has edited the meeting's summary or touched its action items.
pub(crate) fn regeneration_conflict(s: &memory::Store, id: i64) -> Result<bool, String> {
    let draft = s.get_meeting_note_draft(id).map_err(|e| e.to_string())?;
    let items = s.list_meeting_action_items(id).map_err(|e| e.to_string())?;
    let has_override = draft
        .as_ref()
        .is_some_and(|d| d.summary_edited.is_some() || d.decisions_edited.is_some() || d.open_questions_edited.is_some());
    let touched_items = items.iter().any(|a| a.source == "user" || a.completed || a.updated_unix_ms != a.created_unix_ms);
    Ok(has_override || touched_items)
}

/// "Forget meeting": the row and its device-local files. Refused while it records.
/// Copies already synced to other devices stay (M3.3 Decision 13).
#[tauri::command]
pub async fn meeting_forget(state: State<'_, AppState>, id: i64) -> Result<(), CommandError> {
    if state.meetings.as_ref().and_then(|m| m.status().meeting_id) == Some(id) {
        return Err(CommandError::invalid_input(tr("Stop the recording before forgetting this meeting.", &[])));
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

    fn speaker(id: i32, name: &str, status: memory::speaker_names::NameStatus) -> memory::speaker_names::MeetingSpeaker {
        memory::speaker_names::MeetingSpeaker {
            speaker_id: id,
            person_key: Some(format!("{}@x.test", name.to_lowercase())),
            name: Some(name.into()),
            status: Some(status),
        }
    }

    #[test]
    fn turns_show_names_and_mark_guesses() {
        use memory::speaker_names::NameStatus;
        let raw = "[10:00:01] Me: Hi.\n[10:00:03] Speaker 1: Hello.\n[10:00:05] Speaker 2: Hey.\n[10:00:07] Speaker 3: Late.";
        let speakers = [speaker(0, "Sam", NameStatus::Confirmed), speaker(1, "Ana", NameStatus::Guess)];
        let t = turns(raw, &speakers);
        let shown: Vec<(&str, bool, Option<i32>, Option<&str>)> =
            t.iter().map(|t| (t.label.as_str(), t.guess, t.speaker_id, t.person_key.as_deref())).collect();
        assert_eq!(
            shown,
            [
                ("Me", false, None, None),
                ("Sam", false, Some(0), Some("sam@x.test")),
                ("Ana", true, Some(1), Some("ana@x.test")),
                ("Speaker 3", false, Some(2), None),
            ]
        );
        assert_eq!(t[1].speaker, "Speaker 1", "the stored label stays as written");
    }

    #[test]
    fn trim_is_offered_on_the_recording_pc_once_stopped_and_over_a_minute() {
        assert!(trim_offered(true, false, 10 * 60_000));
        assert!(!trim_offered(false, false, 10 * 60_000), "another device owns its audio");
        assert!(!trim_offered(true, true, 10 * 60_000), "not while recording");
        assert!(!trim_offered(true, false, 60_000), "nothing shorter than the sheet's minute");
    }

    #[test]
    fn only_diarized_labels_have_a_speaker_id() {
        assert_eq!(diarized_speaker_id("Speaker 1"), Some(0));
        assert_eq!(diarized_speaker_id("Speaker 0"), None);
        assert_eq!(diarized_speaker_id("Others"), None);
        assert_eq!(diarized_speaker_id("Speaker x"), None);
    }

    #[test]
    fn the_speaker_menu_needs_one_part_or_names_from_the_recorder() {
        assert!(can_name_speakers(true, 1, 0), "this PC's meeting, before naming ran");
        assert!(!can_name_speakers(true, 2, 3), "several parts: \"Speaker 2\" is no one person");
        assert!(!can_name_speakers(false, 1, 0), "another device's meeting it never named (parts don't sync)");
        assert!(can_name_speakers(false, 1, 2), "another device named it, so it was one part");
    }

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
    fn the_transcription_notice_shows_only_where_it_applies_and_only_from_a_marker() {
        let dir = std::env::temp_dir().join(format!("whimbrel-notice-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let audio = dir.join("system-audio-W.wav");
        std::fs::write(&audio, b"x").unwrap();
        assert_eq!(notice_for(true, Some(&audio)), None, "no marker, no notice");
        std::fs::write(dir.join("transcription-failed-W.marker"), "Model missing").unwrap();
        assert_eq!(notice_for(true, Some(&audio)), Some(TranscriptionNotice::Failed { message: "Model missing".into() }));
        assert_eq!(notice_for(false, Some(&audio)), None, "another device's, transcribed, or being worked on");
        assert_eq!(notice_for(true, None), None, "no audio");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failed_summary_shows_its_reason_until_one_succeeds() {
        let dir = std::env::temp_dir().join(format!("whimbrel-summary-notice-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let audio = dir.join("system-audio-S.wav");
        std::fs::write(&audio, b"x").unwrap();
        assert_eq!(summary_failure_for(true, Some(&audio)), None, "no marker");
        std::fs::write(dir.join("summary-failed-S.marker"), "No AI provider is set up.\n").unwrap();
        assert_eq!(summary_failure_for(true, Some(&audio)).as_deref(), Some("No AI provider is set up."));
        assert_eq!(summary_failure_for(false, Some(&audio)), None, "another device's, untranscribed, or being worked on");
        std::fs::remove_dir_all(&dir).unwrap();
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
        let view = turns("[08:24:45] Me: morning\n[08:24:50] Speaker 1: morning", &[]);
        assert_eq!(view.len(), 2);
        assert_eq!((view[0].speaker.as_str(), view[0].text.as_str()), ("Me", "morning"));
    }
}
