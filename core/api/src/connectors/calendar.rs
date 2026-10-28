//! Google Calendar: decoding the connector's `sync_events` result (Google's raw
//! Events-resource JSON, passed through unmodified by mcp-servers/google-calendar),
//! folding it into an account's known meetings, merging with EventKit, mapping to
//! `items`, and the conference-link detector both sources share.

use std::sync::OnceLock;

use memory::{NewCalendarEventItem, Store};

use super::ToolCall;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MeetingProvider {
    Zoom,
    GoogleMeet,
    Teams,
    Webex,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ConferenceLink {
    pub provider: MeetingProvider,
    pub url: String,
}

/// Where a qualifying meeting came from: EventKit, the Google OAuth connector, or a
/// heuristic detection (a known meeting app with sustained audio).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MeetingSource {
    #[default]
    Calendar,
    GoogleCalendar,
    Detected,
}

/// A meeting that qualifies for a recording prompt (FR-MTG-1/2). JSON field names are
/// what the shells' mirrors use.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct QualifyingMeeting {
    pub id: String,
    pub title: String,
    pub start_unix_ms: i64,
    pub end_unix_ms: i64,
    pub attendees: Vec<String>,
    pub conference_link: Option<ConferenceLink>,
    pub source: MeetingSource,
    pub app_bundle_id: Option<String>,
    /// The organizer's address and display name, when the source knows them.
    #[serde(default)]
    pub organizer_email: Option<String>,
    #[serde(default)]
    pub organizer_name: Option<String>,
    /// True when the user organized the meeting.
    #[serde(default)]
    pub self_is_organizer: bool,
    /// The user's own RSVP, when they appear in the attendee list.
    #[serde(default)]
    pub self_response: Option<SelfResponse>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelfResponse {
    Accepted,
    Tentative,
    NeedsAction,
    Declined,
}

/// Ordered so a more specific pattern (teams.live.com) isn't shadowed by a broader
/// one; each regex requires a scheme and stops at the first whitespace or angle bracket.
fn link_patterns() -> &'static [(MeetingProvider, regex::Regex)] {
    static PATTERNS: OnceLock<Vec<(MeetingProvider, regex::Regex)>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        [
            (MeetingProvider::Zoom, r"https?://[a-zA-Z0-9.-]*zoom\.us/j/[^\s<>]+"),
            (MeetingProvider::GoogleMeet, r"https?://meet\.google\.com/[^\s<>]+"),
            (MeetingProvider::Teams, r"https?://teams\.(?:microsoft|live)\.com/[^\s<>]+"),
            (MeetingProvider::Webex, r"https?://[a-zA-Z0-9.-]*webex\.com/[^\s<>]+"),
        ]
        .into_iter()
        .map(|(p, re)| (p, regex::Regex::new(re).expect("static pattern")))
        .collect()
    })
}

/// The first recognized conferencing link in `text`. `[^\s<>]+` happily swallows a
/// trailing `)`, `.` or `,` from surrounding prose, so those are trimmed off the match.
pub fn detect_conference_link(text: &str) -> Option<ConferenceLink> {
    for (provider, re) in link_patterns() {
        if let Some(m) = re.find(text) {
            let url = m.as_str().trim_end_matches([')', '.', ',', ';', ':', '!', '?']);
            if url.is_empty() {
                continue;
            }
            return Some(ConferenceLink { provider: *provider, url: url.to_string() });
        }
    }
    None
}

/// Location first, then notes, then the event's own URL (organizers most often put
/// the join link in Location).
pub fn detect_link(location: Option<&str>, notes: Option<&str>, url: Option<&str>) -> Option<ConferenceLink> {
    [location, notes, url].into_iter().flatten().find_map(detect_conference_link)
}

#[derive(Debug, serde::Deserialize)]
struct Event {
    id: Option<String>,
    summary: Option<String>,
    start: Option<EventDateTime>,
    end: Option<EventDateTime>,
    attendees: Option<Vec<Attendee>>,
    #[serde(rename = "hangoutLink")]
    hangout_link: Option<String>,
    location: Option<String>,
    description: Option<String>,
    status: Option<String>,
    organizer: Option<Organizer>,
}

#[derive(Debug, serde::Deserialize)]
struct Organizer {
    email: Option<String>,
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    #[serde(rename = "self")]
    is_self: Option<bool>,
}

#[derive(Debug, serde::Deserialize)]
struct EventDateTime {
    #[serde(rename = "dateTime")]
    date_time: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct Attendee {
    email: Option<String>,
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    #[serde(rename = "self")]
    is_self: Option<bool>,
    #[serde(rename = "responseStatus")]
    response_status: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct SyncEventsResponse {
    items: Option<Vec<Event>>,
    #[serde(rename = "nextSyncToken")]
    next_sync_token: Option<String>,
    #[serde(rename = "needsFullResync")]
    needs_full_resync: Option<bool>,
}

/// FR-MTG-1 for OAuth-sourced events: a conferencing link (Google's own `hangoutLink`
/// or one found in location/description) OR at least one non-self attendee. All-day
/// events (no `dateTime`) are skipped.
fn qualify(event: &Event) -> Option<QualifyingMeeting> {
    let id = event.id.clone()?;
    let start = parse_rfc3339_ms(event.start.as_ref()?.date_time.as_deref()?)?;
    let end = parse_rfc3339_ms(event.end.as_ref()?.date_time.as_deref()?)?;
    let link = event
        .hangout_link
        .as_deref()
        .filter(|u| !u.is_empty())
        .map(|u| ConferenceLink { provider: MeetingProvider::GoogleMeet, url: u.to_string() })
        .or_else(|| detect_link(event.location.as_deref(), event.description.as_deref(), None));
    let others: Vec<&Attendee> = event.attendees.iter().flatten().filter(|a| !a.is_self.unwrap_or(false)).collect();
    if link.is_none() && others.is_empty() {
        return None;
    }
    Some(QualifyingMeeting {
        id,
        title: event.summary.as_deref().filter(|s| !s.is_empty()).unwrap_or("Untitled meeting").to_string(),
        start_unix_ms: start,
        end_unix_ms: end,
        attendees: others.iter().filter_map(|a| a.display_name.clone().or_else(|| a.email.clone())).collect(),
        conference_link: link,
        source: MeetingSource::GoogleCalendar,
        app_bundle_id: None,
        organizer_email: event.organizer.as_ref().and_then(|o| o.email.clone()),
        organizer_name: event.organizer.as_ref().and_then(|o| o.display_name.clone()),
        self_is_organizer: event.organizer.as_ref().and_then(|o| o.is_self).unwrap_or(false),
        self_response: event.attendees.iter().flatten().find(|a| a.is_self.unwrap_or(false)).and_then(|a| match a.response_status.as_deref()? {
            // A status Google adds later must not fail the whole poll, so match on the string.
            "accepted" => Some(SelfResponse::Accepted),
            "tentative" => Some(SelfResponse::Tentative),
            "needsAction" => Some(SelfResponse::NeedsAction),
            "declined" => Some(SelfResponse::Declined),
            _ => None,
        }),
    })
}

/// Google returns RFC 3339 timestamps, sometimes with fractional seconds.
fn parse_rfc3339_ms(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s).ok().map(|d| d.timestamp_millis())
}

/// Decodes a `list_events` response (Google's Events resource). Malformed JSON or a
/// missing `items` array degrade to an empty list: this feeds a background poll.
pub fn qualifying_meetings_from_list_events(json: &str) -> Vec<QualifyingMeeting> {
    serde_json::from_str::<SyncEventsResponse>(json)
        .map(|r| r.items.unwrap_or_default().iter().filter_map(qualify).collect())
        .unwrap_or_default()
}

/// The result of one `sync_events` call.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub struct SyncResult {
    pub active_meetings: Vec<QualifyingMeeting>,
    pub cancelled_external_ids: Vec<String>,
    pub next_sync_token: Option<String>,
    pub needs_full_resync: bool,
}

/// Decodes `sync_events`'s raw response. A cancelled event is routed to
/// `cancelled_external_ids` before `qualify` (which only checks link/attendees, which a
/// cancelled event can still have). Malformed JSON degrades to an empty, non-resync
/// result.
pub fn sync_result_from_json(json: &str) -> SyncResult {
    let Ok(response) = serde_json::from_str::<SyncEventsResponse>(json) else {
        return SyncResult::default();
    };
    if response.needs_full_resync == Some(true) {
        return SyncResult { needs_full_resync: true, ..Default::default() };
    }
    let events = response.items.unwrap_or_default();
    let is_cancelled = |e: &Event| e.status.as_deref() == Some("cancelled");
    SyncResult {
        cancelled_external_ids: events.iter().filter(|e| is_cancelled(e)).filter_map(|e| e.id.clone()).collect(),
        active_meetings: events.iter().filter(|e| !is_cancelled(e)).filter_map(qualify).collect(),
        next_sync_token: response.next_sync_token,
        needs_full_resync: false,
    }
}

/// Merges EventKit's meetings with the OAuth connector's, deduping a meeting synced
/// into both by exact start time + case-insensitive title; the OAuth copy wins (richer
/// attendee data). The same key also dedupes a meeting invited to two connected
/// accounts (the first account's copy wins). Sorted by start time.
pub fn merge_meetings(event_kit: &[QualifyingMeeting], oauth: &[QualifyingMeeting]) -> Vec<QualifyingMeeting> {
    let mut seen = std::collections::HashSet::new();
    let oauth: Vec<&QualifyingMeeting> = oauth.iter().filter(|o| seen.insert((o.start_unix_ms, o.title.to_lowercase()))).collect();
    let mut matched = std::collections::HashSet::new();
    for o in &oauth {
        if let Some(i) = event_kit
            .iter()
            .position(|e| e.start_unix_ms == o.start_unix_ms && e.title.eq_ignore_ascii_case(&o.title))
        {
            matched.insert(i);
        }
    }
    let mut merged: Vec<QualifyingMeeting> = oauth
        .into_iter()
        .cloned()
        .chain(event_kit.iter().enumerate().filter(|(i, _)| !matched.contains(i)).map(|(_, m)| m.clone()))
        .collect();
    merged.sort_by_key(|m| m.start_unix_ms);
    merged
}

/// Folds one incremental result into an account's known meetings: cancelled ids are
/// removed, active meetings replace a same-id entry or are appended, and meetings that
/// have already ended are pruned. An empty incremental response therefore keeps the
/// list intact rather than blanking it.
pub fn apply_sync_result(existing: &[QualifyingMeeting], result: &SyncResult, now_unix_ms: i64) -> Vec<QualifyingMeeting> {
    let cancelled: std::collections::HashSet<&str> = result.cancelled_external_ids.iter().map(String::as_str).collect();
    let mut merged: Vec<QualifyingMeeting> = existing.iter().filter(|m| !cancelled.contains(m.id.as_str())).cloned().collect();
    for meeting in &result.active_meetings {
        match merged.iter_mut().find(|m| m.id == meeting.id) {
            Some(slot) => *slot = meeting.clone(),
            None => merged.push(meeting.clone()),
        }
    }
    merged.retain(|m| m.end_unix_ms > now_unix_ms);
    merged
}

/// Phase 1: the `sync_events` call for `account_email`, carrying its stored sync
/// token (or none for a full baseline).
pub fn sync_request(store: &Store, account_email: &str) -> rusqlite::Result<ToolCall> {
    let args = match store.get_calendar_sync_token(account_email)? {
        Some(token) => serde_json::json!({ "sync_token": token }).to_string(),
        None => "{}".to_string(),
    };
    Ok(ToolCall { tool: "sync_events".into(), args_json: args })
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ApplyOutcome {
    /// The stored token was cleared; re-request once for a full baseline.
    pub needs_full_resync: bool,
    /// The account's known meetings after folding the result in.
    pub meetings: Vec<QualifyingMeeting>,
    pub cancelled_count: usize,
    /// Deleting the cancelled events failed, so the token was deliberately not saved:
    /// Google never re-reports a cancellation against a newer token.
    pub deletion_failed: bool,
    pub token_saved: bool,
}

/// Phase 2: apply one `sync_events` result. Cancelled events are deleted from `items`
/// directly (the generic upsert path has no notion of an item disappearing); the new
/// token is saved only after that succeeds.
pub fn apply(
    store: &Store,
    account_email: &str,
    result_json: &str,
    existing: &[QualifyingMeeting],
    now_unix_ms: i64,
) -> rusqlite::Result<ApplyOutcome> {
    let result = sync_result_from_json(result_json);
    if result.needs_full_resync {
        store.clear_calendar_sync_token(account_email)?;
        return Ok(ApplyOutcome { needs_full_resync: true, meetings: existing.to_vec(), cancelled_count: 0, deletion_failed: false, token_saved: false });
    }
    let meetings = apply_sync_result(existing, &result, now_unix_ms);
    let deletion_failed = !result.cancelled_external_ids.is_empty()
        && store.delete_calendar_event_items(&result.cancelled_external_ids).is_err();
    let mut token_saved = false;
    if !deletion_failed {
        if let Some(token) = &result.next_sync_token {
            token_saved = store.set_calendar_sync_token(account_email, token).is_ok();
        }
    }
    Ok(ApplyOutcome { needs_full_resync: false, meetings, cancelled_count: result.cancelled_external_ids.len(), deletion_failed, token_saved })
}

/// The `items` calendar-event upsert input for a calendar-sourced meeting; `None` for
/// a detected meeting, which has no calendar entry and gets its `meeting` row through
/// the trigger flow. Serializes with the FFI's `NewCalendarEventItem` field names.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CalendarEventItemInput {
    pub device_id: String,
    pub source: String,
    pub external_id: String,
    pub ts_unix_ms: i64,
    pub title: String,
    pub actors: Vec<String>,
}

impl From<CalendarEventItemInput> for NewCalendarEventItem {
    fn from(i: CalendarEventItemInput) -> Self {
        NewCalendarEventItem { device_id: i.device_id, source: i.source, external_id: i.external_id, ts_unix_ms: i.ts_unix_ms, title: i.title, actors: i.actors }
    }
}

pub fn calendar_event_item(meeting: &QualifyingMeeting, device_id: &str) -> Option<CalendarEventItemInput> {
    let source = match meeting.source {
        MeetingSource::Calendar => "apple_calendar",
        MeetingSource::GoogleCalendar => "google_calendar",
        MeetingSource::Detected => return None,
    };
    Some(CalendarEventItemInput {
        device_id: device_id.to_string(),
        source: source.to_string(),
        external_id: format!("{}#{}", meeting.id, meeting.start_unix_ms.div_euclid(1000)),
        ts_unix_ms: meeting.start_unix_ms,
        title: meeting.title.clone(),
        actors: meeting.attendees.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meeting(id: &str, title: &str, start_unix_ms: i64, source: MeetingSource) -> QualifyingMeeting {
        QualifyingMeeting {
            id: id.into(),
            title: title.into(),
            start_unix_ms,
            end_unix_ms: start_unix_ms + 1_800_000,
            attendees: vec![],
            conference_link: None,
            source,
            app_bundle_id: None,
            organizer_email: None,
            organizer_name: None,
            self_is_organizer: false,
            self_response: None,
        }
    }

    // ---- organizer and own response ----

    fn one(json: &str) -> QualifyingMeeting {
        qualifying_meetings_from_list_events(json).remove(0)
    }

    #[test]
    fn organizer_and_self_response_parse() {
        let m = one(r#"{"items": [{"id": "e1", "start": {"dateTime": "2026-08-05T13:00:00-04:00"}, "end": {"dateTime": "2026-08-05T13:30:00-04:00"}, "organizer": {"email": "a@x.com", "displayName": "Ana Ruiz"}, "attendees": [{"email": "mike@x.com", "self": true, "responseStatus": "tentative"}, {"email": "a@x.com"}]}]}"#);
        assert_eq!(m.organizer_email.as_deref(), Some("a@x.com"));
        assert_eq!(m.organizer_name.as_deref(), Some("Ana Ruiz"));
        assert!(!m.self_is_organizer);
        assert_eq!(m.self_response, Some(SelfResponse::Tentative));
    }

    #[test]
    fn qualifying_meeting_default_is_empty() {
        let m = QualifyingMeeting::default();
        assert_eq!((m.id.as_str(), m.title.as_str()), ("", ""));
        assert!(m.attendees.is_empty());
        assert!(m.conference_link.is_none());
        assert_eq!((m.organizer_email, m.organizer_name, m.self_is_organizer, m.self_response), (None, None, false, None));
    }

    #[test]
    fn self_organizer_detected() {
        let m = one(r#"{"items": [{"id": "e1", "start": {"dateTime": "2026-08-05T13:00:00-04:00"}, "end": {"dateTime": "2026-08-05T13:30:00-04:00"}, "organizer": {"email": "mike@x.com", "self": true}, "attendees": [{"email": "b@x.com"}]}]}"#);
        assert!(m.self_is_organizer);
    }

    #[test]
    fn missing_fields_default() {
        let m = one(r#"{"items": [{"id": "e1", "start": {"dateTime": "2026-08-05T13:00:00-04:00"}, "end": {"dateTime": "2026-08-05T13:30:00-04:00"}, "attendees": [{"email": "b@x.com"}]}]}"#);
        assert_eq!((m.organizer_email, m.organizer_name, m.self_is_organizer, m.self_response), (None, None, false, None));
    }

    // ---- qualifying_meetings_from_list_events (ported from GoogleCalendarMeetingDecodingTests) ----

    #[test]
    fn qualifies_an_event_with_a_hangout_link() {
        let json = r#"{"items": [{"id": "abc123", "summary": "Standup", "start": {"dateTime": "2026-08-05T13:00:00-04:00"}, "end": {"dateTime": "2026-08-05T13:30:00-04:00"}, "hangoutLink": "https://meet.google.com/xyz-abcd-efg", "attendees": []}]}"#;
        let meetings = qualifying_meetings_from_list_events(json);
        assert_eq!(meetings.len(), 1);
        assert_eq!(meetings[0].id, "abc123");
        assert_eq!(meetings[0].title, "Standup");
        assert_eq!(meetings[0].source, MeetingSource::GoogleCalendar);
        assert_eq!(meetings[0].conference_link.as_ref().unwrap().provider, MeetingProvider::GoogleMeet);
        assert_eq!(meetings[0].start_unix_ms, 1785949200000, "13:00 at -04:00 is 17:00Z");
        assert_eq!(meetings[0].end_unix_ms - meetings[0].start_unix_ms, 1_800_000);
    }

    #[test]
    fn qualifies_an_event_with_a_non_self_attendee_and_no_link() {
        let json = r#"{"items": [{"id": "def456", "summary": "1:1 with Alice", "start": {"dateTime": "2026-08-05T14:00:00-04:00"}, "end": {"dateTime": "2026-08-05T14:30:00-04:00"}, "attendees": [{"email": "me@example.com", "self": true}, {"email": "alice@example.com", "displayName": "Alice"}]}]}"#;
        let meetings = qualifying_meetings_from_list_events(json);
        assert_eq!(meetings.len(), 1);
        assert_eq!(meetings[0].attendees, ["Alice"]);
    }

    #[test]
    fn excludes_an_event_with_no_link_and_only_self_as_attendee() {
        let json = r#"{"items": [{"id": "ghi789", "summary": "Focus time", "start": {"dateTime": "2026-08-05T15:00:00-04:00"}, "end": {"dateTime": "2026-08-05T16:00:00-04:00"}, "attendees": [{"email": "me@example.com", "self": true}]}]}"#;
        assert!(qualifying_meetings_from_list_events(json).is_empty());
    }

    #[test]
    fn skips_an_all_day_event_with_no_date_time() {
        let json = r#"{"items": [{"id": "jkl012", "summary": "Company holiday", "start": {"date": "2026-08-05"}, "end": {"date": "2026-08-06"}, "attendees": [{"email": "someone@example.com"}]}]}"#;
        assert!(qualifying_meetings_from_list_events(json).is_empty());
    }

    #[test]
    fn malformed_json_and_empty_items_return_empty() {
        assert!(qualifying_meetings_from_list_events("not json").is_empty());
        assert!(qualifying_meetings_from_list_events(r#"{"items": []}"#).is_empty());
    }

    #[test]
    fn untitled_and_fractional_seconds_and_location_links() {
        let json = r#"{"items": [{"id": "x", "summary": "", "start": {"dateTime": "2026-01-01T10:00:00.500Z"}, "end": {"dateTime": "2026-01-01T11:00:00Z"}, "location": "Join: https://zoom.us/j/123456789?pwd=abc (password 4567)."}]}"#;
        let meetings = qualifying_meetings_from_list_events(json);
        assert_eq!(meetings[0].title, "Untitled meeting");
        assert_eq!(meetings[0].start_unix_ms % 1000, 500);
        assert_eq!(meetings[0].conference_link, Some(ConferenceLink { provider: MeetingProvider::Zoom, url: "https://zoom.us/j/123456789?pwd=abc".into() }));
    }

    // ---- merge_meetings ----

    #[test]
    fn exact_start_and_title_match_collapses_to_oauth_copy() {
        let start = 1_754_413_200_000;
        let merged = merge_meetings(&[meeting("ek-1", "Standup", start, MeetingSource::Calendar)], &[meeting("oauth-1", "STANDUP", start, MeetingSource::GoogleCalendar)]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].id, "oauth-1");
    }

    #[test]
    fn different_start_times_or_titles_do_not_collapse() {
        let start = 1_754_413_200_000;
        assert_eq!(merge_meetings(&[meeting("ek-1", "Standup", start, MeetingSource::Calendar)], &[meeting("oauth-1", "Standup", start + 60_000, MeetingSource::GoogleCalendar)]).len(), 2);
        assert_eq!(merge_meetings(&[meeting("ek-1", "Standup", start, MeetingSource::Calendar)], &[meeting("oauth-1", "Planning", start, MeetingSource::GoogleCalendar)]).len(), 2);
    }

    #[test]
    fn empty_oauth_passes_event_kit_through_and_result_is_sorted() {
        let start = 1_754_413_200_000;
        let merged = merge_meetings(&[meeting("ek-1", "Detected meeting", start, MeetingSource::Detected)], &[]);
        assert_eq!(merged.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["ek-1"]);
        let merged = merge_meetings(&[meeting("ek-1", "Later meeting", start + 3_600_000, MeetingSource::Calendar)], &[meeting("oauth-1", "Earlier meeting", start, MeetingSource::GoogleCalendar)]);
        assert_eq!(merged.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["oauth-1", "ek-1"]);
    }

    #[test]
    fn same_meeting_from_two_google_accounts_appears_once() {
        let start = 1_754_413_200_000;
        let merged = merge_meetings(
            &[],
            &[
                meeting("work-1", "FMA Partner Connect", start, MeetingSource::GoogleCalendar),
                meeting("home-1", "fma partner connect", start, MeetingSource::GoogleCalendar),
                meeting("work-2", "FMA Partner Connect", start + 86_400_000, MeetingSource::GoogleCalendar),
            ],
        );
        assert_eq!(merged.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["work-1", "work-2"]);
    }

    // ---- sync_result_from_json ----

    #[test]
    fn sync_result_separates_cancelled_from_active_events() {
        let json = r#"{"items": [
            {"id": "evt-1", "summary": "Active meeting", "start": {"dateTime": "2026-01-01T10:00:00Z"}, "end": {"dateTime": "2026-01-01T11:00:00Z"}, "hangoutLink": "https://meet.google.com/abc", "status": "confirmed"},
            {"id": "evt-2", "status": "cancelled"}
        ], "nextSyncToken": "token-xyz"}"#;
        let result = sync_result_from_json(json);
        assert_eq!(result.active_meetings.len(), 1);
        assert_eq!(result.active_meetings[0].id, "evt-1");
        assert_eq!(result.cancelled_external_ids, ["evt-2"]);
        assert_eq!(result.next_sync_token.as_deref(), Some("token-xyz"));
        assert!(!result.needs_full_resync);
    }

    #[test]
    fn sync_result_detects_needs_full_resync_and_degrades_on_malformed_json() {
        let result = sync_result_from_json(r#"{"needsFullResync": true}"#);
        assert!(result.needs_full_resync && result.active_meetings.is_empty() && result.cancelled_external_ids.is_empty());
        let result = sync_result_from_json("not json");
        assert!(!result.needs_full_resync && result.active_meetings.is_empty() && result.cancelled_external_ids.is_empty());
    }

    #[test]
    fn sync_result_does_not_qualify_a_cancelled_event_even_if_it_has_a_conference_link() {
        let json = r#"{"items": [{"id": "evt-1", "summary": "Was going to be a meeting", "start": {"dateTime": "2026-01-01T10:00:00Z"}, "end": {"dateTime": "2026-01-01T11:00:00Z"}, "hangoutLink": "https://meet.google.com/abc", "status": "cancelled"}]}"#;
        let result = sync_result_from_json(json);
        assert!(result.active_meetings.is_empty());
        assert_eq!(result.cancelled_external_ids, ["evt-1"]);
    }

    // ---- apply_sync_result ----

    const NOW: i64 = 1_000_000_000;

    fn known(id: &str, title: &str, ends_in_ms: i64) -> QualifyingMeeting {
        let mut m = meeting(id, title, NOW, MeetingSource::GoogleCalendar);
        m.end_unix_ms = NOW + ends_in_ms;
        m
    }

    fn sync(active: Vec<QualifyingMeeting>, cancelled: &[&str]) -> SyncResult {
        SyncResult { active_meetings: active, cancelled_external_ids: cancelled.iter().map(|s| s.to_string()).collect(), next_sync_token: None, needs_full_resync: false }
    }

    #[test]
    fn apply_sync_result_keeps_existing_when_nothing_changed_and_removes_cancelled() {
        let existing = [known("evt-1", "Meeting", 3_600_000), known("evt-2", "Meeting", 3_600_000)];
        let ids = |v: Vec<QualifyingMeeting>| v.into_iter().map(|m| m.id).collect::<Vec<_>>();
        assert_eq!(ids(apply_sync_result(&existing, &sync(vec![], &[]), NOW)), ["evt-1", "evt-2"]);
        assert_eq!(ids(apply_sync_result(&existing, &sync(vec![], &["evt-1"]), NOW)), ["evt-2"]);
    }

    #[test]
    fn apply_sync_result_replaces_adds_and_prunes_ended() {
        let merged = apply_sync_result(&[known("evt-1", "Old title", 3_600_000)], &sync(vec![known("evt-1", "New title", 3_600_000)], &[]), NOW);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].title, "New title");
        let mut merged = apply_sync_result(&[known("evt-1", "M", 3_600_000)], &sync(vec![known("evt-2", "M", 3_600_000)], &[]), NOW);
        merged.sort_by(|a, b| a.id.cmp(&b.id));
        assert_eq!(merged.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["evt-1", "evt-2"]);
        let merged = apply_sync_result(&[known("past", "M", -60_000), known("future", "M", 3_600_000)], &sync(vec![], &[]), NOW);
        assert_eq!(merged.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["future"]);
    }

    // ---- calendar_event_item (ported from CalendarEventItemMappingTests) ----

    fn mapping_meeting(source: MeetingSource) -> QualifyingMeeting {
        QualifyingMeeting {
            id: "evt-123".into(),
            title: "Planning".into(),
            start_unix_ms: 1_000_000,
            end_unix_ms: 2_000_000,
            attendees: vec!["alice@x.com".into(), "bob@x.com".into()],
            conference_link: None,
            source,
            app_bundle_id: None,
            organizer_email: None,
            organizer_name: None,
            self_is_organizer: false,
            self_response: None,
        }
    }

    #[test]
    fn maps_calendar_sources_and_skips_detected() {
        assert_eq!(
            calendar_event_item(&mapping_meeting(MeetingSource::Calendar), "dev-1"),
            Some(CalendarEventItemInput { device_id: "dev-1".into(), source: "apple_calendar".into(), external_id: "evt-123#1000".into(), ts_unix_ms: 1_000_000, title: "Planning".into(), actors: vec!["alice@x.com".into(), "bob@x.com".into()] })
        );
        assert_eq!(calendar_event_item(&mapping_meeting(MeetingSource::GoogleCalendar), "dev-1").unwrap().source, "google_calendar");
        assert!(calendar_event_item(&mapping_meeting(MeetingSource::Detected), "dev-1").is_none());
    }

    // ---- detect_conference_link (MeetingLinkDetector) ----

    #[test]
    fn detects_each_provider_and_trims_trailing_punctuation() {
        assert_eq!(detect_conference_link("join at https://zoom.us/j/123 (password 4567).").unwrap(), ConferenceLink { provider: MeetingProvider::Zoom, url: "https://zoom.us/j/123".into() });
        assert_eq!(detect_conference_link("https://us02web.zoom.us/j/999?pwd=x,").unwrap().url, "https://us02web.zoom.us/j/999?pwd=x");
        assert_eq!(detect_conference_link("<https://meet.google.com/abc-defg-hij>").unwrap().provider, MeetingProvider::GoogleMeet);
        assert_eq!(detect_conference_link("https://teams.live.com/meet/1").unwrap().provider, MeetingProvider::Teams);
        assert_eq!(detect_conference_link("https://teams.microsoft.com/l/meetup-join/x").unwrap().provider, MeetingProvider::Teams);
        assert_eq!(detect_conference_link("see https://acme.webex.com/meet/bob").unwrap().provider, MeetingProvider::Webex);
        assert!(detect_conference_link("no link here").is_none());
        assert_eq!(detect_link(None, Some("https://meet.google.com/x"), Some("https://zoom.us/j/1")).unwrap().provider, MeetingProvider::GoogleMeet);
    }

    #[test]
    fn json_shapes_use_camel_case_enum_values() {
        let m = meeting("a", "T", 5, MeetingSource::GoogleCalendar);
        let json = serde_json::to_value(&m).unwrap();
        assert_eq!(json["source"], "googleCalendar");
        assert_eq!(serde_json::to_value(MeetingProvider::GoogleMeet).unwrap(), "googleMeet");
        assert_eq!(serde_json::from_value::<QualifyingMeeting>(json).unwrap(), m);
    }

    // ---- sync_request / apply against a real store ----

    #[test]
    fn request_carries_the_stored_token_and_apply_advances_it_only_after_deletions_succeed() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(sync_request(&store, "a@x.com").unwrap(), ToolCall { tool: "sync_events".into(), args_json: "{}".into() });
        let json = r#"{"items": [
            {"id": "evt-1", "summary": "Active", "start": {"dateTime": "2100-01-01T10:00:00Z"}, "end": {"dateTime": "2100-01-01T11:00:00Z"}, "hangoutLink": "https://meet.google.com/abc"},
            {"id": "evt-2", "status": "cancelled"}
        ], "nextSyncToken": "tok-1"}"#;
        let out = apply(&store, "a@x.com", json, &[], 0).unwrap();
        assert!(!out.needs_full_resync && !out.deletion_failed && out.token_saved);
        assert_eq!(out.cancelled_count, 1);
        assert_eq!(out.meetings.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["evt-1"]);
        let req = sync_request(&store, "a@x.com").unwrap();
        assert_eq!(serde_json::from_str::<serde_json::Value>(&req.args_json).unwrap()["sync_token"], "tok-1");

        // A resync response clears the token and keeps the known list.
        let out = apply(&store, "a@x.com", r#"{"needsFullResync": true}"#, &out.meetings, 0).unwrap();
        assert!(out.needs_full_resync);
        assert_eq!(out.meetings.len(), 1);
        assert_eq!(sync_request(&store, "a@x.com").unwrap().args_json, "{}");

        // Malformed result: nothing changes, no token.
        let out = apply(&store, "a@x.com", "not json", &out.meetings, 0).unwrap();
        assert!(!out.needs_full_resync && !out.token_saved);
        assert_eq!(out.meetings.len(), 1);
    }
}
