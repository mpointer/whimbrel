//! One call recorded more than once — on several Macs in one room, or twice on one Mac
//! after a relaunch — shows as one meeting: recordings that start together and overlap by
//! at least half the shorter one are grouped, and the best copy stands for the group.
//! (A summary-similarity check missed half the real copies, so it isn't used.) Display
//! only: nothing is merged, rewritten or deleted. Search and Ask apply the same rule to
//! their hits ([`Store::drop_copy_hits`]), so a copy's text doesn't stand beside its lead's.

use std::collections::{HashMap, HashSet};

use rusqlite::params;

use crate::{Meeting, MeetingStatus, Store};

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
    // A room-audio copy carries the event's title behind the prefix.
    let event_title = |m: &Meeting| m.title.strip_prefix(ROOM_AUDIO_PREFIX).unwrap_or(&m.title).to_string();
    let same_event = a_start == b_start && event_title(a) == event_title(b);
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

impl Store {
    /// Every meeting, any status, that started in `[start, end)`, newest first.
    pub fn meetings_starting_between(&self, start_unix_ms: i64, end_unix_ms: i64) -> rusqlite::Result<Vec<Meeting>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, device_id, start_unix_ms, end_unix_ms, title, attendees, trigger, audio_path, status, \
             transcript_path, notes, summary_tldr, summary_decisions_json, summary_action_items_json, summary_open_questions_json, \
             summary_provider, summary_model, local_seq, estimated_speaker_count, away_outcome, away_outcome_at_unix_ms, keep_media \
             FROM meeting WHERE start_unix_ms >= ?1 AND start_unix_ms < ?2 ORDER BY start_unix_ms DESC",
        )?;
        let rows = stmt.query_map(params![start_unix_ms, end_unix_ms], Self::row_to_meeting)?;
        rows.collect()
    }

    /// Each id's group lead, grouped as the Meetings list groups them (over the meetings
    /// that could share its call). A missing meeting maps to itself.
    pub fn copy_leads(&self, meeting_ids: &[i64]) -> rusqlite::Result<HashMap<i64, i64>> {
        let mut leads = HashMap::new();
        for &id in meeting_ids {
            if leads.contains_key(&id) {
                continue;
            }
            let Some(m) = self.get_meeting(id)? else {
                leads.insert(id, id);
                continue;
            };
            let window =
                self.meetings_starting_between(m.start_unix_ms - MAX_START_GAP_MS, m.start_unix_ms + MAX_START_GAP_MS + 1)?;
            let lead = group_copies(window)
                .into_iter()
                .find(|g| g.meeting.id == id || g.also_recorded.iter().any(|c| c.id == id))
                .map_or(id, |g| g.meeting.id);
            leads.insert(id, lead);
        }
        Ok(leads)
    }

    /// `hits` without those from a copy whose lead also has a hit, order kept.
    /// ponytail: runs after the search's LIMIT, so a search can return fewer than `limit`;
    /// over-fetch if that ever matters.
    pub(crate) fn drop_copy_hits<T>(&self, hits: Vec<T>, meeting_of: impl Fn(&T) -> i64) -> rusqlite::Result<Vec<T>> {
        let ids: Vec<i64> = hits.iter().map(&meeting_of).collect();
        let leads = self.copy_leads(&ids)?;
        Ok(keep_lead_hits(hits, &leads, meeting_of))
    }
}

/// Keeps a hit when its meeting leads its group, or when no hit belongs to that lead.
pub fn keep_lead_hits<T>(hits: Vec<T>, leads: &HashMap<i64, i64>, meeting_of: impl Fn(&T) -> i64) -> Vec<T> {
    let hit_meetings: HashSet<i64> = hits.iter().map(&meeting_of).collect();
    hits.into_iter()
        .filter(|h| {
            let m = meeting_of(h);
            let lead = leads.get(&m).copied().unwrap_or(m);
            lead == m || !hit_meetings.contains(&lead)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MeetingTrigger;

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
    fn a_short_room_audio_copy_groups_with_the_in_call_recording_of_its_event() {
        // 11/2 9:30: this Mac recorded the call for 20 min; the Genesys Mac's mic picked it
        // up for 75 s under the same calendar event, titled with the room-audio prefix.
        let mut room = meeting(587, "genesys", "Room audio only · Michael R Pointer / Aditya", 570, 571, false);
        room.end_unix_ms = Some(570 * MIN + 75_000);
        let grouped = group_copies(vec![room, meeting(586, "this", "Michael R Pointer / Aditya", 570, 590, true)]);
        assert_eq!(grouped.len(), 1);
        assert_eq!(grouped[0].meeting.id, 586);
        assert_eq!(grouped[0].also_recorded.iter().map(|c| c.id).collect::<Vec<_>>(), [587]);
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

    mod store_hits {
        use crate::{MeetingStatus, MeetingTrigger, NewMeeting, NewMeetingSnapshot, NewTranscriptSegment, Store};

        const MIN: i64 = 60_000;

        fn add(store: &Store, title: &str, start_min: i64, end_min: i64, status: MeetingStatus) -> i64 {
            store
                .insert_meeting(&NewMeeting {
                    device_id: "d".into(),
                    start_unix_ms: start_min * MIN,
                    end_unix_ms: (status != MeetingStatus::Recording).then_some(end_min * MIN),
                    title: title.into(),
                    attendees: vec![],
                    trigger: MeetingTrigger::Calendar,
                    audio_path: None,
                    status,
                })
                .unwrap()
        }

        fn say(store: &Store, meeting_id: i64, start_cs: i64, text: &str) {
            store
                .insert_transcript_segments(&[NewTranscriptSegment {
                    meeting_id,
                    start_cs,
                    speaker_label: "Others".into(),
                    text: text.into(),
                }])
                .unwrap();
        }

        /// A (in the call, 30 min), B (room audio copy, 2 min later, 28 min), C (2 h earlier).
        fn three(store: &Store, b_status: MeetingStatus) -> (i64, i64, i64) {
            let c = add(store, "Standup", 0, 30, MeetingStatus::Completed);
            let a = add(store, "Planning", 120, 150, MeetingStatus::Completed);
            let b = add(store, "Room audio only · Planning", 122, 150, b_status);
            (a, b, c)
        }

        #[test]
        fn copy_leads_maps_copy_to_lead() {
            let store = Store::open_in_memory().unwrap();
            let (a, b, c) = three(&store, MeetingStatus::Completed);
            let leads = store.copy_leads(&[a, b, c]).unwrap();
            assert_eq!((leads[&a], leads[&b], leads[&c]), (a, a, c));
        }

        #[test]
        fn search_drops_copy_hits_keeps_order() {
            let store = Store::open_in_memory().unwrap();
            let (a, b, c) = three(&store, MeetingStatus::Completed);
            say(&store, c, 100, "budget review");
            say(&store, a, 100, "the budget is fine");
            say(&store, b, 100, "the budget is fine");
            let hits: Vec<i64> = store.search_transcript_keyword("budget", 50).unwrap().iter().map(|h| h.meeting_id).collect();
            assert!(!hits.contains(&b), "copy dropped: {hits:?}");
            assert_eq!(hits.len(), 2);
            let ts: Vec<i64> =
                store.search_transcript_keyword_with_meeting_ts("budget", 50).unwrap().iter().map(|(h, _)| h.meeting_id).collect();
            assert!(!ts.contains(&b));
        }

        #[test]
        fn copy_hit_kept_when_lead_has_none() {
            let store = Store::open_in_memory().unwrap();
            let (a, b, _) = three(&store, MeetingStatus::Completed);
            say(&store, a, 100, "hello");
            say(&store, b, 100, "look at the whiteboard");
            let hits = store.search_transcript_keyword("whiteboard", 50).unwrap();
            assert_eq!(hits.iter().map(|h| h.meeting_id).collect::<Vec<_>>(), [b]);
        }

        #[test]
        fn snapshot_search_drops_copy_hits() {
            let store = Store::open_in_memory().unwrap();
            let (a, b, _) = three(&store, MeetingStatus::Completed);
            for (m, at) in [(a, 121), (b, 123)] {
                store
                    .insert_meeting_snapshot(&NewMeetingSnapshot {
                        meeting_id: m,
                        captured_unix_ms: at * MIN,
                        file_path: format!("/tmp/{m}.png"),
                        width: 1,
                        height: 1,
                        ocr_text: "Roadmap slide".into(),
                    })
                    .unwrap();
            }
            let hits = store.search_snapshot_keyword_with_meeting_ts("Roadmap", 50).unwrap();
            assert_eq!(hits.iter().map(|(h, _)| h.meeting_id).collect::<Vec<_>>(), [a]);
        }

        #[test]
        fn recording_meeting_never_grouped() {
            let store = Store::open_in_memory().unwrap();
            let (a, b, _) = three(&store, MeetingStatus::Recording);
            say(&store, a, 100, "budget");
            say(&store, b, 100, "budget");
            let mut hits: Vec<i64> = store.search_transcript_keyword("budget", 50).unwrap().iter().map(|h| h.meeting_id).collect();
            hits.sort();
            assert_eq!(hits, [a, b]);
        }
    }
}
