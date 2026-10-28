//! FR-MTG-1: which calendar meetings are due a recording prompt. Ported from
//! apps/macos `MeetingTriggerService.evaluate`.

use std::collections::HashSet;

use crate::connectors::calendar::QualifyingMeeting;

/// Returns every meeting whose lead window has opened (`now >= start - lead`) and that
/// hasn't ended (`now < end`), skipping ids already in `acted_on` and adding the ones it
/// returns, so each meeting prompts once. `acted_on` lives in the shell, which persists
/// it until each event ends so a relaunch mid-meeting doesn't prompt it again.
/// Returned in `meetings` order; the Mac shows only the last one it sets.
pub fn due_prompts(
    meetings: &[QualifyingMeeting],
    acted_on: &mut HashSet<String>,
    now_unix_ms: i64,
    lead_minutes: f64,
) -> Vec<QualifyingMeeting> {
    let lead_ms = (lead_minutes * 60_000.0).round() as i64;
    let mut due = Vec::new();
    for meeting in meetings {
        if acted_on.contains(&meeting.id) {
            continue;
        }
        if now_unix_ms < meeting.start_unix_ms - lead_ms || now_unix_ms >= meeting.end_unix_ms {
            continue;
        }
        acted_on.insert(meeting.id.clone());
        due.push(meeting.clone());
    }
    due
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connectors::calendar::MeetingSource;

    const MIN: i64 = 60_000;

    fn meeting(id: &str, start: i64, end: i64) -> QualifyingMeeting {
        QualifyingMeeting {
            id: id.to_string(),
            title: id.to_string(),
            start_unix_ms: start,
            end_unix_ms: end,
            attendees: Vec::new(),
            conference_link: None,
            source: MeetingSource::GoogleCalendar,
            app_bundle_id: None,
            organizer_email: None,
            organizer_name: None,
            self_is_organizer: false,
            self_response: None,
        }
    }

    fn ids(v: &[QualifyingMeeting]) -> Vec<&str> {
        v.iter().map(|m| m.id.as_str()).collect()
    }

    #[test]
    fn not_due_before_the_lead_window() {
        let mut acted = HashSet::new();
        let m = meeting("a", 100 * MIN, 130 * MIN);
        assert!(due_prompts(&[m], &mut acted, 98 * MIN, 1.0).is_empty());
        assert!(acted.is_empty());
    }

    #[test]
    fn due_exactly_when_the_lead_window_opens() {
        let mut acted = HashSet::new();
        let m = meeting("a", 100 * MIN, 130 * MIN);
        assert_eq!(ids(&due_prompts(&[m], &mut acted, 99 * MIN, 1.0)), vec!["a"]);
    }

    #[test]
    fn due_once_only() {
        let mut acted = HashSet::new();
        let m = meeting("a", 100 * MIN, 130 * MIN);
        assert_eq!(due_prompts(std::slice::from_ref(&m), &mut acted, 99 * MIN, 1.0).len(), 1);
        assert!(due_prompts(&[m], &mut acted, 105 * MIN, 1.0).is_empty());
    }

    #[test]
    fn a_meeting_joined_late_is_still_due_until_it_ends() {
        let mut acted = HashSet::new();
        let m = meeting("a", 100 * MIN, 130 * MIN);
        assert_eq!(due_prompts(std::slice::from_ref(&m), &mut acted, 129 * MIN, 1.0).len(), 1);
        let mut fresh = HashSet::new();
        assert!(due_prompts(&[m], &mut fresh, 130 * MIN, 1.0).is_empty(), "end is exclusive");
    }

    #[test]
    fn a_longer_lead_opens_the_window_earlier() {
        let mut acted = HashSet::new();
        let m = meeting("a", 100 * MIN, 130 * MIN);
        assert_eq!(due_prompts(&[m], &mut acted, 90 * MIN, 10.0).len(), 1);
    }

    #[test]
    fn only_due_meetings_are_returned_and_marked() {
        let mut acted = HashSet::new();
        let meetings = [meeting("now", 100 * MIN, 130 * MIN), meeting("later", 200 * MIN, 230 * MIN)];
        assert_eq!(ids(&due_prompts(&meetings, &mut acted, 100 * MIN, 1.0)), vec!["now"]);
        assert_eq!(acted, HashSet::from(["now".to_string()]));
    }
}
