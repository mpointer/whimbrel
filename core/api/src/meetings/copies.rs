//! One call recorded more than once — on several Macs in one room, or twice on one Mac
//! after a relaunch — shows as one meeting: recordings that start together and overlap by
//! at least half the shorter one are grouped, and the best copy stands for the group.
//! (A summary-similarity check missed half the real copies, so it isn't used.) Display
//! only: nothing is merged, rewritten or deleted.

use memory::{Meeting, MeetingStatus};

/// The title prefix the Mac gives a calendar recording that had no system audio: that
/// Mac wasn't in the call, so another copy is the better one to show.
pub const ROOM_AUDIO_PREFIX: &str = "Room audio only · ";

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MeetingCopy {
    pub id: i64,
    pub device_id: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GroupedMeeting {
    #[serde(flatten)]
    pub meeting: Meeting,
    /// The other recordings of the same call, best first.
    pub also_recorded: Vec<MeetingCopy>,
}

/// Copies of one call start together (at the event or when the call is detected); a
/// meeting that merely falls inside a long recording starts later.
const MAX_START_GAP_MS: i64 = 10 * 60_000;

/// Shorter recordings stay ungrouped: half of almost nothing overlaps anything.
const MIN_GROUPED_MS: i64 = 2 * 60_000;

/// Two recordings are copies of one call when they start within `MAX_START_GAP_MS` and
/// overlap by at least half the shorter one. Meetings still recording never match.
/// A recording under `MIN_GROUPED_MS` only counts when it carries the same title and
/// the same start as the other: two Macs recording one calendar event both start at the
/// event's start, so a brief copy of it is still that call.
fn same_call(a: &Meeting, b: &Meeting) -> bool {
    let (Some((a_start, a_end)), Some((b_start, b_end))) = (span(a), span(b)) else { return false };
    let overlap = a_end.min(b_end) - a_start.max(b_start);
    let shorter = (a_end - a_start).min(b_end - b_start);
    let same_event = a_start == b_start && a.title == b.title;
    (shorter >= MIN_GROUPED_MS || same_event) && (a_start - b_start).abs() <= MAX_START_GAP_MS && overlap * 2 >= shorter
}

fn span(m: &Meeting) -> Option<(i64, i64)> {
    match (m.status, m.end_unix_ms) {
        (MeetingStatus::Recording, _) | (_, None) => None,
        (_, Some(end)) => Some((m.start_unix_ms, end.max(m.start_unix_ms + 1))),
    }
}

/// Groups `meetings` (in display order, newest first) and returns one entry per group at
/// the position of its newest member. Grouping isn't transitive: every member must match
/// the group's first meeting, so one long recording can't chain unrelated meetings.
pub fn group_copies(meetings: Vec<Meeting>) -> Vec<GroupedMeeting> {
    let n = meetings.len();
    let mut assigned = vec![false; n];
    let mut groups: Vec<Vec<usize>> = Vec::new();
    // ponytail: O(n²) over one list page (a few hundred rows); sort by start if lists grow.
    for i in 0..n {
        if assigned[i] {
            continue;
        }
        assigned[i] = true;
        let mut members = vec![i];
        for j in i + 1..n {
            if !assigned[j] && same_call(&meetings[i], &meetings[j]) {
                assigned[j] = true;
                members.push(j);
            }
        }
        groups.push(members);
    }
    // Best copy: recorded in the call (not room audio), has a transcript, longest; ties
    // keep display order.
    let rank = |m: &Meeting| {
        (
            !m.title.starts_with(ROOM_AUDIO_PREFIX),
            m.transcript_path.is_some(),
            span(m).map_or(0, |(s, e)| e - s),
        )
    };
    let mut slots: Vec<Option<Meeting>> = meetings.into_iter().map(Some).collect();
    groups
        .into_iter()
        .map(|mut members| {
            members.sort_by(|&x, &y| {
                let (mx, my) = (slots[x].as_ref().unwrap(), slots[y].as_ref().unwrap());
                rank(my).cmp(&rank(mx)).then(x.cmp(&y))
            });
            let meeting = slots[members[0]].take().unwrap();
            let also_recorded = members[1..]
                .iter()
                .map(|&k| {
                    let m = slots[k].take().unwrap();
                    MeetingCopy { id: m.id, device_id: m.device_id, title: m.title }
                })
                .collect();
            GroupedMeeting { meeting, also_recorded }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use memory::MeetingTrigger;

    const MIN: i64 = 60_000;

    fn meeting(id: i64, device: &str, title: &str, start_min: i64, end_min: i64, transcript: bool) -> Meeting {
        Meeting {
            id,
            device_id: device.to_string(),
            start_unix_ms: start_min * MIN,
            end_unix_ms: Some(end_min * MIN),
            title: title.to_string(),
            attendees: vec![],
            trigger: MeetingTrigger::Calendar,
            audio_path: None,
            status: MeetingStatus::Completed,
            transcript_path: transcript.then(|| "/t.txt".to_string()),
            notes: None,
            summary_tldr: None,
            summary_decisions: vec![],
            summary_action_items: vec![],
            summary_open_questions: vec![],
            summary_provider: None,
            summary_model: None,
            local_seq: None,
            estimated_speaker_count: None,             away_outcome: None,             away_outcome_at_unix_ms: None, keep_media: false,
        }
    }

    #[test]
    fn one_call_on_three_macs_is_one_entry_led_by_the_copy_that_was_in_the_call() {
        // The 9/25 partner call: this Mac 16:00-16:35, the third laptop 15:59-16:07, and
        // the work Mac's room recording under its calendar event 16:00-16:09.
        let grouped = group_copies(vec![
            meeting(212, "work", "Room audio only · CMS Daily Win Room", 960, 969, true),
            meeting(211, "third", "Rock Arkie's Zoom Meeting", 959, 967, true),
            meeting(210, "this", "Rock Arkie's Zoom Meeting", 960, 995, true),
        ]);
        assert_eq!(grouped.len(), 1);
        assert_eq!(grouped[0].meeting.id, 210, "the longest in-call copy leads");
        assert_eq!(grouped[0].also_recorded.iter().map(|c| c.id).collect::<Vec<_>>(), [211, 212]);
    }

    #[test]
    fn a_relaunch_duplicate_on_one_mac_groups_under_the_transcribed_copy() {
        let grouped = group_copies(vec![
            meeting(190, "this", "GN: NA AI Team Meeting", 930, 980, false),
            meeting(187, "this", "GN: NA AI Team Meeting", 930, 954, true),
        ]);
        assert_eq!(grouped.len(), 1);
        assert_eq!(grouped[0].meeting.id, 187);
    }

    #[test]
    fn meetings_overlapping_less_than_half_the_shorter_stay_separate() {
        let grouped = group_copies(vec![
            meeting(28, "b", "Koozie AI Strategy", 30, 90, true),
            meeting(27, "a", "Records Management", 0, 48, true), // 18 of 48 min = 38%
        ]);
        assert_eq!(grouped.iter().map(|g| g.meeting.id).collect::<Vec<_>>(), [28, 27]);
        assert!(grouped.iter().all(|g| g.also_recorded.is_empty()));
    }

    #[test]
    fn a_long_recording_does_not_absorb_or_chain_the_meetings_inside_it() {
        let grouped = group_copies(vec![
            meeting(3, "b", "Xperience 2026", 120, 150, true),
            meeting(2, "a", "AXL Session: Zurich", 60, 90, true),
            meeting(1, "a", "All hands on deck AXL help", 0, 627, true),
        ]);
        assert_eq!(grouped.len(), 3);
    }

    #[test]
    fn short_copies_of_one_calendar_event_group_when_start_and_title_match() {
        // 10/16 on two Macs: Rock Arkie's 16:00 call, 79s and 74s; FMA Partner Connect
        // 10:30, 15 min and 75s. Same event start and title, so one call.
        let mut a = meeting(259, "a", "Rock Arkie's Zoom Meeting", 960, 961, true);
        a.end_unix_ms = Some(960 * MIN + 79_000);
        let mut b = meeting(258, "b", "Rock Arkie's Zoom Meeting", 960, 961, true);
        b.end_unix_ms = Some(960 * MIN + 74_000);
        let mut c = meeting(256, "a", "FMA Partner Connect", 630, 645, true);
        c.end_unix_ms = Some(630 * MIN + 903_000);
        let mut d = meeting(255, "b", "FMA Partner Connect", 630, 631, true);
        d.end_unix_ms = Some(630 * MIN + 75_000);
        let grouped = group_copies(vec![a, b, c, d]);
        assert_eq!(grouped.iter().map(|g| g.meeting.id).collect::<Vec<_>>(), [259, 256]);
        assert_eq!(grouped[0].also_recorded.iter().map(|c| c.id).collect::<Vec<_>>(), [258]);
        assert_eq!(grouped[1].also_recorded.iter().map(|c| c.id).collect::<Vec<_>>(), [255]);
    }

    #[test]
    fn a_recording_under_two_minutes_is_never_grouped() {
        let grouped = group_copies(vec![
            meeting(47, "b", "GN: NA AI Team Meeting", 0, 60, true),
            meeting(46, "a", "One Americas Quarterly All Hands", 0, 1, true),
        ]);
        assert_eq!(grouped.len(), 2);
    }

    #[test]
    fn a_meeting_still_recording_is_never_grouped() {
        let mut live = meeting(2, "a", "Live", 0, 60, false);
        live.status = MeetingStatus::Recording;
        live.end_unix_ms = None;
        let grouped = group_copies(vec![live, meeting(1, "b", "Other", 0, 60, true)]);
        assert_eq!(grouped.len(), 2);
    }

    #[test]
    fn grouped_meeting_json_keeps_meeting_fields_flat() {
        let json = serde_json::to_value(&group_copies(vec![meeting(1, "a", "T", 0, 10, false)])).unwrap();
        assert_eq!(json[0]["id"], 1);
        assert_eq!(json[0]["also_recorded"], serde_json::json!([]));
    }
}
