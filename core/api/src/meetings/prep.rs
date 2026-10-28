//! Which upcoming calendar meetings are due a prep brief: the lead window is open, the
//! meeting has not started, someone else is attending, and no brief exists yet. Pure.

use std::collections::HashSet;

use crate::connectors::calendar::QualifyingMeeting;

pub fn due(
    meetings: &[QualifyingMeeting],
    prepared: &HashSet<String>,
    now_unix_ms: i64,
    lead_minutes: f64,
) -> Vec<QualifyingMeeting> {
    let lead_ms = (lead_minutes * 60_000.0).round() as i64;
    meetings
        .iter()
        .filter(|m| {
            !m.attendees.is_empty()
                && !prepared.contains(&m.id)
                && now_unix_ms >= m.start_unix_ms - lead_ms
                && now_unix_ms < m.start_unix_ms
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connectors::calendar::MeetingSource;

    const MIN: i64 = 60_000;

    fn meeting(id: &str, start: i64, attendees: &[&str]) -> QualifyingMeeting {
        QualifyingMeeting {
            id: id.into(),
            title: id.into(),
            start_unix_ms: start,
            end_unix_ms: start + 30 * MIN,
            attendees: attendees.iter().map(|s| s.to_string()).collect(),
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
    fn due_only_inside_the_lead_window() {
        let ms = [meeting("a", 100 * MIN, &["Ben"])];
        let none = HashSet::new();
        assert!(due(&ms, &none, 69 * MIN, 30.0).is_empty(), "before the window opens");
        assert_eq!(ids(&due(&ms, &none, 70 * MIN, 30.0)), ["a"], "the window opens at start - lead");
        assert_eq!(ids(&due(&ms, &none, 99 * MIN, 30.0)), ["a"]);
        assert!(due(&ms, &none, 100 * MIN, 30.0).is_empty(), "a started meeting is too late");
    }

    #[test]
    fn a_meeting_with_a_brief_is_not_due_again() {
        let ms = [meeting("a", 100 * MIN, &["Ben"]), meeting("b", 100 * MIN, &["Ben"])];
        let prepared: HashSet<String> = ["a".to_string()].into_iter().collect();
        assert_eq!(ids(&due(&ms, &prepared, 80 * MIN, 30.0)), ["b"]);
    }

    #[test]
    fn a_meeting_with_nobody_else_is_skipped() {
        let ms = [meeting("solo", 100 * MIN, &[])];
        assert!(due(&ms, &HashSet::new(), 80 * MIN, 30.0).is_empty());
    }

    #[test]
    fn the_lead_time_is_configurable() {
        let ms = [meeting("a", 100 * MIN, &["Ben"])];
        assert!(due(&ms, &HashSet::new(), 85 * MIN, 10.0).is_empty());
        assert_eq!(ids(&due(&ms, &HashSet::new(), 91 * MIN, 10.0)), ["a"]);
    }
}
