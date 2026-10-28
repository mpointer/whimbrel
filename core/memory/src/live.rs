//! Local-only provisional transcript rows written during a recording. Never mirrored
//! into `items`, never enqueued to the sync outbox, never backfilled; replaced by the
//! real `transcript_segment` rows when the post-call pass runs. See
//! docs/superpowers/specs/2026-10-06-live-transcription-design.md.

use rusqlite::{params, Connection};
use serde::Serialize;

use crate::{MeetingTranscriptSegment, Store};

const STALE_AFTER_MS: i64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone)]
pub struct NewLiveSegment {
    pub meeting_id: i64,
    pub channel: String,
    pub chunk_index: i64,
    pub start_cs: i64,
    pub end_cs: i64,
    pub speaker_label: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LiveSegment {
    pub id: i64,
    pub meeting_id: i64,
    pub channel: String,
    pub chunk_index: i64,
    pub start_cs: i64,
    pub end_cs: i64,
    pub speaker_label: String,
    pub text: String,
}

pub(crate) fn ensure_live_table(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS live_transcript_segment (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            meeting_id INTEGER NOT NULL,
            channel TEXT NOT NULL,
            chunk_index INTEGER NOT NULL,
            start_cs INTEGER NOT NULL,
            end_cs INTEGER NOT NULL,
            speaker_label TEXT NOT NULL,
            text TEXT NOT NULL,
            created_at_unix_ms INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS live_transcript_segment_meeting
            ON live_transcript_segment (meeting_id, start_cs);",
    )
}

impl Store {
    pub fn insert_live_segments(&self, segments: &[NewLiveSegment]) -> rusqlite::Result<()> {
        let now = crate::now_unix_ms();
        let tx = self.conn.unchecked_transaction()?;
        for s in segments {
            tx.execute(
                "INSERT INTO live_transcript_segment
                    (meeting_id, channel, chunk_index, start_cs, end_cs, speaker_label, text, created_at_unix_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![s.meeting_id, s.channel, s.chunk_index, s.start_cs, s.end_cs, s.speaker_label, s.text, now],
            )?;
        }
        tx.commit()
    }

    pub fn list_live_segments(&self, meeting_id: i64) -> rusqlite::Result<Vec<LiveSegment>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, meeting_id, channel, chunk_index, start_cs, end_cs, speaker_label, text
             FROM live_transcript_segment WHERE meeting_id = ?1 ORDER BY start_cs ASC, id ASC",
        )?;
        let rows = stmt.query_map(params![meeting_id], |r| {
            Ok(LiveSegment {
                id: r.get(0)?,
                meeting_id: r.get(1)?,
                channel: r.get(2)?,
                chunk_index: r.get(3)?,
                start_cs: r.get(4)?,
                end_cs: r.get(5)?,
                speaker_label: r.get(6)?,
                text: r.get(7)?,
            })
        })?;
        rows.collect()
    }

    pub fn clear_live_segments(&self, meeting_id: i64) -> rusqlite::Result<usize> {
        self.conn
            .execute("DELETE FROM live_transcript_segment WHERE meeting_id = ?1", params![meeting_id])
    }

    /// Deletes live rows that can no longer be useful: older than a day, belonging to a
    /// meeting that no longer exists, or belonging to a completed/cancelled meeting that
    /// already has its real transcript rows. A recording meeting's rows stay.
    pub fn purge_stale_live_segments(&self, now_unix_ms: i64) -> rusqlite::Result<usize> {
        self.conn.execute(
            "DELETE FROM live_transcript_segment WHERE
                created_at_unix_ms < ?1
                OR meeting_id NOT IN (SELECT id FROM meeting)
                OR (meeting_id IN (SELECT id FROM meeting WHERE status IN ('completed', 'cancelled'))
                    AND EXISTS (SELECT 1 FROM transcript_segment t WHERE t.meeting_id = live_transcript_segment.meeting_id))",
            params![now_unix_ms - STALE_AFTER_MS],
        )
    }

    /// The transcript to read for a meeting: the real rows when any exist, otherwise the
    /// provisional live rows with negative ids (`-rowid`), so a citation to a live row can
    /// never collide with a real segment id.
    pub fn transcript_for_meeting(&self, meeting_id: i64) -> rusqlite::Result<Vec<MeetingTranscriptSegment>> {
        let (real, _total) = self.get_transcript_segments_page(meeting_id, 0, usize::MAX >> 1)?;
        if !real.is_empty() {
            return Ok(real);
        }
        let title: String = self
            .conn
            .query_row("SELECT title FROM meeting WHERE id = ?1", params![meeting_id], |r| r.get(0))
            .unwrap_or_default();
        Ok(self
            .list_live_segments(meeting_id)?
            .into_iter()
            .map(|s| MeetingTranscriptSegment {
                id: -s.id,
                meeting_id: s.meeting_id,
                meeting_title: title.clone(),
                start_cs: s.start_cs,
                speaker_label: s.speaker_label,
                text: s.text,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MeetingStatus, MeetingTrigger, NewMeeting, NewTranscriptSegment, Store};

    fn meeting(store: &Store, status: MeetingStatus) -> i64 {
        store
            .insert_meeting(&NewMeeting {
                device_id: "d".into(),
                start_unix_ms: 1_000,
                end_unix_ms: None,
                title: "Acme sync".into(),
                attendees: vec![],
                trigger: MeetingTrigger::Calendar,
                audio_path: None,
                status,
            })
            .unwrap()
    }

    fn live(meeting_id: i64, chunk: i64, start: i64, who: &str, text: &str) -> NewLiveSegment {
        NewLiveSegment {
            meeting_id,
            channel: if who == "Me" { "mic".into() } else { "system".into() },
            chunk_index: chunk,
            start_cs: start,
            end_cs: start + 100,
            speaker_label: who.into(),
            text: text.into(),
        }
    }

    #[test]
    fn live_rows_round_trip_in_time_order_and_clear() {
        let store = Store::open_in_memory().unwrap();
        let id = meeting(&store, MeetingStatus::Recording);
        store
            .insert_live_segments(&[live(id, 1, 500, "Others", "second"), live(id, 0, 100, "Me", "first")])
            .unwrap();
        let rows = store.list_live_segments(id).unwrap();
        assert_eq!(rows.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(), ["first", "second"]);
        assert!(rows.iter().all(|r| r.id > 0));
        assert_eq!(store.clear_live_segments(id).unwrap(), 2);
        assert!(store.list_live_segments(id).unwrap().is_empty());
    }

    #[test]
    fn live_rows_never_touch_the_transcript_tables_items_or_the_outbox() {
        let store = Store::open_in_memory().unwrap();
        let id = meeting(&store, MeetingStatus::Recording);
        let count = |sql: &str| -> i64 { store.conn.query_row(sql, [], |r| r.get(0)).unwrap() };
        let before = (
            count("SELECT COUNT(*) FROM transcript_segment"),
            count("SELECT COUNT(*) FROM items"),
            count("SELECT COUNT(*) FROM sync_outbox"),
        );
        store.insert_live_segments(&[live(id, 0, 0, "Me", "hello there")]).unwrap();
        let after = (
            count("SELECT COUNT(*) FROM transcript_segment"),
            count("SELECT COUNT(*) FROM items"),
            count("SELECT COUNT(*) FROM sync_outbox"),
        );
        assert_eq!(before, after);
    }

    #[test]
    fn transcript_for_meeting_uses_negative_ids_for_live_rows_and_real_rows_win() {
        let store = Store::open_in_memory().unwrap();
        let id = meeting(&store, MeetingStatus::Recording);
        store.insert_live_segments(&[live(id, 0, 100, "Me", "live words")]).unwrap();
        let rows = store.transcript_for_meeting(id).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].id < 0, "live rows must have negative ids, got {}", rows[0].id);
        assert_eq!((rows[0].meeting_title.as_str(), rows[0].speaker_label.as_str()), ("Acme sync", "Me"));

        store
            .insert_transcript_segments(&[NewTranscriptSegment {
                meeting_id: id,
                start_cs: 100,
                speaker_label: "Me".into(),
                text: "final words".into(),
            }])
            .unwrap();
        let rows = store.transcript_for_meeting(id).unwrap();
        assert_eq!(rows.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(), ["final words"]);
        assert!(rows[0].id > 0);
    }

    #[test]
    fn stale_live_rows_are_purged_but_a_recording_meetings_rows_stay() {
        let store = Store::open_in_memory().unwrap();
        let recording = meeting(&store, MeetingStatus::Recording);
        let finished = meeting(&store, MeetingStatus::Completed);
        store.insert_live_segments(&[live(recording, 0, 0, "Me", "keep")]).unwrap();
        store.insert_live_segments(&[live(finished, 0, 0, "Me", "stale")]).unwrap();
        store
            .insert_transcript_segments(&[NewTranscriptSegment {
                meeting_id: finished,
                start_cs: 0,
                speaker_label: "Me".into(),
                text: "final".into(),
            }])
            .unwrap();
        let now = crate::now_unix_ms();
        assert_eq!(store.purge_stale_live_segments(now).unwrap(), 1);
        assert_eq!(store.list_live_segments(recording).unwrap().len(), 1);
        assert!(store.list_live_segments(finished).unwrap().is_empty());
    }

    #[test]
    fn a_completed_meeting_without_final_rows_keeps_its_live_rows_for_a_day() {
        let store = Store::open_in_memory().unwrap();
        let finished = meeting(&store, MeetingStatus::Completed);
        store.insert_live_segments(&[live(finished, 0, 0, "Me", "only copy")]).unwrap();
        assert_eq!(store.purge_stale_live_segments(crate::now_unix_ms()).unwrap(), 0);
        let day_later = crate::now_unix_ms() + 25 * 60 * 60 * 1000;
        assert_eq!(store.purge_stale_live_segments(day_later).unwrap(), 1);
    }

    #[test]
    fn rows_older_than_a_day_are_purged_even_for_a_recording_meeting() {
        let store = Store::open_in_memory().unwrap();
        let id = meeting(&store, MeetingStatus::Recording);
        store.insert_live_segments(&[live(id, 0, 0, "Me", "old")]).unwrap();
        let day_later = crate::now_unix_ms() + 25 * 60 * 60 * 1000;
        assert_eq!(store.purge_stale_live_segments(day_later).unwrap(), 1);
    }

    #[test]
    fn rows_of_a_deleted_meeting_are_purged() {
        let store = Store::open_in_memory().unwrap();
        store.insert_live_segments(&[live(9_999, 0, 0, "Me", "orphan")]).unwrap();
        assert_eq!(store.purge_stale_live_segments(crate::now_unix_ms()).unwrap(), 1);
    }

    #[test]
    fn forgetting_a_meeting_deletes_its_live_rows_at_once() {
        let store = Store::open_in_memory().unwrap();
        let id = meeting(&store, MeetingStatus::Recording);
        store.insert_live_segments(&[live(id, 0, 0, "Me", "private words")]).unwrap();
        assert!(store.delete_meeting(id).unwrap());
        assert!(store.list_live_segments(id).unwrap().is_empty(), "no transcript text may outlive a forgotten meeting");
    }

    #[test]
    fn retention_deletes_the_live_rows_of_expired_meetings() {
        let store = Store::open_in_memory().unwrap();
        let id = meeting(&store, MeetingStatus::Scheduled); // starts at 1_000
        store.insert_live_segments(&[live(id, 0, 0, "Me", "old words")]).unwrap();
        let (deleted, _files) = store.delete_meetings_before(2_000).unwrap();
        assert_eq!(deleted, 1);
        assert!(store.list_live_segments(id).unwrap().is_empty());
    }

    #[test]
    fn a_very_large_now_deletes_every_live_row_which_is_how_turning_the_setting_off_clears_them() {
        let store = Store::open_in_memory().unwrap();
        let recording = meeting(&store, MeetingStatus::Recording);
        let finished = meeting(&store, MeetingStatus::Completed);
        store.insert_live_segments(&[live(recording, 0, 0, "Me", "a"), live(finished, 0, 0, "Me", "b")]).unwrap();
        assert_eq!(store.purge_stale_live_segments(i64::MAX / 2).unwrap(), 2);
        assert!(store.list_live_segments(recording).unwrap().is_empty());
        assert!(store.list_live_segments(finished).unwrap().is_empty());
    }
}
