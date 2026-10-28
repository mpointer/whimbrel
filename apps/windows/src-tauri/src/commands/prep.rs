//! Meeting Prep on Today (docs/superpowers/plans/2026-10-15-meeting-prep-windows.md): the brief for
//! the next calendar meeting, whether this PC made it or another device's synced in. The run itself
//! is `routines::prep_tick`; this only reads.

use api::connectors::calendar::QualifyingMeeting;
use llm::meeting_prep::{Brief, BriefSource};
use memory::prep::PrepSourceKind;
use schemars::JsonSchema;
use serde::Serialize;
use tauri::State;

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
fn brief_for(s: &memory::Store, meeting: &QualifyingMeeting) -> Result<Option<PrepBriefView>, String> {
    s.alias_synced_meeting_prep(&meeting.id, meeting.start_unix_ms, &meeting.title).map_err(|e| e.to_string())?;
    Ok(s.get_meeting_prep(&meeting.id).map_err(|e| e.to_string())?.as_ref().and_then(|row| brief_view(s, row)))
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
