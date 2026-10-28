//! The Library tab (plan Task 4): the interleaved cross-device timeline, episodes,
//! search, and detail views. Every row carries its device's label, which the exit gate
//! is about.
//!
//! Each command is a thin wrapper over a `*_rows` function that takes the store handle
//! and a device labeler, so the query-and-shape logic is tested against a real
//! in-memory store.

use schemars::JsonSchema;
use serde::Serialize;
use tauri::State;

use crate::device_labels::DeviceLabeler;
use crate::error::CommandError;
use crate::state::AppState;
use crate::store_actor::StoreHandle;

pub const RECENT_CAPTURES: usize = 50;
pub const RECENT_MEETINGS: usize = 20;
pub const RECENT_EPISODES: usize = 50;
/// How many recent episodes to read so hidden ones don't leave the list short.
pub const EPISODE_SCAN: usize = 400;
pub const SEARCH_LIMIT: usize = 50;
pub const SNIPPET_CHARS: usize = 280;

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceTag {
    pub device_id: String,
    pub label: String,
    pub this_device: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CaptureRow {
    pub id: i64,
    pub ts_unix_ms: i64,
    pub app: String,
    pub title: String,
    pub snippet: String,
    /// `uia`, `ocr` (Windows) or `ax` (Mac); `None` for rows older than the field.
    pub source: Option<String>,
    pub device: DeviceTag,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MeetingRow {
    pub id: i64,
    pub ts_unix_ms: i64,
    pub end_unix_ms: Option<i64>,
    pub title: String,
    pub status: String,
    /// How it started: "calendar", "detected" or "manual".
    pub trigger: String,
    pub has_summary: bool,
    pub device: DeviceTag,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EpisodeRow {
    pub id: i64,
    pub ts_unix_ms: i64,
    pub end_ts_unix_ms: i64,
    pub app: String,
    pub title: Option<String>,
    pub item_count: i64,
    pub summary: Option<String>,
    /// The core's summary state, e.g. `pending` or `summarize_failed`.
    pub status: String,
    /// Hidden on its own (synced across devices by episode identity).
    pub hidden: bool,
    /// Its app is in this PC's hidden-apps list.
    pub app_hidden: bool,
    pub device: DeviceTag,
}

/// A transcript search hit. Transcript segments carry no device of their own.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptRow {
    pub id: i64,
    pub meeting_id: i64,
    pub meeting_title: String,
    /// Offset from the meeting start, in centiseconds.
    pub start_cs: i64,
    pub speaker: String,
    pub snippet: String,
}

/// A slide-text search hit: a snapshot saved during a meeting (local to this device).
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotRow {
    /// The `items` row id, as Ask cites it.
    pub id: i64,
    pub meeting_id: i64,
    pub meeting_title: String,
    pub snippet: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LibraryRow {
    Capture(CaptureRow),
    Meeting(MeetingRow),
    Episode(EpisodeRow),
    Transcript(TranscriptRow),
    Snapshot(SnapshotRow),
}

impl LibraryRow {
    /// The time the timeline sorts by (transcript and slide hits have none and sort last).
    pub fn ts(&self) -> i64 {
        match self {
            LibraryRow::Capture(r) => r.ts_unix_ms,
            LibraryRow::Meeting(r) => r.ts_unix_ms,
            LibraryRow::Episode(r) => r.ts_unix_ms,
            LibraryRow::Transcript(_) | LibraryRow::Snapshot(_) => i64::MIN,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CaptureDetail {
    #[serde(flatten)]
    pub row: CaptureRow,
    pub url: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MeetingDetail {
    #[serde(flatten)]
    pub row: MeetingRow,
    pub attendees: Vec<String>,
    pub notes: Option<String>,
    pub summary_tldr: Option<String>,
    pub decisions: Vec<String>,
    pub action_items: Vec<String>,
    pub open_questions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySettings {
    pub show_screen_captures: bool,
    /// Apps whose episodes the Episodes list hides, on this PC only.
    pub hidden_episode_apps: Vec<String>,
}

/// Whitespace collapsed, cut at `SNIPPET_CHARS` characters.
pub fn snippet(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= SNIPPET_CHARS {
        return collapsed;
    }
    let cut: String = collapsed.chars().take(SNIPPET_CHARS).collect();
    format!("{}…", cut.trim_end())
}

pub(crate) fn tag(labeler: &DeviceLabeler, device_id: &str) -> DeviceTag {
    DeviceTag { device_id: device_id.to_string(), label: labeler.label(device_id), this_device: labeler.is_this_device(device_id) }
}

fn capture_row(item: &memory::CaptureItem, labeler: &DeviceLabeler) -> CaptureRow {
    CaptureRow {
        id: item.id,
        ts_unix_ms: item.ts_unix_ms,
        app: item.app_bundle_id.clone(),
        title: item.window_title.clone(),
        snippet: snippet(&item.text),
        source: item.source.clone(),
        device: tag(labeler, &item.device_id),
    }
}

fn status_text<T: Serialize>(status: &T) -> String {
    serde_json::to_value(status).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default()
}

pub(crate) fn meeting_row(m: &memory::Meeting, labeler: &DeviceLabeler) -> MeetingRow {
    MeetingRow {
        id: m.id,
        ts_unix_ms: m.start_unix_ms,
        end_unix_ms: m.end_unix_ms,
        title: m.title.clone(),
        status: status_text(&m.status),
        trigger: crate::commands::workspace::trigger_text(m.trigger),
        has_summary: m.summary_tldr.is_some(),
        device: tag(labeler, &m.device_id),
    }
}

fn episode_row(e: &memory::Episode, hidden: bool, hidden_apps: &[String], labeler: &DeviceLabeler) -> EpisodeRow {
    EpisodeRow {
        id: e.id,
        ts_unix_ms: e.start_ts_unix_ms,
        end_ts_unix_ms: e.end_ts_unix_ms,
        app: e.app_bundle_id.clone(),
        title: e.window_title.clone(),
        item_count: e.item_count,
        summary: e.summary.clone(),
        status: e.status.clone(),
        hidden,
        app_hidden: hidden_apps.iter().any(|a| a.eq_ignore_ascii_case(&e.app_bundle_id)),
        device: tag(labeler, &e.device_id),
    }
}

/// The newest `RECENT_EPISODES` episodes, leaving out hidden ones and those from hidden
/// apps unless `include_hidden`.
pub fn visible_episodes(
    episodes: &[memory::Episode],
    hidden_ids: &[i64],
    hidden_apps: &[String],
    include_hidden: bool,
    labeler: &DeviceLabeler,
) -> Vec<LibraryRow> {
    episodes
        .iter()
        .map(|e| episode_row(e, hidden_ids.contains(&e.id), hidden_apps, labeler))
        .filter(|r| include_hidden || !(r.hidden || r.app_hidden))
        .take(RECENT_EPISODES)
        .map(LibraryRow::Episode)
        .collect()
}

fn transcript_row(s: &memory::MeetingTranscriptSegment) -> TranscriptRow {
    TranscriptRow {
        id: s.id,
        meeting_id: s.meeting_id,
        meeting_title: s.meeting_title.clone(),
        start_cs: s.start_cs,
        speaker: s.speaker_label.clone(),
        snippet: snippet(&s.text),
    }
}

fn snapshot_row(h: &memory::MeetingSnapshotHit) -> SnapshotRow {
    SnapshotRow { id: h.id, meeting_id: h.meeting_id, meeting_title: h.meeting_title.clone(), snippet: snippet(&h.text) }
}

/// Newest first; a stable sort keeps each source's own order within a tie.
pub fn newest_first(mut rows: Vec<LibraryRow>) -> Vec<LibraryRow> {
    rows.sort_by_key(|r| std::cmp::Reverse(r.ts()));
    rows
}

fn store_error(e: impl std::fmt::Display) -> CommandError {
    CommandError::store(e.to_string())
}

pub async fn recent_rows(store: &StoreHandle, labeler: DeviceLabeler, show_captures: bool) -> Result<Vec<LibraryRow>, CommandError> {
    let (captures, meetings) = store
        .call(move |s| -> Result<_, String> {
            let captures = if show_captures { s.recent(RECENT_CAPTURES).map_err(|e| e.to_string())? } else { Vec::new() };
            Ok((captures, s.recent_meetings(RECENT_MEETINGS).map_err(|e| e.to_string())?))
        })
        .await?
        .map_err(store_error)?;
    let rows = captures
        .iter()
        .map(|c| LibraryRow::Capture(capture_row(c, &labeler)))
        .chain(meetings.iter().map(|m| LibraryRow::Meeting(meeting_row(m, &labeler))))
        .collect();
    Ok(newest_first(rows))
}

pub async fn episode_rows(
    store: &StoreHandle,
    labeler: DeviceLabeler,
    include_hidden: bool,
    hidden_apps: Vec<String>,
) -> Result<Vec<LibraryRow>, CommandError> {
    let own = labeler.own_device_id.clone().unwrap_or_default();
    let (episodes, hidden) = store
        .call(move |s| -> Result<_, String> {
            Ok((
                s.recent_episodes(EPISODE_SCAN).map_err(|e| e.to_string())?,
                s.list_hidden_episodes(&own).map_err(|e| e.to_string())?,
            ))
        })
        .await?
        .map_err(store_error)?;
    Ok(visible_episodes(&episodes, &hidden, &hidden_apps, include_hidden, &labeler))
}

/// Captures first, then transcript hits (the Mac's order), then slide hits. Semantic search returns
/// nothing until the embedding model is downloaded (Task 7).
pub async fn search_rows(store: &StoreHandle, labeler: DeviceLabeler, query: String, semantic: bool) -> Result<Vec<LibraryRow>, CommandError> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    let (captures, transcripts, slides) = store
        .call(move |s| -> Result<_, String> {
            let q = query.trim();
            if semantic {
                Ok((
                    s.search_semantic(q, SEARCH_LIMIT).map_err(|e| e.to_string())?,
                    s.search_transcript_semantic(q, SEARCH_LIMIT).map_err(|e| e.to_string())?,
                    s.search_snapshot_semantic_with_meeting_ts(q, SEARCH_LIMIT).map_err(|e| e.to_string())?,
                ))
            } else {
                Ok((
                    s.search_keyword(q, SEARCH_LIMIT).map_err(|e| e.to_string())?,
                    s.search_transcript_keyword(q, SEARCH_LIMIT).map_err(|e| e.to_string())?,
                    s.search_snapshot_keyword_with_meeting_ts(q, SEARCH_LIMIT).map_err(|e| e.to_string())?,
                ))
            }
        })
        .await?
        .map_err(store_error)?;
    Ok(captures
        .iter()
        .map(|c| LibraryRow::Capture(capture_row(c, &labeler)))
        .chain(transcripts.iter().map(|t| LibraryRow::Transcript(transcript_row(t))))
        .chain(slides.iter().map(|(h, _)| LibraryRow::Snapshot(snapshot_row(h))))
        .collect())
}

#[tauri::command]
pub async fn library_recent(state: State<'_, AppState>) -> Result<Vec<LibraryRow>, CommandError> {
    let show = state.settings.lock().unwrap().show_screen_captures();
    recent_rows(state.store()?, state.device_labeler(), show).await
}

#[tauri::command]
pub async fn library_episodes(state: State<'_, AppState>, include_hidden: bool) -> Result<Vec<LibraryRow>, CommandError> {
    let hidden_apps = state.settings.lock().unwrap().hidden_episode_apps();
    episode_rows(state.store()?, state.device_labeler(), include_hidden, hidden_apps).await
}

#[tauri::command]
pub async fn library_search(state: State<'_, AppState>, query: String, semantic: bool) -> Result<Vec<LibraryRow>, CommandError> {
    search_rows(state.store()?, state.device_labeler(), query, semantic).await
}

#[tauri::command]
pub async fn library_capture(state: State<'_, AppState>, id: i64) -> Result<Option<CaptureDetail>, CommandError> {
    let labeler = state.device_labeler();
    let item = state.store()?.call(move |s| s.get_capture_item(id).map_err(|e| e.to_string())).await?.map_err(store_error)?;
    Ok(item.map(|item| CaptureDetail { row: capture_row(&item, &labeler), url: item.url.clone(), text: item.text.clone() }))
}

#[tauri::command]
pub async fn library_meeting(state: State<'_, AppState>, id: i64) -> Result<Option<MeetingDetail>, CommandError> {
    let labeler = state.device_labeler();
    let meeting = state.store()?.call(move |s| s.get_meeting(id).map_err(|e| e.to_string())).await?.map_err(store_error)?;
    Ok(meeting.map(|m| MeetingDetail {
        row: meeting_row(&m, &labeler),
        attendees: m.attendees.clone(),
        notes: m.notes.clone(),
        summary_tldr: m.summary_tldr.clone(),
        decisions: m.summary_decisions.clone(),
        action_items: m.summary_action_items.clone(),
        open_questions: m.summary_open_questions.clone(),
    }))
}

#[tauri::command]
pub async fn library_episode(state: State<'_, AppState>, id: i64) -> Result<Option<EpisodeRow>, CommandError> {
    let labeler = state.device_labeler();
    let own = labeler.own_device_id.clone().unwrap_or_default();
    let hidden_apps = state.settings.lock().unwrap().hidden_episode_apps();
    let (episode, hidden) = state
        .store()?
        .call(move |s| -> Result<_, String> {
            Ok((s.get_episode(id).map_err(|e| e.to_string())?, s.list_hidden_episodes(&own).map_err(|e| e.to_string())?))
        })
        .await?
        .map_err(store_error)?;
    Ok(episode.map(|e| episode_row(&e, hidden.contains(&e.id), &hidden_apps, &labeler)))
}

#[tauri::command]
pub fn library_settings(state: State<'_, AppState>) -> LibrarySettings {
    let settings = state.settings.lock().unwrap();
    LibrarySettings { show_screen_captures: settings.show_screen_captures(), hidden_episode_apps: settings.hidden_episode_apps() }
}

#[tauri::command]
pub fn set_show_screen_captures(state: State<'_, AppState>, show: bool) -> Result<LibrarySettings, CommandError> {
    let mut settings = state.settings.lock().unwrap();
    settings.set_show_screen_captures(show);
    settings.save().map_err(|e| CommandError::invalid_input(format!("could not save settings: {e}")))?;
    Ok(LibrarySettings { show_screen_captures: show, hidden_episode_apps: settings.hidden_episode_apps() })
}

/// Hides or shows every episode from `app` in this PC's Episodes list.
#[tauri::command]
pub fn set_episode_app_hidden(state: State<'_, AppState>, app: String, hidden: bool) -> Result<LibrarySettings, CommandError> {
    if app.trim().is_empty() {
        return Err(CommandError::invalid_input("no app given"));
    }
    let mut settings = state.settings.lock().unwrap();
    settings.set_episode_app_hidden(app.trim(), hidden);
    settings.save().map_err(|e| CommandError::invalid_input(format!("could not save settings: {e}")))?;
    Ok(LibrarySettings { show_screen_captures: settings.show_screen_captures(), hidden_episode_apps: settings.hidden_episode_apps() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use memory::Store;
    use std::collections::HashMap;

    const THIS_PC: &str = "ThisPcPublicKey=";
    const MAC: &str = "MacPublicKey1234=";

    fn labeler() -> DeviceLabeler {
        DeviceLabeler { own_device_id: Some(THIS_PC.into()), names: HashMap::from([(MAC.to_string(), "MacBook Pro".to_string())]) }
    }

    fn record(ts: i64, app: &str, text: &str) -> ingest::CaptureRecord {
        ingest::CaptureRecord {
            ts_unix_ms: ts,
            bundle_id: app.into(),
            window_title: format!("{app} window"),
            url: None,
            text: text.into(),
            source: "uia".into(),
        }
    }

    /// Captures from this PC and from the Mac, interleaved in time.
    fn seeded_store() -> StoreHandle {
        let store = StoreHandle::spawn(|| Store::open_in_memory().map_err(|e| e.to_string())).unwrap();
        store
            .call_blocking(|s| {
                ingest::ingest_record(s, THIS_PC, &record(1_000, "notepad.exe", "first note from windows"));
                ingest::ingest_record(s, MAC, &record(2_000, "com.apple.Safari", "mac page about falcons"));
                ingest::ingest_record(s, THIS_PC, &record(3_000, "code.exe", "second note from windows"));
            })
            .unwrap();
        store
    }

    #[test]
    fn snippet_collapses_and_cuts() {
        assert_eq!(snippet("a  b\n\tc"), "a b c");
        let long = "x".repeat(SNIPPET_CHARS + 10);
        let s = snippet(&long);
        assert_eq!(s.chars().count(), SNIPPET_CHARS + 1);
        assert!(s.ends_with('…'));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn recent_interleaves_devices_newest_first_with_labels() {
        let rows = recent_rows(&seeded_store(), labeler(), true).await.unwrap();
        let summary: Vec<(i64, String, bool)> = rows
            .iter()
            .map(|r| match r {
                LibraryRow::Capture(c) => (c.ts_unix_ms, c.device.label.clone(), c.device.this_device),
                other => panic!("unexpected row {other:?}"),
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                (3_000, "This PC".to_string(), true),
                (2_000, "MacBook Pro".to_string(), false),
                (1_000, "This PC".to_string(), true),
            ]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn hiding_screen_captures_leaves_only_meetings() {
        assert!(recent_rows(&seeded_store(), labeler(), false).await.unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn keyword_search_finds_a_row_from_either_device() {
        let store = seeded_store();
        let hits = search_rows(&store, labeler(), "falcons".into(), false).await.unwrap();
        assert!(matches!(hits.as_slice(), [LibraryRow::Capture(c)] if c.device.label == "MacBook Pro"));
        // Operators and quotes are the core's to sanitize; they must not error.
        assert!(search_rows(&store, labeler(), "\"note\" -from OR (".into(), false).await.is_ok());
        assert!(search_rows(&store, labeler(), "   ".into(), false).await.unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn slide_text_is_found_as_a_slide_of_its_meeting_not_as_a_capture() {
        let store = seeded_store();
        let meeting_id = store
            .call_blocking(|s| {
                let id = s
                    .insert_meeting(&memory::NewMeeting {
                        device_id: THIS_PC.into(),
                        start_unix_ms: 5_000,
                        end_unix_ms: Some(9_000),
                        title: "Acme sync".into(),
                        attendees: vec![],
                        trigger: memory::MeetingTrigger::Manual,
                        audio_path: None,
                        status: memory::MeetingStatus::Completed,
                    })
                    .unwrap();
                s.insert_meeting_snapshot(&memory::NewMeetingSnapshot {
                    meeting_id: id,
                    captured_unix_ms: 6_000,
                    file_path: r"C:\snaps\10.00.00.png".into(),
                    width: 1920,
                    height: 1080,
                    ocr_text: "Quarterly roadmap\nHire two engineers".into(),
                })
                .unwrap();
                id
            })
            .unwrap();
        let hits = search_rows(&store, labeler(), "roadmap".into(), false).await.unwrap();
        assert!(
            matches!(hits.as_slice(), [LibraryRow::Snapshot(r)] if r.meeting_id == meeting_id && r.meeting_title == "Acme sync" && r.snippet.contains("Quarterly roadmap")),
            "{hits:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn semantic_search_without_a_model_is_empty_not_an_error() {
        assert!(search_rows(&seeded_store(), labeler(), "birds".into(), true).await.unwrap().is_empty());
    }

    fn episode(id: i64, app: &str) -> memory::Episode {
        memory::Episode {
            id,
            device_id: THIS_PC.into(),
            app_bundle_id: app.into(),
            window_title: None,
            start_ts_unix_ms: id,
            end_ts_unix_ms: id,
            item_count: 1,
            summary: None,
            status: "summarize_failed".into(),
        }
    }

    #[test]
    fn hidden_apps_and_hidden_episodes_are_left_out_unless_asked_for() {
        let episodes: Vec<_> = [(1, "WindowsTerminal!App"), (2, "code.exe"), (3, "code.exe")].map(|(id, app)| episode(id, app)).into();
        let hidden_apps = vec!["windowsterminal!app".to_string()];
        let ids = |rows: Vec<LibraryRow>| -> Vec<(i64, bool, bool)> {
            rows.into_iter()
                .map(|r| match r {
                    LibraryRow::Episode(e) => (e.id, e.hidden, e.app_hidden),
                    other => panic!("unexpected row {other:?}"),
                })
                .collect()
        };
        assert_eq!(ids(visible_episodes(&episodes, &[3], &hidden_apps, false, &labeler())), [(2, false, false)]);
        assert_eq!(
            ids(visible_episodes(&episodes, &[3], &hidden_apps, true, &labeler())),
            [(1, false, true), (2, false, false), (3, true, false)]
        );
    }

    #[test]
    fn the_list_keeps_its_length_when_hidden_episodes_are_skipped() {
        let episodes: Vec<_> = (0..RECENT_EPISODES as i64 * 2).map(|id| episode(id, if id % 2 == 0 { "noisy.exe" } else { "code.exe" })).collect();
        let rows = visible_episodes(&episodes, &[], &["noisy.exe".to_string()], false, &labeler());
        assert_eq!(rows.len(), RECENT_EPISODES);
    }

    #[test]
    fn rows_serialize_with_a_kind_tag_and_camel_case_fields() {
        let row = LibraryRow::Capture(CaptureRow {
            id: 1,
            ts_unix_ms: 5,
            app: "code.exe".into(),
            title: "t".into(),
            snippet: "s".into(),
            source: Some("uia".into()),
            device: DeviceTag { device_id: THIS_PC.into(), label: "This PC".into(), this_device: true },
        });
        let v = serde_json::to_value(row).unwrap();
        assert_eq!(v["kind"], "capture");
        assert_eq!(v["tsUnixMs"], 5);
        assert_eq!(v["device"]["thisDevice"], true);
    }
}
