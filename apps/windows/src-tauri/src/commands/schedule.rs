//! Today's schedule card (docs/superpowers/plans/2026-10-20-today-refresh-windows.md, Task 4): the
//! calendar events still ahead today, which of them already have a recording, and the meetings held
//! earlier today. Read-only, no model; `meeting_record_start_event` starts a recording for one event the
//! way the alert's Accept does.

use api::connectors::calendar::QualifyingMeeting;
use schemars::JsonSchema;
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::commands::library::{meeting_row, MeetingRow};
use crate::commands::today::day_bounds;
use crate::device_labels::DeviceLabeler;
use crate::error::CommandError;
use crate::meetings::controller::{MeetingRecordingView, RECONCILE_WINDOW};
use crate::state::AppState;

/// A meeting recorded up to this long before its event's start still counts as that event's recording:
/// Start can be pressed early, and the row then starts at the press, not at the event.
const EARLY_START_MS: i64 = 30 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleEvent {
    pub event_id: String,
    pub title: String,
    pub start_unix_ms: i64,
    pub end_unix_ms: i64,
    pub attendees: Vec<String>,
    /// The local meeting recorded for this event, when there is one (recording now or done).
    pub recorded_meeting_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleView {
    /// Today's events that are not over yet, soonest first.
    pub events: Vec<ScheduleEvent>,
    /// Meetings that started today and are not one of the events above, newest first.
    pub earlier_today: Vec<MeetingRow>,
    /// A meeting is recording now, so no row offers Start.
    pub recording: bool,
}

fn same_title(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

/// The newest meeting recorded for `event`: the same title (trimmed, case-insensitive) that started no
/// earlier than `EARLY_START_MS` before the event and before it ended. A different title at the same
/// time is a different meeting.
fn recorded_meeting_id(event: &QualifyingMeeting, meetings: &[memory::Meeting]) -> Option<i64> {
    meetings
        .iter()
        .filter(|m| {
            same_title(&m.title, &event.title) && m.start_unix_ms >= event.start_unix_ms - EARLY_START_MS && m.start_unix_ms < event.end_unix_ms
        })
        .max_by_key(|m| m.start_unix_ms)
        .map(|m| m.id)
}

/// `calendar` is every event that could still prompt (not over); `day_start`/`day_end` bound the local
/// day. An event that began yesterday and is still on counts as today's.
pub fn build_schedule(
    calendar: &[QualifyingMeeting],
    meetings: &[memory::Meeting],
    now_unix_ms: i64,
    day_start: i64,
    day_end: i64,
    recording: bool,
    labeler: &DeviceLabeler,
) -> ScheduleView {
    let mut today: Vec<&QualifyingMeeting> = calendar.iter().filter(|e| e.end_unix_ms > now_unix_ms && e.start_unix_ms < day_end).collect();
    today.sort_by_key(|e| e.start_unix_ms);
    let events: Vec<ScheduleEvent> = today
        .into_iter()
        .map(|e| ScheduleEvent {
            event_id: e.id.clone(),
            title: e.title.clone(),
            start_unix_ms: e.start_unix_ms,
            end_unix_ms: e.end_unix_ms,
            attendees: e.attendees.clone(),
            recorded_meeting_id: recorded_meeting_id(e, meetings),
        })
        .collect();
    let mut earlier: Vec<&memory::Meeting> = meetings
        .iter()
        .filter(|m| m.start_unix_ms >= day_start && m.start_unix_ms < day_end && !events.iter().any(|e| e.recorded_meeting_id == Some(m.id)))
        .collect();
    earlier.sort_by_key(|m| std::cmp::Reverse(m.start_unix_ms));
    ScheduleView { events, earlier_today: earlier.into_iter().map(|m| meeting_row(m, labeler)).collect(), recording }
}

#[tauri::command]
pub async fn today_schedule(state: State<'_, AppState>) -> Result<ScheduleView, CommandError> {
    let labeler = state.device_labeler();
    let now = chrono::Utc::now().timestamp_millis();
    let (_, day_start, day_end) = day_bounds(&chrono::Local::now());
    let calendar = state.calendar.as_ref().map(|c| c.upcoming(now)).unwrap_or_default();
    let recording = state.meetings.as_ref().is_some_and(|m| m.status().meeting_id.is_some());
    let meetings = state
        .store()?
        .call(move |s| s.recent_meetings(RECONCILE_WINDOW).map_err(|e| e.to_string()))
        .await?
        .map_err(CommandError::store)?;
    Ok(build_schedule(&calendar, &meetings, now, day_start, day_end, recording, &labeler))
}

/// Start recording one of today's calendar events: its title, invitees and start, and the meeting app
/// when detection has found one (what the alert's Accept does).
#[tauri::command]
pub async fn meeting_record_start_event(app: AppHandle, event_id: String) -> Result<MeetingRecordingView, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let now = chrono::Utc::now().timestamp_millis();
        let event = state
            .calendar
            .as_ref()
            .and_then(|c| c.upcoming(now).into_iter().find(|e| e.id == event_id))
            .ok_or_else(|| CommandError::invalid_input("That meeting is no longer on the calendar."))?;
        let started = state.meetings()?.start_calendar(event.title, event.attendees, Some(event.start_unix_ms), state.detected.candidate_pid());
        crate::tray::refresh(&app);
        started
    })
    .await
    .map_err(|e| CommandError::store(format!("the meeting command failed: {e}")))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::connectors::calendar::MeetingSource;

    const MIN: i64 = 60_000;
    const DAY_START: i64 = 1_000_000_000;
    const DAY_END: i64 = DAY_START + 24 * 60 * MIN;

    fn event(id: &str, title: &str, start: i64, minutes: i64) -> QualifyingMeeting {
        QualifyingMeeting {
            id: id.into(),
            title: title.into(),
            start_unix_ms: start,
            end_unix_ms: start + minutes * MIN,
            attendees: vec!["ann@acme.test".into()],
            conference_link: None,
            source: MeetingSource::GoogleCalendar,
            app_bundle_id: None,
            ..Default::default()
        }
    }

    /// Stores a meeting and reads it back, so the fixture has every field a real one does.
    fn put(store: &memory::Store, title: &str, start: i64) -> memory::Meeting {
        let id = store
            .insert_meeting(&memory::NewMeeting {
                device_id: "dev".into(),
                start_unix_ms: start,
                end_unix_ms: Some(start + 5 * MIN),
                title: title.into(),
                attendees: vec![],
                trigger: memory::MeetingTrigger::Manual,
                audio_path: None,
                status: memory::MeetingStatus::Completed,
            })
            .unwrap();
        store.get_meeting(id).unwrap().expect("the meeting just stored")
    }

    fn view(calendar: &[QualifyingMeeting], meetings: &[memory::Meeting], now: i64) -> ScheduleView {
        build_schedule(calendar, meetings, now, DAY_START, DAY_END, false, &DeviceLabeler::default())
    }

    fn titles(v: &ScheduleView) -> Vec<&str> {
        v.events.iter().map(|e| e.title.as_str()).collect()
    }

    #[test]
    fn events_are_todays_that_are_not_over_soonest_first() {
        let now = DAY_START + 600 * MIN;
        let calendar = vec![
            event("a", "Late", now + 300 * MIN, 30),
            event("b", "Standup", now + 10 * MIN, 15),
            event("c", "Planning", now - 30 * MIN, 120), // started, still on
            event("d", "Over", now - 90 * MIN, 30),
            event("e", "Tomorrow", DAY_END + 60 * MIN, 30),
        ];
        assert_eq!(titles(&view(&calendar, &[], now)), ["Planning", "Standup", "Late"]);
    }

    #[test]
    fn an_event_that_began_yesterday_and_is_still_on_counts_as_today() {
        let now = DAY_START + 30 * MIN;
        let calendar = vec![event("a", "Overnight", DAY_START - 60 * MIN, 240)];
        assert_eq!(titles(&view(&calendar, &[], now)), ["Overnight"]);
    }

    #[test]
    fn an_event_carries_its_attendees_and_id() {
        let now = DAY_START;
        let v = view(&[event("evt-1", "Sync", now + MIN, 30)], &[], now);
        assert_eq!((v.events[0].event_id.as_str(), v.events[0].attendees.clone()), ("evt-1", vec!["ann@acme.test".to_string()]));
    }

    #[test]
    fn a_recording_with_the_same_title_started_at_the_event_is_its_recording() {
        let store = memory::Store::open_in_memory().unwrap();
        let now = DAY_START + 600 * MIN;
        let ev = event("a", "Acme sync", now - 5 * MIN, 30);
        let m = put(&store, "  acme SYNC ", ev.start_unix_ms);
        let v = view(&[ev], &[m.clone()], now);
        assert_eq!(v.events[0].recorded_meeting_id, Some(m.id));
        assert!(v.earlier_today.is_empty(), "a linked meeting is not listed twice");
    }

    #[test]
    fn started_early_still_counts_but_not_more_than_half_an_hour_early() {
        let store = memory::Store::open_in_memory().unwrap();
        let now = DAY_START + 600 * MIN;
        let ev = event("a", "Acme sync", now + 20 * MIN, 30);
        let early = put(&store, "Acme sync", ev.start_unix_ms - 10 * MIN);
        assert_eq!(view(&[ev.clone()], &[early.clone()], now).events[0].recorded_meeting_id, Some(early.id));
        let too_early = put(&store, "Acme sync", ev.start_unix_ms - 45 * MIN);
        let v = view(&[ev], &[too_early.clone()], now);
        assert_eq!(v.events[0].recorded_meeting_id, None);
        assert_eq!(v.earlier_today.iter().map(|r| r.id).collect::<Vec<_>>(), [too_early.id]);
    }

    #[test]
    fn a_different_title_at_the_same_time_is_a_different_meeting() {
        let store = memory::Store::open_in_memory().unwrap();
        let now = DAY_START + 600 * MIN;
        let ev = event("a", "Acme sync", now, 30);
        let other = put(&store, "Dentist", now);
        let v = view(&[ev], &[other.clone()], now);
        assert_eq!(v.events[0].recorded_meeting_id, None);
        assert_eq!(v.earlier_today.len(), 1);
    }

    #[test]
    fn two_recordings_of_one_event_link_the_newest() {
        let store = memory::Store::open_in_memory().unwrap();
        let now = DAY_START + 600 * MIN;
        let ev = event("a", "Acme sync", now - 10 * MIN, 60);
        let first = put(&store, "Acme sync", now - 10 * MIN);
        let second = put(&store, "Acme sync", now - 2 * MIN);
        assert_eq!(view(&[ev], &[first, second.clone()], now).events[0].recorded_meeting_id, Some(second.id));
    }

    #[test]
    fn earlier_today_is_todays_other_meetings_newest_first_and_nothing_older() {
        let store = memory::Store::open_in_memory().unwrap();
        let now = DAY_START + 600 * MIN;
        let morning = put(&store, "Morning", DAY_START + 60 * MIN);
        let noon = put(&store, "Noon", DAY_START + 300 * MIN);
        let yesterday = put(&store, "Yesterday", DAY_START - MIN);
        let v = view(&[], &[morning.clone(), yesterday, noon.clone()], now);
        assert_eq!(v.earlier_today.iter().map(|r| r.id).collect::<Vec<_>>(), [noon.id, morning.id]);
    }

    #[test]
    fn a_finished_event_drops_off_but_its_recording_is_listed_under_earlier_today() {
        let store = memory::Store::open_in_memory().unwrap();
        let now = DAY_START + 600 * MIN;
        let over = event("a", "Acme sync", now - 90 * MIN, 30);
        let m = put(&store, "Acme sync", over.start_unix_ms);
        let v = view(&[over], &[m.clone()], now);
        assert!(v.events.is_empty());
        assert_eq!(v.earlier_today.iter().map(|r| r.id).collect::<Vec<_>>(), [m.id]);
    }

    #[test]
    fn the_recording_flag_is_passed_through() {
        let v = build_schedule(&[], &[], DAY_START, DAY_START, DAY_END, true, &DeviceLabeler::default());
        assert!(v.recording);
    }
}
