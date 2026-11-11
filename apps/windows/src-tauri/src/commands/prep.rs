//! Meeting Prep on Today (docs/superpowers/plans/2026-10-15-meeting-prep-windows.md): the brief for
//! the next calendar meeting, whether this PC made it or another device's synced in. The run itself
//! is `routines::prep_tick`; this only reads.

use api::connectors::calendar::QualifyingMeeting;
use llm::meeting_prep::{Brief, BriefSource};
use memory::prep::PrepSourceKind;
use schemars::JsonSchema;
use serde::Serialize;
use tauri::State;

use crate::commands::topic_page::TopicRefView;
use crate::error::CommandError;
use crate::state::AppState;

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PrepAttendee {
    pub who: String,
    pub note: String,
    pub sources: Vec<String>,
}

/// An open item or question and the ids of the sources it rests on.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PrepClaim {
    pub text: String,
    pub sources: Vec<String>,
}

/// A source the brief cites: the id the claims name ("M1", "E2") and what it was.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PrepSourceView {
    pub id: String,
    pub title: String,
    /// The meeting on **this** PC the source is, when it provably is one (see `local_meeting`); the
    /// chip opens it. Absent for an email, and for a meeting this PC cannot vouch for.
    pub meeting_id: Option<i64>,
}

/// A prepared brief, flattened for the card. Claims keep the ids of the sources they rest on.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PrepBriefView {
    pub generated_at_unix_ms: i64,
    pub summary: String,
    pub summary_sources: Vec<String>,
    pub attendees: Vec<PrepAttendee>,
    pub open_items: Vec<PrepClaim>,
    pub open_questions: Vec<PrepClaim>,
    pub sources: Vec<PrepSourceView>,
}

/// The next meeting Today shows, with its brief when one exists.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NextMeetingView {
    pub event_id: String,
    pub title: String,
    pub start_unix_ms: i64,
    pub brief: Option<PrepBriefView>,
}

/// The local meeting a cited source is, or nothing. A source's `meeting_id` is the row id on the
/// device that made the brief, and ids are not shared between devices, so on a brief a peer made it
/// can name an unrelated meeting here. It counts only if that meeting has the source's start time
/// and title.
fn local_meeting(s: &memory::Store, source: &BriefSource) -> Option<i64> {
    if source.kind != PrepSourceKind::Meeting {
        return None;
    }
    let id = source.meeting_id?;
    let meeting = s.get_meeting(id).ok().flatten()?;
    let same = meeting.start_unix_ms == source.ts_unix_ms && meeting.title.trim().to_lowercase() == source.title.trim().to_lowercase();
    same.then_some(id)
}

fn brief_view(s: &memory::Store, row: &memory::prep::MeetingPrepRow) -> Option<PrepBriefView> {
    // A row from a newer build on another device may not read here; it is hidden, not an error.
    let brief: Brief = serde_json::from_str(&row.brief_json).ok()?;
    Some(PrepBriefView {
        generated_at_unix_ms: row.generated_at_unix_ms,
        summary: brief.summary.text,
        summary_sources: brief.summary.sources,
        attendees: brief.attendees.into_iter().map(|a| PrepAttendee { who: a.who, note: a.note, sources: a.sources }).collect(),
        open_items: brief.open_items.into_iter().map(|c| PrepClaim { text: c.text, sources: c.sources }).collect(),
        open_questions: brief.open_questions.into_iter().map(|c| PrepClaim { text: c.text, sources: c.sources }).collect(),
        sources: brief.sources.into_iter().map(|src| PrepSourceView { meeting_id: local_meeting(s, &src), id: src.id, title: src.title }).collect(),
    })
}

/// The brief stored under this PC's calendar event id for `meeting`, aliasing a peer's first so
/// a brief made on another device (under its own event id) shows without waiting for the next tick.
/// Keyed per occurrence (#578): a recurring series shares its event id, so a bare id would show the
/// newest occurrence's brief on every one. A row written before still matches (core's fallback).
fn brief_for(s: &memory::Store, meeting: &QualifyingMeeting) -> Result<Option<PrepBriefView>, String> {
    let key = memory::prep::prep_key(&meeting.id, meeting.start_unix_ms);
    s.alias_synced_meeting_prep(&key, meeting.start_unix_ms, &meeting.title).map_err(|e| e.to_string())?;
    Ok(s.get_meeting_prep(&key).map_err(|e| e.to_string())?.as_ref().and_then(|row| brief_view(s, row)))
}

/// Whether a stored brief reads in this build: a row that doesn't is hidden, so it gets no Prep chip.
fn readable(row: &memory::prep::MeetingPrepRow) -> bool {
    serde_json::from_str::<Brief>(&row.brief_json).is_ok()
}

/// The Prep chip's key for one occurrence of a calendar event: the stored row's own `event_id`,
/// exactly as stored, when a readable brief exists. Aliases a peer's brief first, as `brief_for` does.
pub(crate) fn event_prep_key(s: &memory::Store, event_id: &str, start_unix_ms: i64, title: &str) -> Result<Option<String>, String> {
    let key = memory::prep::prep_key(event_id, start_unix_ms);
    s.alias_synced_meeting_prep(&key, start_unix_ms, title).map_err(|e| e.to_string())?;
    Ok(s.get_meeting_prep(&key).map_err(|e| e.to_string())?.filter(readable).map(|row| row.event_id))
}

/// The Prep chip's key for a recorded meeting: the brief core matches to it, by its stored
/// `event_id` unchanged (never with the start appended again), when that brief is readable.
pub(crate) fn meeting_prep_key(s: &memory::Store, meeting_id: i64) -> Result<Option<String>, String> {
    Ok(s.meeting_prep_for_meeting(meeting_id).map_err(|e| e.to_string())?.filter(readable).map(|row| row.event_id))
}

/// The brief stored under `key` (a Prep chip's key) with its meeting's title and start; nothing when
/// there is none or it doesn't read here.
fn prep_by_key(s: &memory::Store, key: &str) -> Result<Option<NextMeetingView>, String> {
    let Some(row) = s.get_meeting_prep(key).map_err(|e| e.to_string())? else { return Ok(None) };
    let Some(brief) = brief_view(s, &row) else { return Ok(None) };
    Ok(Some(NextMeetingView { event_id: row.event_id, title: row.title, start_unix_ms: row.start_unix_ms, brief: Some(brief) }))
}

/// One recorded meeting's chips that need the store: its Prep chip key and its topic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MeetingChipsView {
    pub meeting_id: i64,
    pub prep_key: Option<String>,
    pub topic: Option<TopicRefView>,
}

fn meeting_chips_for(s: &memory::Store, ids: &[i64]) -> Result<Vec<MeetingChipsView>, String> {
    ids.iter()
        .map(|&id| {
            Ok(MeetingChipsView {
                meeting_id: id,
                prep_key: meeting_prep_key(s, id)?,
                topic: s.topic_for_meeting(id).map_err(|e| e.to_string())?.map(Into::into),
            })
        })
        .collect()
}

/// The first of `upcoming` (qualifying, not over, soonest first) as the card shows it.
fn next_meeting(s: &memory::Store, upcoming: &[QualifyingMeeting]) -> Result<Option<NextMeetingView>, String> {
    let Some(meeting) = upcoming.first() else { return Ok(None) };
    Ok(Some(NextMeetingView {
        event_id: meeting.id.clone(),
        title: meeting.title.clone(),
        start_unix_ms: meeting.start_unix_ms,
        brief: brief_for(s, meeting)?,
    }))
}

/// The next meeting on the calendar and its brief, or nothing when no calendar is connected or
/// nothing is coming up.
#[tauri::command]
pub async fn next_meeting_prep(state: State<'_, AppState>) -> Result<Option<NextMeetingView>, CommandError> {
    let Some(calendar) = state.calendar.clone() else { return Ok(None) };
    let upcoming = calendar.upcoming(chrono::Utc::now().timestamp_millis());
    state.store()?.call(move |s| next_meeting(s, &upcoming)).await?.map_err(CommandError::store)
}

/// The brief a Prep chip names, by its key (`ScheduleEvent::prep_key`, `MeetingChipsView::prep_key`).
#[tauri::command]
pub async fn meeting_prep_brief(state: State<'_, AppState>, key: String) -> Result<Option<NextMeetingView>, CommandError> {
    state.store()?.call(move |s| prep_by_key(s, &key)).await?.map_err(CommandError::store)
}

/// The Prep chip key and topic of each of `ids`, in one store call: the Meetings list asks once for
/// every row, and again only when the set of meetings changes.
#[tauri::command]
pub async fn meeting_chips(state: State<'_, AppState>, ids: Vec<i64>) -> Result<Vec<MeetingChipsView>, CommandError> {
    state.store()?.call(move |s| meeting_chips_for(s, &ids)).await?.map_err(CommandError::store)
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::connectors::calendar::MeetingSource;
    use llm::meeting_prep::{AttendeeNote, BriefSource, Claim};
    use memory::prep::MeetingPrepRow;

    fn meeting(id: &str, start: i64) -> QualifyingMeeting {
        QualifyingMeeting {
            id: id.into(),
            title: "Acme sync".into(),
            start_unix_ms: start,
            end_unix_ms: start + 1_800_000,
            attendees: vec!["ann@acme.test".into()],
            conference_link: None,
            source: MeetingSource::GoogleCalendar,
            app_bundle_id: None,
            ..Default::default()
        }
    }

    fn row(event_id: &str, start: i64, brief_json: String) -> MeetingPrepRow {
        MeetingPrepRow {
            event_id: event_id.into(),
            title: "Acme sync".into(),
            start_unix_ms: start,
            generated_at_unix_ms: start - 600_000,
            brief_json,
            provider: "ollama".into(),
            model: "m".into(),
        }
    }

    fn a_brief() -> String {
        serde_json::to_string(&Brief {
            summary: Claim { text: "Pricing is open.".into(), sources: vec!["M1".into()] },
            attendees: vec![AttendeeNote { who: "Ann".into(), note: "Owns the budget.".into(), sources: vec![], last_interaction_unix_ms: None }],
            open_items: vec![Claim { text: "Send the quote".into(), sources: vec!["M1".into()] }],
            open_questions: vec![Claim { text: "Who signs?".into(), sources: vec![] }],
            sources: vec![BriefSource {
                id: "M1".into(),
                kind: memory::prep::PrepSourceKind::Meeting,
                title: "Acme kickoff".into(),
                ts_unix_ms: 1,
                meeting_id: Some(1),
                thread_id: None,
            }],
        })
        .unwrap()
    }

    #[test]
    fn a_brief_made_here_shows_with_its_claims_and_sources() {
        let store = memory::Store::open_in_memory().unwrap();
        store.save_meeting_prep(&row("local", 10_000_000, a_brief())).unwrap();
        let shown = next_meeting(&store, &[meeting("local", 10_000_000)]).unwrap().expect("a next meeting");
        let brief = shown.brief.expect("the brief");
        assert_eq!((shown.event_id.as_str(), brief.summary.as_str(), brief.open_items.as_slice()), ("local", "Pricing is open.", [PrepClaim { text: "Send the quote".into(), sources: vec!["M1".into()] }].as_slice()));
        assert_eq!(brief.sources, [PrepSourceView { id: "M1".into(), title: "Acme kickoff".into(), meeting_id: None }]);
        assert_eq!(brief.attendees, [PrepAttendee { who: "Ann".into(), note: "Owns the budget.".into(), sources: vec![] }]);
    }

    #[test]
    fn a_brief_another_device_made_shows_under_this_pcs_event_id() {
        let store = memory::Store::open_in_memory().unwrap();
        // The peer's calendar id differs; start and title match.
        assert!(store.apply_synced_meeting_prep(&row("peer-id", 10_000_000, a_brief())).unwrap());
        let shown = next_meeting(&store, &[meeting("my-id", 10_000_000)]).unwrap().unwrap();
        assert_eq!(shown.brief.expect("aliased brief").summary, "Pricing is open.");
        assert!(store.get_meeting_prep("my-id").unwrap().is_some(), "now stored under the local id");
    }

    #[test]
    fn each_occurrence_of_a_recurring_meeting_shows_its_own_brief() {
        let store = memory::Store::open_in_memory().unwrap();
        let brief = |summary: &str| a_brief().replace("Pricing is open.", summary);
        store.save_meeting_prep(&row(&memory::prep::prep_key("weekly", 10_000_000), 10_000_000, brief("Monday's"))).unwrap();
        store.save_meeting_prep(&row(&memory::prep::prep_key("weekly", 20_000_000), 20_000_000, brief("Next Monday's"))).unwrap();
        let shown = next_meeting(&store, &[meeting("weekly", 10_000_000)]).unwrap().unwrap();
        assert_eq!(shown.brief.expect("this occurrence's brief").summary, "Monday's");
    }

    #[test]
    fn a_meeting_with_no_brief_yet_shows_without_one_and_no_meeting_shows_nothing() {
        let store = memory::Store::open_in_memory().unwrap();
        let shown = next_meeting(&store, &[meeting("e", 10_000_000)]).unwrap().unwrap();
        assert_eq!(shown.brief, None);
        assert_eq!(next_meeting(&store, &[]).unwrap(), None);
    }

    #[test]
    fn it_is_the_first_meeting_in_the_list_not_a_later_one_with_a_brief() {
        let store = memory::Store::open_in_memory().unwrap();
        store.save_meeting_prep(&row("later", 20_000_000, a_brief())).unwrap();
        let shown = next_meeting(&store, &[meeting("soon", 10_000_000), meeting("later", 20_000_000)]).unwrap().unwrap();
        assert_eq!((shown.event_id.as_str(), shown.brief), ("soon", None));
    }

    #[test]
    fn a_brief_in_a_shape_this_build_cannot_read_is_hidden_not_an_error() {
        let store = memory::Store::open_in_memory().unwrap();
        store.save_meeting_prep(&row("e", 10_000_000, r#"{"from":"a newer build"}"#.into())).unwrap();
        let shown = next_meeting(&store, &[meeting("e", 10_000_000)]).unwrap().unwrap();
        assert_eq!(shown.brief, None);
    }

    // --- chips: a source opens a meeting only if it is provably that meeting ---

    fn put_meeting(store: &memory::Store, title: &str, start: i64) -> i64 {
        store
            .insert_meeting(&memory::NewMeeting {
                device_id: "dev".into(),
                start_unix_ms: start,
                end_unix_ms: Some(start + 1_000),
                title: title.into(),
                attendees: vec![],
                trigger: memory::MeetingTrigger::Manual,
                audio_path: None,
                status: memory::MeetingStatus::Completed,
            })
            .unwrap()
    }

    fn source(kind: PrepSourceKind, title: &str, ts: i64, meeting_id: Option<i64>, thread_id: Option<i64>) -> BriefSource {
        BriefSource { id: "M1".into(), kind, title: title.into(), ts_unix_ms: ts, meeting_id, thread_id }
    }

    #[test]
    fn a_source_that_is_a_local_meeting_with_the_same_title_and_start_opens_it() {
        let store = memory::Store::open_in_memory().unwrap();
        let id = put_meeting(&store, "Acme kickoff", 1_000);
        let src = source(PrepSourceKind::Meeting, "  acme KICKOFF ", 1_000, Some(id), None);
        assert_eq!(local_meeting(&store, &src), Some(id), "case and padding do not matter");
    }

    #[test]
    fn an_id_that_names_a_different_meeting_here_opens_nothing() {
        // The peer-made brief: its meeting_id is the peer's row id, which here is some other meeting.
        let store = memory::Store::open_in_memory().unwrap();
        let other = put_meeting(&store, "Lunch", 5_000);
        let wrong_title = source(PrepSourceKind::Meeting, "Acme kickoff", 5_000, Some(other), None);
        let wrong_start = source(PrepSourceKind::Meeting, "Lunch", 6_000, Some(other), None);
        assert_eq!(local_meeting(&store, &wrong_title), None);
        assert_eq!(local_meeting(&store, &wrong_start), None);
    }

    #[test]
    fn a_missing_meeting_and_an_email_source_open_nothing() {
        let store = memory::Store::open_in_memory().unwrap();
        assert_eq!(local_meeting(&store, &source(PrepSourceKind::Meeting, "Gone", 1, Some(999), None)), None);
        assert_eq!(local_meeting(&store, &source(PrepSourceKind::Meeting, "No id", 1, None, None)), None);
        let id = put_meeting(&store, "Re: quote", 1);
        assert_eq!(local_meeting(&store, &source(PrepSourceKind::Email, "Re: quote", 1, Some(id), Some(9))), None, "no thread view yet");
    }

    // --- Prep chip keys ---

    #[test]
    fn an_event_prep_key_is_the_stored_rows_event_id() {
        let store = memory::Store::open_in_memory().unwrap();
        let key = memory::prep::prep_key("weekly", 10_000_000);
        store.save_meeting_prep(&row(&key, 10_000_000, a_brief())).unwrap();
        assert_eq!(event_prep_key(&store, "weekly", 10_000_000, "Acme sync").unwrap(), Some(key));
        // A row written before per-occurrence keys holds the bare id, and the chip keeps it as is.
        store.save_meeting_prep(&row("old", 20_000_000, a_brief())).unwrap();
        assert_eq!(event_prep_key(&store, "old", 20_000_000, "Acme sync").unwrap().as_deref(), Some("old"));
        assert_eq!(event_prep_key(&store, "none", 30_000_000, "Acme sync").unwrap(), None);
    }

    #[test]
    fn an_unreadable_brief_gets_no_prep_key() {
        let store = memory::Store::open_in_memory().unwrap();
        store.save_meeting_prep(&row("e@10000000", 10_000_000, r#"{"from":"a newer build"}"#.into())).unwrap();
        assert_eq!(event_prep_key(&store, "e", 10_000_000, "Acme sync").unwrap(), None);
    }

    #[test]
    fn a_recorded_meetings_prep_key_is_its_briefs_stored_id_unchanged() {
        let store = memory::Store::open_in_memory().unwrap();
        let id = put_meeting(&store, "Acme sync", 10_000_000);
        let other = put_meeting(&store, "Lunch", 10_000_000);
        store.save_meeting_prep(&row("evt@10000000", 10_000_000, a_brief())).unwrap();
        assert_eq!(meeting_prep_key(&store, id).unwrap().as_deref(), Some("evt@10000000"));
        assert_eq!(meeting_prep_key(&store, other).unwrap(), None);
        let chips = meeting_chips_for(&store, &[id, other]).unwrap();
        assert_eq!(chips.iter().map(|c| (c.meeting_id, c.prep_key.clone())).collect::<Vec<_>>(), [(id, Some("evt@10000000".into())), (other, None)]);
    }

    #[test]
    fn a_brief_opens_by_its_key_and_a_missing_one_is_nothing() {
        let store = memory::Store::open_in_memory().unwrap();
        store.save_meeting_prep(&row("evt@10000000", 10_000_000, a_brief())).unwrap();
        let shown = prep_by_key(&store, "evt@10000000").unwrap().expect("the brief");
        assert_eq!((shown.event_id.as_str(), shown.title.as_str(), shown.start_unix_ms), ("evt@10000000", "Acme sync", 10_000_000));
        assert_eq!(shown.brief.unwrap().summary, "Pricing is open.");
        assert_eq!(prep_by_key(&store, "missing").unwrap(), None);
        store.save_meeting_prep(&row("bad", 1, r#"{"x":1}"#.into())).unwrap();
        assert_eq!(prep_by_key(&store, "bad").unwrap(), None);
    }

    #[test]
    fn the_brief_view_carries_the_openable_meeting_id() {
        let store = memory::Store::open_in_memory().unwrap();
        let id = put_meeting(&store, "Acme kickoff", 1);
        assert_eq!(id, 1);
        store.save_meeting_prep(&row("local", 10_000_000, a_brief())).unwrap();
        let shown = next_meeting(&store, &[meeting("local", 10_000_000)]).unwrap().unwrap().brief.unwrap();
        assert_eq!(shown.sources[0].meeting_id, Some(1));
    }
}
