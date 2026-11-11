//! Today's schedule card (docs/superpowers/plans/2026-10-20-today-refresh-windows.md, Task 4): the
//! calendar events still ahead today, which of them already have a recording, and the meetings held
//! earlier today. Read-only, no model; `meeting_record_start_event` starts a recording for one event the
//! way the alert's Accept does.

use api::connectors::calendar::QualifyingMeeting;
use schemars::JsonSchema;
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::commands::library::{has_summary, meeting_row, MeetingRow};
use crate::commands::prep::event_prep_key;
use crate::commands::today::day_bounds;
use crate::device_labels::DeviceLabeler;
use crate::error::CommandError;
use crate::meetings::controller::{MeetingRecordingView, RECONCILE_WINDOW};
use crate::state::AppState;
use crate::l10n::tr;

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
    /// The Prep chip's key (the stored brief's own id) when a readable brief exists; Today only.
    pub prep_key: Option<String>,
    /// The recorded meeting has a non-empty summary; false when there is no recording.
    pub has_summary: bool,
    /// The recorded meeting's summary action items; 0 when there is no recording.
    pub action_item_count: u32,
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
        .map(|e| {
            let recorded_id = recorded_meeting_id(e, meetings);
            let recorded = recorded_id.and_then(|id| meetings.iter().find(|m| m.id == id));
            ScheduleEvent {
                event_id: e.id.clone(),
                title: e.title.clone(),
                start_unix_ms: e.start_unix_ms,
                end_unix_ms: e.end_unix_ms,
                attendees: e.attendees.clone(),
                recorded_meeting_id: recorded_id,
                prep_key: None,
                has_summary: recorded.is_some_and(has_summary),
                action_item_count: recorded.map_or(0, |m| m.summary_action_items.len() as u32),
            }
        })
        .collect();
    let mut earlier: Vec<&memory::Meeting> = meetings
        .iter()
        .filter(|m| m.start_unix_ms >= day_start && m.start_unix_ms < day_end && !events.iter().any(|e| e.recorded_meeting_id == Some(m.id)))
        .collect();
    earlier.sort_by_key(|m| std::cmp::Reverse(m.start_unix_ms));
    ScheduleView { events, earlier_today: earlier.into_iter().map(|m| meeting_row(m, labeler)).collect(), recording }
}

/// Fills in each event's Prep chip key, which `build_schedule` leaves empty: its brief by occurrence.
/// (The earlier-today meetings get theirs from `meeting_chips`, as the Meetings list does.)
pub(crate) fn add_prep_keys(s: &memory::Store, view: &mut ScheduleView) -> Result<(), String> {
    for e in &mut view.events {
        e.prep_key = event_prep_key(s, &e.event_id, e.start_unix_ms, &e.title)?;
    }
    Ok(())
}

/// Today's Upcoming section (docs/superpowers/specs/2026-11-08-today-upcoming-design.md): the days after
/// today it covers.
pub const UPCOMING_DAYS: u32 = 7;

/// One day in Upcoming: its local date and the meetings that start on it, soonest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpcomingDay {
    /// `YYYY-MM-DD`, local.
    pub date: String,
    /// The first day after today: the UI calls it "Tomorrow".
    pub tomorrow: bool,
    pub events: Vec<ScheduleEvent>,
}

/// The local days after `now`'s, as (date, start, end), for `count` days. Each day is found from its own
/// noon, so a daylight-saving change can't shift it.
pub fn days_after<Tz: chrono::TimeZone>(now: &chrono::DateTime<Tz>, count: u32) -> Vec<(String, i64, i64)> {
    let tz = now.timezone();
    let today = now.date_naive();
    (1..=count)
        .filter_map(|n| {
            let date = today.checked_add_days(chrono::Days::new(n as u64))?;
            let noon = tz.from_local_datetime(&date.and_hms_opt(12, 0, 0)?).earliest()?;
            Some(day_bounds(&noon))
        })
        .collect()
}

/// `calendar`'s meetings grouped into `days`, by when they start. Days with none are left out.
pub fn build_upcoming(calendar: &[QualifyingMeeting], days: &[(String, i64, i64)]) -> Vec<UpcomingDay> {
    days.iter()
        .enumerate()
        .filter_map(|(i, (date, start, end))| {
            let mut on_day: Vec<&QualifyingMeeting> = calendar.iter().filter(|e| e.start_unix_ms >= *start && e.start_unix_ms < *end).collect();
            if on_day.is_empty() {
                return None;
            }
            on_day.sort_by_key(|e| e.start_unix_ms);
            let events = on_day
                .into_iter()
                .map(|e| ScheduleEvent {
                    event_id: e.id.clone(),
                    title: e.title.clone(),
                    start_unix_ms: e.start_unix_ms,
                    end_unix_ms: e.end_unix_ms,
                    attendees: e.attendees.clone(),
                    recorded_meeting_id: None,
                    prep_key: None,
                    has_summary: false,
                    action_item_count: 0,
                })
                .collect();
            Some(UpcomingDay { date: date.clone(), tomorrow: i == 0, events })
        })
        .collect()
}

/// Today's Upcoming, and why it may be empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpcomingView {
    pub days: Vec<UpcomingDay>,
    /// The calendar's last sync failed (a Google sign-in that expired, say): the week may look empty
    /// only because nothing could be read.
    pub sync_error: Option<String>,
}

/// Today's Upcoming: the qualifying meetings of the next `UPCOMING_DAYS` days, from the calendar's last
/// sync. Nothing is stored.
#[tauri::command]
pub fn today_upcoming(state: State<'_, AppState>) -> UpcomingView {
    let now = chrono::Local::now();
    let calendar = state.calendar.as_ref().map(|c| c.upcoming(now.timestamp_millis())).unwrap_or_default();
    let sync_error = state.calendar.as_ref().and_then(|c| c.status().error);
    UpcomingView { days: build_upcoming(&calendar, &days_after(&now, UPCOMING_DAYS)), sync_error }
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
    let mut view = build_schedule(&calendar, &meetings, now, day_start, day_end, recording, &labeler);
    state
        .store()?
        .call(move |s| add_prep_keys(s, &mut view).map(|()| view))
        .await?
        .map_err(CommandError::store)
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
            .ok_or_else(|| CommandError::invalid_input(tr("That meeting is no longer on the calendar.", &[])))?;
        let started = state.meetings()?.start_calendar(event.title, event.attendees, event.attendee_details, Some(event.start_unix_ms), state.detected.candidate_pid());
        crate::tray::refresh(&app);
        started
    })
    .await
    .map_err(|e| CommandError::store(tr("The meeting command failed: {error}", &[("error", &e.to_string())])))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::connectors::calendar::MeetingSource;

    const MIN: i64 = 60_000;
    const DAY_START: i64 = 1_000_000_000;
    const DAY_END: i64 = DAY_START + 24 * 60 * MIN;

    #[test]
    fn upcoming_groups_by_start_day_and_leaves_out_empty_days_and_today() {
        let day = 24 * 60 * MIN;
        let days: Vec<(String, i64, i64)> =
            (1..=3).map(|n| (format!("d{n}"), DAY_START + n * day, DAY_START + (n + 1) * day)).collect();
        let calendar = vec![
            event("today", "Still today", DAY_START + 23 * 60 * MIN, 120),
            event("b", "Late", DAY_START + day + 15 * 60 * MIN, 30),
            event("a", "Early", DAY_START + day + 9 * 60 * MIN, 30),
            event("c", "Third day", DAY_START + 3 * day + 60 * MIN, 30),
            event("far", "Next month", DAY_START + 30 * day, 30),
        ];
        let up = build_upcoming(&calendar, &days);
        let shape: Vec<(&str, bool, Vec<&str>)> =
            up.iter().map(|d| (d.date.as_str(), d.tomorrow, d.events.iter().map(|e| e.event_id.as_str()).collect())).collect();
        assert_eq!(shape, vec![("d1", true, vec!["a", "b"]), ("d3", false, vec!["c"])]);
    }

    #[test]
    fn the_days_after_today_are_seven_back_to_back_local_days() {
        use chrono::TimeZone;
        // Evening, so "today" is still Oct 31 in this offset though UTC has moved on.
        let tz = chrono::FixedOffset::west_opt(4 * 3600).unwrap();
        let now = tz.with_ymd_and_hms(2026, 10, 31, 20, 0, 0).unwrap();
        let days = days_after(&now, 7);
        assert_eq!(days.len(), 7);
        assert_eq!(days[0].0, "2026-11-01");
        assert_eq!(days[6].0, "2026-11-07");
        assert!(days.windows(2).all(|w| w[0].2 == w[1].1), "each day ends where the next starts");
    }

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
    fn a_recorded_event_carries_its_summary_and_action_items() {
        let store = memory::Store::open_in_memory().unwrap();
        let now = DAY_START + 600 * MIN;
        let ev = event("a", "Acme sync", now - 5 * MIN, 30);
        let mut m = put(&store, "Acme sync", ev.start_unix_ms);
        m.summary_tldr = Some("We shipped.".into());
        m.summary_action_items = vec!["Send the quote".into(), "Book a room".into()];
        let v = view(&[ev.clone(), event("b", "Later", now + 60 * MIN, 30)], &[m.clone()], now);
        assert_eq!((v.events[0].has_summary, v.events[0].action_item_count), (true, 2));
        assert_eq!((v.events[1].has_summary, v.events[1].action_item_count), (false, 0), "not recorded");
        m.summary_tldr = Some(String::new());
        assert!(!view(&[ev], &[m], now).events[0].has_summary, "an empty summary is no summary");
    }

    #[test]
    fn prep_keys_are_filled_from_the_store() {
        let store = memory::Store::open_in_memory().unwrap();
        let now = DAY_START + 600 * MIN;
        let brief = r#"{"summary":{"text":"x","sources":[]},"attendees":[],"open_items":[],"open_questions":[],"sources":[]}"#;
        let save = |id: &str, start: i64, title: &str| {
            store
                .save_meeting_prep(&memory::prep::MeetingPrepRow {
                    event_id: id.into(),
                    title: title.into(),
                    start_unix_ms: start,
                    generated_at_unix_ms: 0,
                    brief_json: brief.into(),
                    provider: "p".into(),
                    model: "m".into(),
                })
                .unwrap()
        };
        let ev = event("evt", "Acme sync", now + 30 * MIN, 30);
        save(&memory::prep::prep_key("evt", ev.start_unix_ms), ev.start_unix_ms, "Acme sync");
        let mut v = view(&[ev.clone(), event("bare", "No brief", now + 90 * MIN, 30)], &[], now);
        add_prep_keys(&store, &mut v).unwrap();
        assert_eq!(v.events[0].prep_key, Some(memory::prep::prep_key("evt", ev.start_unix_ms)));
        assert_eq!(v.events[1].prep_key, None);
    }

    #[test]
    fn the_recording_flag_is_passed_through() {
        let v = build_schedule(&[], &[], DAY_START, DAY_START, DAY_END, true, &DeviceLabeler::default());
        assert!(v.recording);
    }
}
