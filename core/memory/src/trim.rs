//! Trim a meeting: keep its first `keep_ms` and remove everything after
//! (docs/superpowers/specs/2026-11-05-trim-and-still-in-meeting-design.md).
//!
//! The keep time is on the transcript clock: ms from part 1's start (the recording start
//! a segment's `start_cs` counts from), or from the meeting row's start for a meeting
//! without parts. A calendar meeting's row keeps its scheduled start, so the two differ.
//!
//! No tombstones: the trimmed meeting row carries `trimmed_keep_ms`, and a peer applying
//! it deletes its own copies of the lines past it and drops any that arrive later.

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::{MeetingStatus, Store};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrimResult {
    /// Parts that started at or after the cut, now deleted; the caller deletes their files.
    pub removed_parts: Vec<i64>,
    /// `(part_index, keep_ms within that part)`: the part the cut falls in, now ending at
    /// the cut; the caller truncates its audio. `None` when the cut falls between parts.
    pub spanning_part: Option<(i64, i64)>,
    /// Snapshot files taken after the cut, whose rows are gone; the caller deletes them.
    pub removed_snapshot_paths: Vec<String>,
}

/// Live rows are timed from the start of the part being recorded (the live reader starts
/// at the part's audio file, and a continued meeting's earlier live rows are cleared), so
/// they sit this far into the transcript clock.
fn live_offset_ms(parts: &[crate::meeting_parts::MeetingPart], origin: i64) -> i64 {
    parts.last().map_or(0, |p| p.start_unix_ms - origin)
}

impl Store {
    /// Keeps the first `keep_ms` of a meeting this device recorded; anything starting at or
    /// after the cut goes. `None` (nothing changed) for a missing meeting, one still
    /// recording, another device's meeting (or this device's id unknown), or `keep_ms` not
    /// inside `(0, meeting_length_ms)`. One transaction; the meeting row is
    /// restamped and re-enqueued so peers get the new end and the trim marker.
    pub fn trim_meeting(&self, meeting_id: i64, keep_ms: i64) -> rusqlite::Result<Option<TrimResult>> {
        let Some(meeting) = self.get_meeting(meeting_id)? else {
            return Ok(None);
        };
        let not_ours = self
            .own_device_id
            .borrow()
            .as_deref()
            .is_none_or(|d| d != meeting.device_id);
        if not_ours
            || meeting.status == MeetingStatus::Recording
            || meeting.end_unix_ms.is_none()
            || keep_ms <= 0
            || keep_ms >= self.meeting_length_ms(meeting_id)?
        {
            return Ok(None);
        }
        let tx = self.conn.unchecked_transaction()?;
        // A meeting recorded before parts existed gets its part 1 row from the meeting row
        // (as `merge_into_previous` does), so the part the cut falls in is a stored one.
        tx.execute(
            "INSERT INTO meeting_part (meeting_id, part_index, start_unix_ms, end_unix_ms, audio_path, transcript_path, transcribed) \
             SELECT id, 1, start_unix_ms, end_unix_ms, audio_path, transcript_path, transcript_path IS NOT NULL FROM meeting \
             WHERE id = ?1 AND NOT EXISTS (SELECT 1 FROM meeting_part WHERE meeting_id = ?1)",
            params![meeting_id],
        )?;
        let parts = self.meeting_parts(meeting_id)?;
        let origin = parts.first().map_or(meeting.start_unix_ms, |p| p.start_unix_ms);
        let cut = origin + keep_ms;
        let removed_parts: Vec<i64> = parts
            .iter()
            .filter(|p| p.start_unix_ms >= cut)
            .map(|p| p.part_index)
            .collect();
        let spanning_part = parts
            .iter()
            .find(|p| p.start_unix_ms < cut && p.end_unix_ms.is_none_or(|e| e > cut))
            .map(|p| (p.part_index, cut - p.start_unix_ms));
        Self::delete_transcript_past_tx(&tx, meeting_id, keep_ms)?;
        tx.execute(
            "DELETE FROM live_transcript_segment WHERE meeting_id = ?1 AND ?3 + start_cs * 10 >= ?2",
            params![meeting_id, keep_ms, live_offset_ms(&parts, origin)],
        )?;
        let removed_snapshot_paths = tx
            .prepare(
                "SELECT file_path FROM meeting_snapshot WHERE meeting_id = ?1 AND captured_unix_ms >= ?2 \
                 ORDER BY captured_unix_ms, id",
            )?
            .query_map(params![meeting_id, cut], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        tx.execute(
            "DELETE FROM items WHERE source_table = 'meeting_snapshot' AND source_id IN \
             (SELECT id FROM meeting_snapshot WHERE meeting_id = ?1 AND captured_unix_ms >= ?2)",
            params![meeting_id, cut],
        )?;
        tx.execute(
            "DELETE FROM meeting_snapshot WHERE meeting_id = ?1 AND captured_unix_ms >= ?2",
            params![meeting_id, cut],
        )?;
        tx.execute(
            "DELETE FROM meeting_part WHERE meeting_id = ?1 AND start_unix_ms >= ?2",
            params![meeting_id, cut],
        )?;
        if let Some((index, _)) = spanning_part {
            tx.execute(
                "UPDATE meeting_part SET end_unix_ms = ?3 WHERE meeting_id = ?1 AND part_index = ?2",
                params![meeting_id, index, cut],
            )?;
        }
        // The row repeats a part's file paths; one that was a removed part's moves to the last
        // kept part's (NULL when it has none).
        let kept_last = parts.iter().rev().find(|p| p.start_unix_ms < cut);
        let removed = |path: &Option<String>, of: fn(&crate::meeting_parts::MeetingPart) -> &Option<String>| {
            path.is_some() && parts.iter().any(|p| p.start_unix_ms >= cut && of(p) == path)
        };
        let audio_path = if removed(&meeting.audio_path, |p| &p.audio_path) {
            kept_last.and_then(|p| p.audio_path.clone())
        } else {
            meeting.audio_path.clone()
        };
        let transcript_path = if removed(&meeting.transcript_path, |p| &p.transcript_path) {
            kept_last.and_then(|p| p.transcript_path.clone())
        } else {
            meeting.transcript_path.clone()
        };
        let local_seq = Self::next_local_seq_tx(&tx)?;
        tx.execute(
            "UPDATE meeting SET end_unix_ms = ?2, trimmed_keep_ms = ?3, local_seq = ?4, audio_path = ?5, \
             transcript_path = ?6 WHERE id = ?1",
            params![meeting_id, cut, keep_ms, local_seq, audio_path, transcript_path],
        )?;
        self.enqueue_meeting_outbox(&tx, meeting_id, local_seq)?;
        tx.commit()?;
        Ok(Some(TrimResult { removed_parts, spanning_part, removed_snapshot_paths }))
    }

    /// The meeting's length on the transcript clock: the longest of its recorded span
    /// (end minus part 1's start, or minus the row's start without parts), its last stored
    /// segment's start and its last live segment's end (live rows are timed from the last
    /// part's start). 0 for a missing meeting.
    pub fn meeting_length_ms(&self, meeting_id: i64) -> rusqlite::Result<i64> {
        let Some(meeting) = self.get_meeting(meeting_id)? else {
            return Ok(0);
        };
        let parts = self.meeting_parts(meeting_id)?;
        let origin = parts.first().map_or(meeting.start_unix_ms, |p| p.start_unix_ms);
        let span = meeting.end_unix_ms.map_or(0, |e| e - origin);
        let (segment, live): (Option<i64>, Option<i64>) = self.conn.query_row(
            "SELECT (SELECT MAX(start_cs) FROM transcript_segment WHERE meeting_id = ?1) * 10, \
             (SELECT MAX(end_cs) FROM live_transcript_segment WHERE meeting_id = ?1) * 10",
            params![meeting_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let live = live.map(|l| l + live_offset_ms(&parts, origin));
        Ok(span.max(segment.unwrap_or(0)).max(live.unwrap_or(0)))
    }

    /// `(meeting row start, trimmed_keep_ms)` when the meeting was trimmed: what sync needs
    /// to drop a pulled transcript line past the trim. A line at `start_cs` is past it when
    /// `start_cs * 10 >= keep`; a transcript item at `ts` when `ts - start >= keep`.
    pub fn meeting_trim(&self, meeting_id: i64) -> rusqlite::Result<Option<(i64, i64)>> {
        Ok(self
            .conn
            .query_row(
                "SELECT start_unix_ms, trimmed_keep_ms FROM meeting WHERE id = ?1",
                params![meeting_id],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?)),
            )
            .optional()?
            .and_then(|(start, keep)| keep.map(|k| (start, k))))
    }

    /// A trimmed meeting's cut on the live clock (ms from its last part's start), where
    /// `insert_live_segments` stops keeping rows; None when it was never trimmed.
    pub(crate) fn live_cut_ms(&self, meeting_id: i64) -> rusqlite::Result<Option<i64>> {
        let Some((start, keep)) = self.meeting_trim(meeting_id)? else {
            return Ok(None);
        };
        let parts = self.meeting_parts(meeting_id)?;
        let origin = parts.first().map_or(start, |p| p.start_unix_ms);
        Ok(Some(keep - live_offset_ms(&parts, origin)))
    }

    /// Whether the meeting was trimmed. A trimmed meeting can't be continued, reopened or
    /// merged into: its peers drop every line past the trim, so new parts would vanish there.
    pub fn is_trimmed(&self, meeting_id: i64) -> rusqlite::Result<bool> {
        Ok(self.meeting_trim(meeting_id)?.is_some())
    }

    /// Deletes the meeting's transcript lines starting at or after `keep_ms`: `transcript_segment`
    /// rows (with their `items` mirrors) and transcript items with no segment twin (a
    /// peer's). Items are timed from the meeting row's start (`mirror_into_items`).
    pub(crate) fn delete_transcript_past_tx(
        tx: &rusqlite::Connection,
        meeting_id: i64,
        keep_ms: i64,
    ) -> rusqlite::Result<()> {
        tx.execute(
            "DELETE FROM items WHERE source_table = 'transcript_segment' AND source_id IN \
             (SELECT id FROM transcript_segment WHERE meeting_id = ?1 AND start_cs * 10 >= ?2)",
            params![meeting_id, keep_ms],
        )?;
        tx.execute(
            "DELETE FROM transcript_segment WHERE meeting_id = ?1 AND start_cs * 10 >= ?2",
            params![meeting_id, keep_ms],
        )?;
        tx.execute(
            "DELETE FROM items WHERE kind = 'transcript' AND thread_id = ?1 \
             AND ts_unix_ms - (SELECT start_unix_ms FROM meeting WHERE id = ?1) >= ?2",
            params![meeting_id, keep_ms],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::NewLiveSegment;
    use crate::{AppliedMeeting, AppliedTranscriptSegment, MeetingTrigger, NewMeeting, NewMeetingSnapshot, NewTranscriptSegment};

    fn count(s: &Store, sql: &str, id: i64) -> i64 {
        s.conn.query_row(sql, [id], |r| r.get(0)).unwrap()
    }

    fn insert(s: &Store, device: &str, status: MeetingStatus) -> i64 {
        // Scheduled start 0; the recording (part 1) starts at 1_000.
        s.insert_meeting(&NewMeeting {
            device_id: device.into(),
            start_unix_ms: 0,
            end_unix_ms: None,
            title: "t".into(),
            attendees: vec![],
            trigger: MeetingTrigger::Calendar,
            audio_path: None,
            status,
        })
        .unwrap()
    }

    /// Own meeting with parts 1_000-50_000 / 61_000-100_000 / 121_000-190_000, one segment
    /// 100 ms into each part, one snapshot 500 ms into each.
    fn three_parts(s: &Store) -> i64 {
        s.set_own_device_id("d");
        let id = insert(s, "d", MeetingStatus::Recording);
        for (n, (start, end)) in [(1_000, 50_000), (61_000, 100_000), (121_000, 190_000)].into_iter().enumerate() {
            let n = n as i64 + 1;
            s.begin_meeting_part(id, start, None).unwrap();
            s.complete_meeting_part(id, n, end, Some(&format!("/a/{n}.caf"))).unwrap();
            let seg = NewTranscriptSegment { meeting_id: id, start_cs: 10, speaker_label: "Me".into(), text: format!("s{n}") };
            s.replace_part_transcript_segments(id, n, &[seg]).unwrap();
            s.insert_meeting_snapshot(&NewMeetingSnapshot {
                meeting_id: id,
                captured_unix_ms: start + 500,
                file_path: format!("/s/{n}.png"),
                width: 1,
                height: 1,
                ocr_text: format!("slide{n}"),
            })
            .unwrap();
        }
        s.complete_meeting(id, 190_000, None).unwrap();
        id
    }

    fn texts(s: &Store, id: i64) -> Vec<String> {
        s.conn
            .prepare("SELECT text FROM transcript_segment WHERE meeting_id = ?1 ORDER BY start_cs")
            .unwrap()
            .query_map([id], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    #[test]
    fn trim_cuts_spanning_part_and_drops_later_parts() {
        let s = Store::open_in_memory().unwrap();
        let id = three_parts(&s);
        assert_eq!(s.meeting_length_ms(id).unwrap(), 189_000, "measured from part 1, not the scheduled start");

        // Cut at 1_000 + 80_000 = 81_000, 20 s into part 2.
        let r = s.trim_meeting(id, 80_000).unwrap().unwrap();
        assert_eq!(r.removed_parts, [3]);
        assert_eq!(r.spanning_part, Some((2, 20_000)));
        assert_eq!(r.removed_snapshot_paths, ["/s/3.png"]);
        let parts: Vec<_> = s.meeting_parts(id).unwrap().iter().map(|p| (p.part_index, p.end_unix_ms)).collect();
        assert_eq!(parts, [(1, Some(50_000)), (2, Some(81_000))], "part 1 whole, part 2 ends at the cut");
        assert_eq!(texts(&s, id), ["s1", "s2"]);
        assert_eq!(
            count(&s, "SELECT COUNT(*) FROM items WHERE kind = 'transcript' AND thread_id = ?1", id),
            2,
            "the removed segment's search mirror goes too"
        );

        // A cut in the gap between parts 1 and 2 spans nothing.
        let s = Store::open_in_memory().unwrap();
        let id = three_parts(&s);
        let r = s.trim_meeting(id, 55_000).unwrap().unwrap();
        assert_eq!((r.removed_parts, r.spanning_part), (vec![2, 3], None));
    }

    #[test]
    fn trim_noop_cases() {
        let s = Store::open_in_memory().unwrap();
        let id = three_parts(&s);
        for keep in [189_000, 500_000, 0, -5] {
            assert_eq!(s.trim_meeting(id, keep).unwrap(), None, "keep {keep}");
        }
        assert_eq!(s.trim_meeting(9_999, 1_000).unwrap(), None, "missing meeting");

        let recording = insert(&s, "d", MeetingStatus::Recording);
        s.begin_meeting_part(recording, 1_000, None).unwrap();
        assert_eq!(s.trim_meeting(recording, 1).unwrap(), None, "still recording");
        s.complete_meeting(recording, 60_000, None).unwrap();
        s.reopen_meeting(recording).unwrap();
        assert_eq!(s.trim_meeting(recording, 1).unwrap(), None, "reopened keeps its end but is recording");

        let theirs = insert(&s, "peer", MeetingStatus::Recording);
        s.conn.execute("UPDATE meeting SET status = 'completed', end_unix_ms = 60000 WHERE id = ?1", [theirs]).unwrap();
        assert_eq!(s.trim_meeting(theirs, 1_000).unwrap(), None, "another device's meeting");

        let unknown = Store::open_in_memory().unwrap();
        let mine = three_parts(&unknown);
        *unknown.own_device_id.borrow_mut() = None;
        assert_eq!(unknown.trim_meeting(mine, 80_000).unwrap(), None, "this device's id unknown");

        assert_eq!(s.get_meeting(id).unwrap().unwrap().end_unix_ms, Some(190_000));
        assert_eq!(texts(&s, id).len(), 3);
        assert_eq!(s.meeting_parts(id).unwrap().len(), 3);
        assert_eq!(s.meeting_trim(id).unwrap(), None);
    }

    #[test]
    fn trim_sets_end_and_marker_and_reenqueues() {
        let s = Store::open_in_memory().unwrap();
        let id = three_parts(&s);
        let before = s.list_outbox_operations().unwrap().len();
        s.trim_meeting(id, 80_000).unwrap().unwrap();
        assert_eq!(s.get_meeting(id).unwrap().unwrap().end_unix_ms, Some(81_000));
        assert_eq!(s.meeting_trim(id).unwrap(), Some((0, 80_000)));

        let ops = s.list_outbox_operations().unwrap();
        assert_eq!(ops.len(), before + 1);
        let (_, row_type, _, payload) = ops.last().unwrap();
        assert_eq!(row_type, "meeting");
        let pushed: AppliedMeeting = serde_json::from_str(payload).unwrap();
        assert_eq!((pushed.end_unix_ms, pushed.trimmed_keep_ms), (Some(81_000), Some(80_000)));
        let pushed_legacy = s.unpushed_meeting_rows("d", 0).unwrap();
        assert!(matches!(&pushed_legacy[0], crate::SyncableRow::Meeting { meeting, .. } if meeting.trimmed_keep_ms == Some(80_000)));

        // A peer holding legacy segment rows drops those past the trim when the row arrives,
        // and an older payload without the marker never un-trims it.
        let peer = Store::open_in_memory().unwrap();
        let untrimmed = AppliedMeeting { trimmed_keep_ms: None, end_unix_ms: Some(190_000), ..pushed.clone() };
        let local = peer.apply_synced_meeting("d", 1, None, &untrimmed).unwrap();
        for (seq, cs) in [(2, 10), (3, 12_010)] {
            let seg = AppliedTranscriptSegment { start_cs: cs, speaker_label: "Me".into(), text: format!("at {cs}") };
            peer.apply_synced_transcript_segment(seq, local, &seg).unwrap();
        }
        peer.apply_synced_meeting("d", 4, Some(local), &pushed).unwrap();
        assert_eq!(texts(&peer, local), ["at 10"]);
        assert_eq!(count(&peer, "SELECT COUNT(*) FROM items WHERE kind = 'transcript' AND thread_id = ?1", local), 1);
        peer.apply_synced_meeting("d", 5, Some(local), &untrimmed).unwrap();
        assert_eq!(peer.meeting_trim(local).unwrap(), Some((0, 80_000)));
    }

    #[test]
    fn trim_removes_live_segments_and_later_snapshots() {
        let s = Store::open_in_memory().unwrap();
        let id = three_parts(&s);
        // Live rows are timed from the part being recorded: part 3, 120 s into the meeting.
        let live = |start_cs, text: &str| NewLiveSegment {
            meeting_id: id,
            channel: "mic".into(),
            chunk_index: 0,
            start_cs,
            end_cs: start_cs + 100,
            speaker_label: "Me".into(),
            text: text.into(),
        };
        s.insert_live_segments(&[live(100, "early"), live(900, "late")]).unwrap();
        s.insert_meeting_snapshot(&NewMeetingSnapshot {
            meeting_id: id,
            captured_unix_ms: 130_000,
            file_path: "/s/late.png".into(),
            width: 1,
            height: 1,
            ocr_text: "late slide".into(),
        })
        .unwrap();
        // Cut at 126_000: 5 s into part 3. "early" is at 121 s, "late" at 129 s.
        let r = s.trim_meeting(id, 125_000).unwrap().unwrap();
        assert_eq!((r.spanning_part, r.removed_snapshot_paths), (Some((3, 5_000)), vec!["/s/late.png".to_string()]));
        let kept: Vec<String> = s.list_live_segments(id).unwrap().into_iter().map(|l| l.text).collect();
        assert_eq!(kept, ["early"]);
        let snaps: Vec<String> = s.list_meeting_snapshots(id).unwrap().into_iter().map(|x| x.file_path).collect();
        assert_eq!(snaps, ["/s/1.png", "/s/2.png", "/s/3.png"]);
        assert_eq!(count(&s, "SELECT COUNT(*) FROM items WHERE kind = 'snapshot' AND thread_id = ?1", id), 3);
    }

    #[test]
    fn trim_moves_meeting_paths_off_removed_parts() {
        let s = Store::open_in_memory().unwrap();
        let id = three_parts(&s);
        s.conn
            .execute_batch(&format!(
                "UPDATE meeting_part SET transcript_path = '/t/' || part_index || '.txt' WHERE meeting_id = {id};
                 UPDATE meeting SET audio_path = '/a/3.caf', transcript_path = '/t/3.txt' WHERE id = {id};"
            ))
            .unwrap();
        s.trim_meeting(id, 80_000).unwrap().unwrap();
        let m = s.get_meeting(id).unwrap().unwrap();
        assert_eq!((m.audio_path.as_deref(), m.transcript_path.as_deref()), (Some("/a/2.caf"), Some("/t/2.txt")));

        // The last kept part has no audio: the row's path is cleared, not left on a removed file.
        let s = Store::open_in_memory().unwrap();
        let id = three_parts(&s);
        s.conn
            .execute_batch(&format!(
                "UPDATE meeting_part SET audio_path = NULL WHERE meeting_id = {id} AND part_index = 2;
                 UPDATE meeting SET audio_path = '/a/3.caf' WHERE id = {id};"
            ))
            .unwrap();
        s.trim_meeting(id, 80_000).unwrap().unwrap();
        assert_eq!(s.get_meeting(id).unwrap().unwrap().audio_path, None);

        // A path that isn't a removed part's is kept.
        let s = Store::open_in_memory().unwrap();
        let id = three_parts(&s);
        s.conn.execute("UPDATE meeting SET audio_path = '/a/1.caf' WHERE id = ?1", [id]).unwrap();
        s.trim_meeting(id, 80_000).unwrap().unwrap();
        assert_eq!(s.get_meeting(id).unwrap().unwrap().audio_path.as_deref(), Some("/a/1.caf"));
    }

    #[test]
    fn trimmed_meeting_drops_reinserted_lines_past_the_cut() {
        let s = Store::open_in_memory().unwrap();
        let id = three_parts(&s);
        s.trim_meeting(id, 80_000).unwrap().unwrap(); // cut 20 s into part 2
        let seg = |start_cs, text: &str| NewTranscriptSegment { meeting_id: id, start_cs, speaker_label: "Me".into(), text: text.into() };
        // "Transcribe again" on part 2's audio (as if its truncation had failed): part-relative times.
        s.replace_part_transcript_segments(id, 2, &[seg(1_000, "kept"), seg(2_000, "at cut"), seg(3_000, "past")]).unwrap();
        // Meeting-clock insert: 85 s is past the cut.
        s.insert_transcript_segments(&[seg(8_500, "late")]).unwrap();
        assert_eq!(texts(&s, id), ["s1", "kept"]);

        // Live rows are timed from part 2 (60 s in): the cut is 20 s into it.
        let live = |start_cs, text: &str| NewLiveSegment {
            meeting_id: id,
            channel: "mic".into(),
            chunk_index: 0,
            start_cs,
            end_cs: start_cs + 100,
            speaker_label: "Me".into(),
            text: text.into(),
        };
        s.insert_live_segments(&[live(1_900, "live kept"), live(2_000, "live at cut")]).unwrap();
        let kept: Vec<String> = s.list_live_segments(id).unwrap().into_iter().map(|l| l.text).collect();
        assert_eq!(kept, ["live kept"]);
    }

    #[test]
    fn trim_removes_everything_starting_exactly_at_the_cut() {
        let s = Store::open_in_memory().unwrap();
        let id = three_parts(&s);
        // keep 60_000 puts the cut at 61_000, exactly where part 2 starts.
        s.insert_transcript_segments(&[NewTranscriptSegment { meeting_id: id, start_cs: 6_000, speaker_label: "Me".into(), text: "edge".into() }])
            .unwrap();
        s.insert_meeting_snapshot(&NewMeetingSnapshot {
            meeting_id: id,
            captured_unix_ms: 61_000,
            file_path: "/s/edge.png".into(),
            width: 1,
            height: 1,
            ocr_text: "edge slide".into(),
        })
        .unwrap();
        let live = NewLiveSegment { meeting_id: id, channel: "mic".into(), chunk_index: 0, start_cs: 0, end_cs: 0, speaker_label: "Me".into(), text: "x".into() };
        s.insert_live_segments(&[live]).unwrap();
        let r = s.trim_meeting(id, 60_000).unwrap().unwrap();
        assert_eq!((r.removed_parts, r.spanning_part), (vec![2, 3], None));
        assert!(r.removed_snapshot_paths.contains(&"/s/edge.png".to_string()));
        assert_eq!(texts(&s, id), ["s1"], "the segment at exactly keep goes");
        assert!(s.list_live_segments(id).unwrap().is_empty());
        assert_eq!(s.meeting_trim(id).unwrap(), Some((0, 60_000)));
    }

    #[test]
    fn trimmed_meeting_cannot_be_continued() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("d");
        let id = insert(&s, "d", MeetingStatus::Recording);
        s.begin_meeting_part(id, 1_000, Some("zoom:1")).unwrap();
        s.complete_meeting(id, 60_000, None).unwrap();
        assert_eq!(s.find_continuable_meeting("zoom:1", 61_000).unwrap(), Some(id));
        assert!(!s.is_trimmed(id).unwrap());

        s.trim_meeting(id, 30_000).unwrap().unwrap();
        assert!(s.is_trimmed(id).unwrap());
        assert_eq!(s.find_continuable_meeting("zoom:1", 32_000).unwrap(), None, "a rejoin starts a new meeting");
        assert!(s.reopen_meeting(id).is_err());
        assert!(s.begin_meeting_part(id, 40_000, None).is_err());
        assert_eq!(s.get_meeting(id).unwrap().unwrap().status, MeetingStatus::Completed);
        assert_eq!(s.meeting_parts(id).unwrap().len(), 1);

        // Nor merged into: the same-titled meeting right after it isn't offered the merge.
        let next = s
            .insert_meeting(&NewMeeting {
                device_id: "d".into(),
                start_unix_ms: 40_000,
                end_unix_ms: Some(50_000),
                title: "t".into(),
                attendees: vec![],
                trigger: MeetingTrigger::Calendar,
                audio_path: None,
                status: MeetingStatus::Completed,
            })
            .unwrap();
        assert_eq!(s.previous_mergeable_meeting(next, "d").unwrap(), None);
        assert!(s.merge_into_previous(id, next).is_err());
    }

    #[test]
    fn trim_partless_meeting_creates_part_and_reports_spanning() {
        // Recorded before meeting_part existed: the row holds the audio and transcript paths.
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("d");
        let id = insert(&s, "d", MeetingStatus::Completed);
        s.conn
            .execute(
                "UPDATE meeting SET end_unix_ms = 60000, audio_path = '/a/old.caf', transcript_path = '/t/old.txt' WHERE id = ?1",
                [id],
            )
            .unwrap();
        let seg = |start_cs, text: &str| NewTranscriptSegment { meeting_id: id, start_cs, speaker_label: "Me".into(), text: text.into() };
        s.insert_transcript_segments(&[seg(1_000, "kept"), seg(4_000, "past")]).unwrap();
        assert_eq!(count(&s, "SELECT COUNT(*) FROM meeting_part WHERE meeting_id = ?1", id), 0);

        let r = s.trim_meeting(id, 30_000).unwrap().unwrap();
        assert_eq!((r.removed_parts, r.spanning_part), (vec![], Some((1, 30_000))));
        let parts = s.meeting_parts(id).unwrap();
        assert_eq!(parts.len(), 1);
        let p = &parts[0];
        assert_eq!((p.part_index, p.start_unix_ms, p.end_unix_ms), (1, 0, Some(30_000)), "part 1 is the row's span, cut");
        assert_eq!((p.audio_path.as_deref(), p.transcript_path.as_deref()), (Some("/a/old.caf"), Some("/t/old.txt")));
        assert_eq!(count(&s, "SELECT transcribed FROM meeting_part WHERE meeting_id = ?1", id), 1, "it was transcribed");
        assert_eq!(texts(&s, id), ["kept"]);
    }

    #[test]
    fn trimmed_meeting_cannot_be_merged_either_way() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("d");
        let meeting = |start, end| {
            s.insert_meeting(&NewMeeting {
                device_id: "d".into(),
                start_unix_ms: start,
                end_unix_ms: Some(end),
                title: "t".into(),
                attendees: vec![],
                trigger: MeetingTrigger::Calendar,
                audio_path: None,
                status: MeetingStatus::Completed,
            })
            .unwrap()
        };
        let first = meeting(0, 60_000);
        let second = meeting(61_000, 120_000);
        assert_eq!(s.previous_mergeable_meeting(second, "d").unwrap(), Some(first));

        // The later one trimmed: it can't be merged into the one before it either.
        s.trim_meeting(second, 30_000).unwrap().unwrap();
        assert_eq!(s.previous_mergeable_meeting(second, "d").unwrap(), None);
        assert!(s.merge_into_previous(first, second).is_err());
        assert!(s.get_meeting(second).unwrap().is_some(), "nothing merged");
    }
}
