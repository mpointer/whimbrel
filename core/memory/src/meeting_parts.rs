//! Meeting parts: one recording session of a meeting. A recording that restarts within
//! `CONTINUE_WINDOW_MS` of a completed meeting with the same `continue_key` reopens that
//! meeting and adds a part. Local-only: neither the table nor `meeting.continue_key` syncs.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::{MeetingStatus, NewTranscriptSegment, Store};

/// How long after a meeting ends a new recording with the same key still continues it.
pub const CONTINUE_WINDOW_MS: i64 = 600_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SplitResult {
    pub new_meeting_id: i64,
    /// Snapshot files now owned by the new meeting; the caller moves them on disk and then
    /// calls [`Store::set_meeting_snapshot_paths`].
    pub moved_snapshot_paths: Vec<String>,
}

fn bad(msg: &str) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(msg.into())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeetingPart {
    pub part_index: i64,
    pub start_unix_ms: i64,
    pub end_unix_ms: Option<i64>,
    pub audio_path: Option<String>,
    pub transcript_path: Option<String>,
    pub transcribed: bool,
}

pub(crate) fn ensure_meeting_part_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS meeting_part (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            meeting_id INTEGER NOT NULL,
            part_index INTEGER NOT NULL,
            start_unix_ms INTEGER NOT NULL,
            end_unix_ms INTEGER,
            audio_path TEXT,
            transcript_path TEXT,
            transcribed INTEGER NOT NULL DEFAULT 0,
            UNIQUE(meeting_id, part_index)
        );",
    )?;
    let has_key = conn
        .prepare("PRAGMA table_info(meeting)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .iter()
        .any(|c| c == "continue_key");
    if !has_key {
        conn.execute("ALTER TABLE meeting ADD COLUMN continue_key TEXT", [])?;
    }
    Ok(())
}

impl Store {
    /// Adds the next part (1-based) and returns its index. The first part also sets the
    /// meeting's `continue_key`.
    pub fn begin_meeting_part(
        &self,
        meeting_id: i64,
        start_unix_ms: i64,
        continue_key: Option<&str>,
    ) -> rusqlite::Result<i64> {
        let tx = self.conn.unchecked_transaction()?;
        let index: i64 = tx.query_row(
            "SELECT COALESCE(MAX(part_index), 0) + 1 FROM meeting_part WHERE meeting_id = ?1",
            params![meeting_id],
            |r| r.get(0),
        )?;
        tx.execute(
            "INSERT INTO meeting_part (meeting_id, part_index, start_unix_ms) VALUES (?1, ?2, ?3)",
            params![meeting_id, index, start_unix_ms],
        )?;
        if index == 1 {
            if let Some(key) = continue_key.filter(|k| !k.is_empty()) {
                tx.execute(
                    "UPDATE meeting SET continue_key = ?1 WHERE id = ?2",
                    params![key, meeting_id],
                )?;
            }
        }
        tx.commit()?;
        Ok(index)
    }

    /// The completed meeting with `continue_key` that ended within the window, latest end wins.
    pub fn find_continuable_meeting(
        &self,
        continue_key: &str,
        now_unix_ms: i64,
    ) -> rusqlite::Result<Option<i64>> {
        self.find_continuable_meeting_within(continue_key, now_unix_ms, CONTINUE_WINDOW_MS)
    }

    /// [`Store::find_continuable_meeting`] with the caller's window (the app picks a shorter
    /// one for native-app keys without a URL).
    pub fn find_continuable_meeting_within(
        &self,
        continue_key: &str,
        now_unix_ms: i64,
        window_ms: i64,
    ) -> rusqlite::Result<Option<i64>> {
        if continue_key.is_empty() {
            return Ok(None);
        }
        self.conn
            .query_row(
                "SELECT id FROM meeting WHERE continue_key = ?1 AND status = ?2 \
                 AND end_unix_ms >= ?3 ORDER BY end_unix_ms DESC, id DESC LIMIT 1",
                params![
                    continue_key,
                    MeetingStatus::Completed.as_str(),
                    now_unix_ms - window_ms
                ],
                |r| r.get(0),
            )
            .optional()
    }

    /// Back to `Recording`. Deliberately no `local_seq` restamp: the push scan skips
    /// non-terminal rows, and `complete_meeting` restamps when the meeting ends again.
    pub fn reopen_meeting(&self, id: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE meeting SET status = ?1 WHERE id = ?2",
            params![MeetingStatus::Recording.as_str(), id],
        )?;
        Ok(())
    }

    /// Undoes [`Store::reopen_meeting`] plus the part begun after it, for a start that lost
    /// its race: drops that part (never part 1) and puts the meeting back to `Completed`.
    /// `end_unix_ms` is untouched, since reopening never changed it.
    pub fn undo_meeting_reopen(&self, meeting_id: i64, part_index: i64) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM meeting_part WHERE meeting_id = ?1 AND part_index = ?2 AND part_index > 1",
            params![meeting_id, part_index],
        )?;
        tx.execute(
            "UPDATE meeting SET status = ?1 WHERE id = ?2",
            params![MeetingStatus::Completed.as_str(), meeting_id],
        )?;
        tx.commit()
    }

    pub fn complete_meeting_part(
        &self,
        meeting_id: i64,
        part_index: i64,
        end_unix_ms: i64,
        audio_path: Option<&str>,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE meeting_part SET end_unix_ms = ?1, audio_path = ?2 WHERE meeting_id = ?3 AND part_index = ?4",
            params![end_unix_ms, audio_path, meeting_id, part_index],
        )?;
        Ok(())
    }

    /// Writes only `meeting.notes` (nil clears it), restamped for sync like
    /// `update_meeting_transcript_and_notes`. Each part's ending writes its notes here.
    pub fn set_meeting_notes(&self, id: i64, notes: Option<&str>) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let local_seq = Self::next_local_seq_tx(&tx)?;
        tx.execute(
            "UPDATE meeting SET notes = ?1, local_seq = ?2 WHERE id = ?3",
            params![notes, local_seq, id],
        )?;
        self.enqueue_meeting_outbox(&tx, id, local_seq)?;
        tx.commit()
    }

    pub fn set_meeting_part_transcript(
        &self,
        meeting_id: i64,
        part_index: i64,
        transcript_path: &str,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE meeting_part SET transcript_path = ?1, transcribed = 1 WHERE meeting_id = ?2 AND part_index = ?3",
            params![transcript_path, meeting_id, part_index],
        )?;
        Ok(())
    }

    /// Centiseconds from part 1's start to part `part_index`'s start (0 for part 1).
    pub fn part_offset_cs(&self, meeting_id: i64, part_index: i64) -> rusqlite::Result<i64> {
        let parts = self.meeting_parts(meeting_id)?;
        let part = parts
            .iter()
            .find(|p| p.part_index == part_index)
            .ok_or(rusqlite::Error::QueryReturnedNoRows)?;
        Ok((part.start_unix_ms - parts[0].start_unix_ms) / 10)
    }

    /// Replaces this device's segments for one part: stores `segments` (part-relative
    /// `start_cs`) shifted by the part's offset, after deleting the segments, and their
    /// `items` mirrors, already in `[offset, next_part_offset)`. One transaction. Returns
    /// the number inserted; a missing meeting inserts nothing. A meeting authored by
    /// another device is left untouched (`transcript_segment` has no `device_id`, so
    /// meeting ownership is the only available "this device's segments" test).
    pub fn replace_part_transcript_segments(
        &self,
        meeting_id: i64,
        part_index: i64,
        segments: &[NewTranscriptSegment],
    ) -> rusqlite::Result<usize> {
        let owner: Option<String> = self
            .conn
            .query_row(
                "SELECT device_id FROM meeting WHERE id = ?1",
                params![meeting_id],
                |r| r.get(0),
            )
            .optional()?;
        let Some(owner) = owner else { return Ok(0) };
        if self
            .own_device_id
            .borrow()
            .as_deref()
            .is_some_and(|d| d != owner)
        {
            return Ok(0);
        }
        let parts = self.meeting_parts(meeting_id)?;
        let offset = self.part_offset_cs(meeting_id, part_index)?;
        let end = parts
            .iter()
            .find(|p| p.part_index > part_index)
            .map_or(i64::MAX, |p| {
                (p.start_unix_ms - parts[0].start_unix_ms) / 10
            });
        let shifted: Vec<NewTranscriptSegment> = segments
            .iter()
            .map(|s| NewTranscriptSegment {
                meeting_id,
                start_cs: s.start_cs + offset,
                ..s.clone()
            })
            .collect();
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM items WHERE source_table = 'transcript_segment' AND source_id IN \
             (SELECT id FROM transcript_segment WHERE meeting_id = ?1 AND start_cs >= ?2 AND start_cs < ?3)",
            params![meeting_id, offset, end],
        )?;
        tx.execute(
            "DELETE FROM transcript_segment WHERE meeting_id = ?1 AND start_cs >= ?2 AND start_cs < ?3",
            params![meeting_id, offset, end],
        )?;
        self.insert_transcript_segments_tx(&tx, &shifted)?;
        tx.commit()?;
        Ok(shifted.len())
    }

    /// Whole-meeting transcript for the summary: `[HH:mm:ss] Speaker: text` per segment (the
    /// shape Swift writes and rebuilds, wall-clock from part 1's start), in `start_cs`
    /// order, with `— rejoined HH:MM —` before the first line of each part >= 2.
    pub fn meeting_transcript_text(&self, meeting_id: i64) -> rusqlite::Result<String> {
        use chrono::{Local, TimeZone};
        let hms = |ms: i64, f: &str| {
            Local
                .timestamp_millis_opt(ms)
                .single()
                .map(|t| t.format(f).to_string())
                .unwrap_or_default()
        };
        // Clock from part 1's start (the recording start, which the transcript file uses),
        // not the meeting row's start (a calendar meeting's scheduled time).
        let parts = self.meeting_parts(meeting_id)?;
        let Some(start) = parts.first().map(|p| p.start_unix_ms) else {
            return Ok(String::new());
        };
        let mut dividers: Vec<(i64, &MeetingPart)> = parts
            .iter()
            .skip(1)
            .map(|p| ((p.start_unix_ms - parts[0].start_unix_ms) / 10, p))
            .collect();
        let mut stmt = self.conn.prepare(
            "SELECT start_cs, speaker_label, text FROM transcript_segment WHERE meeting_id = ?1 ORDER BY start_cs, id",
        )?;
        let rows = stmt.query_map(params![meeting_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        let mut lines = Vec::new();
        for row in rows {
            let (cs, speaker, text) = row?;
            // Emit the divider of the latest part this segment reaches; a part with no
            // segments gets none.
            let mut due = None;
            while dividers.first().is_some_and(|(off, _)| cs >= *off) {
                due = Some(dividers.remove(0).1);
            }
            if let Some(p) = due {
                lines.push(format!("— rejoined {} —", hms(p.start_unix_ms, "%H:%M")));
            }
            lines.push(format!(
                "[{}] {}: {}",
                hms(start + cs * 10, "%H:%M:%S"),
                speaker,
                text
            ));
        }
        Ok(lines.join("\n"))
    }

    /// Splits a completed meeting at part `from_part_index` (>= 2): that part and later ones,
    /// their segments and snapshots move to a new `Completed` meeting (same title, attendees,
    /// trigger, device; no `continue_key`, as the original loses its key too). Both summaries
    /// are cleared for re-summarizing. One transaction; moved segments and the touched meetings
    /// are restamped and enqueued so sync sees them under the new meeting id. Notes, note
    /// drafts and action items stay on the original. ponytail: peers keep stale copies of
    /// already-pushed moved segments (no tombstone protocol, same as re-transcribe).
    pub fn split_meeting(
        &self,
        meeting_id: i64,
        from_part_index: i64,
    ) -> rusqlite::Result<SplitResult> {
        let meeting = self
            .get_meeting(meeting_id)?
            .ok_or(rusqlite::Error::QueryReturnedNoRows)?;
        if meeting.status == MeetingStatus::Recording {
            return Err(bad("cannot split a meeting that is recording"));
        }
        let parts = self.meeting_parts(meeting_id)?;
        let Some(at) = parts.iter().position(|p| p.part_index == from_part_index) else {
            return Err(bad("no such part"));
        };
        if at == 0 {
            return Err(bad("cannot split before the first part"));
        }
        let (kept, moved) = parts.split_at(at);
        let split_ms = moved[0].start_unix_ms;
        let offset = (split_ms - parts[0].start_unix_ms) / 10;
        let now = crate::now_unix_ms();

        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO meeting (device_id, start_unix_ms, end_unix_ms, title, attendees, trigger, \
             audio_path, transcript_path, status, keep_snapshots, local_seq) \
             SELECT device_id, ?2, end_unix_ms, title, attendees, trigger, ?3, ?4, ?5, keep_snapshots, ?6 \
             FROM meeting WHERE id = ?1",
            params![
                meeting_id,
                split_ms,
                moved[0].audio_path,
                moved[0].transcript_path,
                MeetingStatus::Completed.as_str(),
                Self::next_local_seq_tx(&tx)?,
            ],
        )?;
        let new_id = tx.last_insert_rowid();

        // The original must not keep pointing at a file that now belongs to the new meeting.
        let owned = |f: fn(&MeetingPart) -> &Option<String>, cur: &Option<String>| {
            let moved_away = cur
                .as_ref()
                .is_some_and(|c| moved.iter().any(|p| f(p).as_ref() == Some(c)));
            moved_away.then(|| kept.iter().find_map(|p| f(p).clone()))
        };
        let audio = owned(|p| &p.audio_path, &meeting.audio_path);
        let transcript = owned(|p| &p.transcript_path, &meeting.transcript_path);
        tx.execute(
            "UPDATE meeting SET end_unix_ms = ?2, continue_key = NULL, \
             audio_path = CASE WHEN ?3 THEN ?4 ELSE audio_path END, \
             transcript_path = CASE WHEN ?5 THEN ?6 ELSE transcript_path END WHERE id = ?1",
            params![
                meeting_id,
                kept[kept.len() - 1].end_unix_ms.unwrap_or(split_ms),
                audio.is_some(),
                audio.flatten(),
                transcript.is_some(),
                transcript.flatten(),
            ],
        )?;

        tx.execute(
            "UPDATE meeting_part SET meeting_id = ?3, part_index = part_index - ?4 \
             WHERE meeting_id = ?1 AND part_index >= ?2",
            params![meeting_id, from_part_index, new_id, from_part_index - 1],
        )?;

        let segs = tx
            .prepare("SELECT id, start_cs FROM transcript_segment WHERE meeting_id = ?1 AND start_cs >= ?2")?
            .query_map(params![meeting_id, offset], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (seg_id, cs) in segs {
            let seq = Self::next_local_seq_tx(&tx)?;
            tx.execute(
                "UPDATE transcript_segment SET meeting_id = ?1, start_cs = ?2, local_seq = ?3 WHERE id = ?4",
                params![new_id, cs - offset, seq, seg_id],
            )?;
            let item: Option<i64> = tx
                .query_row(
                    "UPDATE items SET thread_id = ?1, ts_unix_ms = ?2 + ?3 * 10, local_seq = ?4 \
                     WHERE source_table = 'transcript_segment' AND source_id = ?5 RETURNING id",
                    params![new_id, split_ms, cs - offset, seq, seg_id],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(item) = item {
                self.enqueue_transcript_item_outbox(&tx, item, &meeting.device_id, seq, now)?;
            }
        }

        let moved_snapshot_paths = tx
            .prepare(
                "SELECT file_path FROM meeting_snapshot WHERE meeting_id = ?1 AND captured_unix_ms >= ?2 \
                 ORDER BY captured_unix_ms, id",
            )?
            .query_map(params![meeting_id, split_ms], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        tx.execute(
            "UPDATE items SET thread_id = ?1 WHERE source_table = 'meeting_snapshot' AND source_id IN \
             (SELECT id FROM meeting_snapshot WHERE meeting_id = ?2 AND captured_unix_ms >= ?3)",
            params![new_id, meeting_id, split_ms],
        )?;
        tx.execute(
            "UPDATE meeting_snapshot SET meeting_id = ?1 WHERE meeting_id = ?2 AND captured_unix_ms >= ?3",
            params![new_id, meeting_id, split_ms],
        )?;

        for id in [meeting_id, new_id] {
            let seq = Self::next_local_seq_tx(&tx)?;
            tx.execute(
                "UPDATE meeting SET summary_tldr = NULL, summary_decisions_json = NULL, \
                 summary_action_items_json = NULL, summary_open_questions_json = NULL, \
                 summary_provider = NULL, summary_model = NULL, local_seq = ?2 WHERE id = ?1",
                params![id, seq],
            )?;
            self.enqueue_meeting_outbox(&tx, id, seq)?;
        }
        tx.commit()?;
        Ok(SplitResult {
            new_meeting_id: new_id,
            moved_snapshot_paths,
        })
    }

    /// After the caller moved snapshot files on disk: points `meeting_id`'s snapshot rows at
    /// their new paths and records the directory they now live in.
    pub fn set_meeting_snapshot_paths(
        &self,
        meeting_id: i64,
        moves: &[(String, String)],
        snapshots_dir: &str,
    ) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        for (old, new) in moves {
            tx.execute(
                "UPDATE meeting_snapshot SET file_path = ?1 WHERE meeting_id = ?2 AND file_path = ?3",
                params![new, meeting_id, old],
            )?;
        }
        tx.execute(
            "UPDATE meeting SET snapshots_dir = ?2 WHERE id = ?1",
            params![meeting_id, snapshots_dir],
        )?;
        tx.commit()
    }

    /// Parts in order. A legacy meeting (no rows) yields one part built from the meeting row.
    pub fn meeting_parts(&self, meeting_id: i64) -> rusqlite::Result<Vec<MeetingPart>> {
        let mut stmt = self.conn.prepare(
            "SELECT part_index, start_unix_ms, end_unix_ms, audio_path, transcript_path, transcribed \
             FROM meeting_part WHERE meeting_id = ?1 ORDER BY part_index",
        )?;
        let parts = stmt
            .query_map(params![meeting_id], |r| {
                Ok(MeetingPart {
                    part_index: r.get(0)?,
                    start_unix_ms: r.get(1)?,
                    end_unix_ms: r.get(2)?,
                    audio_path: r.get(3)?,
                    transcript_path: r.get(4)?,
                    transcribed: r.get::<_, i64>(5)? != 0,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if !parts.is_empty() {
            return Ok(parts);
        }
        self.conn
            .query_row(
                "SELECT start_unix_ms, end_unix_ms, audio_path, transcript_path FROM meeting WHERE id = ?1",
                params![meeting_id],
                |r| {
                    let transcript_path: Option<String> = r.get(3)?;
                    Ok(MeetingPart {
                        part_index: 1,
                        start_unix_ms: r.get(0)?,
                        end_unix_ms: r.get(1)?,
                        audio_path: r.get(2)?,
                        transcribed: transcript_path.is_some(),
                        transcript_path,
                    })
                },
            )
            .optional()
            .map(|p| p.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MeetingTrigger, NewMeeting};

    const MIN: i64 = 60_000;

    fn mk(store: &Store, status: MeetingStatus, audio: Option<&str>) -> i64 {
        store
            .insert_meeting(&NewMeeting {
                device_id: "d".into(),
                start_unix_ms: 1_000,
                end_unix_ms: None,
                title: "t".into(),
                attendees: vec![],
                trigger: MeetingTrigger::Calendar,
                audio_path: audio.map(str::to_string),
                status,
            })
            .unwrap()
    }

    fn key_of(store: &Store, id: i64) -> Option<String> {
        store
            .conn
            .query_row(
                "SELECT continue_key FROM meeting WHERE id = ?1",
                [id],
                |r| r.get(0),
            )
            .unwrap()
    }

    /// A completed meeting with `key` that ended at `end`.
    fn done(store: &Store, key: Option<&str>, end: i64) -> i64 {
        let id = mk(store, MeetingStatus::Recording, None);
        store.begin_meeting_part(id, 1_000, key).unwrap();
        store.complete_meeting(id, end, None).unwrap();
        id
    }

    #[test]
    fn set_meeting_notes_keeps_transcript_path_and_restamps() {
        let s = Store::open_in_memory().unwrap();
        let id = done(&s, None, 5_000);
        s.update_meeting_transcript_and_notes(id, "/r/t.txt", Some("a")).unwrap();
        let seq = |s: &Store| -> i64 {
            s.conn
                .query_row("SELECT local_seq FROM meeting WHERE id = ?1", [id], |r| r.get(0))
                .unwrap()
        };
        let before = seq(&s);
        s.set_meeting_notes(id, Some("a, b")).unwrap();
        let m = s.get_meeting(id).unwrap().unwrap();
        assert_eq!(m.notes.as_deref(), Some("a, b"));
        assert_eq!(m.transcript_path.as_deref(), Some("/r/t.txt"));
        assert!(seq(&s) > before, "notes sync, so the row is restamped");
        s.set_meeting_notes(id, None).unwrap();
        assert_eq!(s.get_meeting(id).unwrap().unwrap().notes, None);
    }

    #[test]
    fn undo_reopen_drops_the_part_and_restores_completed() {
        let s = Store::open_in_memory().unwrap();
        let id = done(&s, Some("k"), 5_000);
        s.reopen_meeting(id).unwrap();
        let part = s.begin_meeting_part(id, 6_000, None).unwrap();
        assert_eq!(part, 2);
        s.undo_meeting_reopen(id, part).unwrap();
        let m = s.get_meeting(id).unwrap().unwrap();
        assert_eq!(m.status, MeetingStatus::Completed);
        assert_eq!(m.end_unix_ms, Some(5_000));
        assert_eq!(s.meeting_parts(id).unwrap().len(), 1);
        assert_eq!(s.find_continuable_meeting("k", 6_000).unwrap(), Some(id));
        // Never drops part 1.
        s.undo_meeting_reopen(id, 1).unwrap();
        assert_eq!(s.meeting_parts(id).unwrap().len(), 1);
    }

    #[test]
    fn first_part_sets_key_and_index_1() {
        let s = Store::open_in_memory().unwrap();
        let id = mk(&s, MeetingStatus::Recording, None);
        assert_eq!(s.begin_meeting_part(id, 1_000, Some("cal:x")).unwrap(), 1);
        assert_eq!(key_of(&s, id).as_deref(), Some("cal:x"));
        assert_eq!(s.begin_meeting_part(id, 5_000, Some("other")).unwrap(), 2);
        assert_eq!(
            key_of(&s, id).as_deref(),
            Some("cal:x"),
            "later parts keep the key"
        );
    }

    #[test]
    fn continuable_within_window_only() {
        let s = Store::open_in_memory().unwrap();
        let now = 100 * MIN;
        let id = done(&s, Some("cal:x"), now - 9 * MIN);
        assert_eq!(s.find_continuable_meeting("cal:x", now).unwrap(), Some(id));
        let s2 = Store::open_in_memory().unwrap();
        done(&s2, Some("cal:x"), now - 11 * MIN);
        assert_eq!(s2.find_continuable_meeting("cal:x", now).unwrap(), None);
    }

    #[test]
    fn recurring_event_tomorrow_does_not_continue_today() {
        let s = Store::open_in_memory().unwrap();
        let day = 24 * 60 * MIN;
        done(&s, Some("cal:daily"), 10 * day);
        assert_eq!(
            s.find_continuable_meeting("cal:daily", 11 * day).unwrap(),
            None
        );
    }

    #[test]
    fn manual_meeting_without_key_never_continues() {
        let s = Store::open_in_memory().unwrap();
        done(&s, None, 99 * MIN);
        assert_eq!(s.find_continuable_meeting("", 100 * MIN).unwrap(), None);
        assert_eq!(
            s.find_continuable_meeting("cal:x", 100 * MIN).unwrap(),
            None
        );
    }

    #[test]
    fn latest_of_several_matches_wins() {
        let s = Store::open_in_memory().unwrap();
        let now = 100 * MIN;
        done(&s, Some("k"), now - 8 * MIN);
        let latest = done(&s, Some("k"), now - 2 * MIN);
        done(&s, Some("k"), now - 5 * MIN);
        assert_eq!(s.find_continuable_meeting("k", now).unwrap(), Some(latest));
    }

    #[test]
    fn reopen_sets_recording_and_push_scan_skips_it() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("d");
        let id = done(&s, Some("k"), 5_000);
        let count = |s: &Store| {
            s.unpushed_rows("d", 0)
                .unwrap()
                .iter()
                .filter(|r| matches!(r, crate::SyncableRow::Meeting { .. }))
                .count()
        };
        assert_eq!(count(&s), 1);
        let seq_before: i64 = s
            .conn
            .query_row("SELECT local_seq FROM meeting WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .unwrap();
        s.reopen_meeting(id).unwrap();
        assert_eq!(
            s.get_meeting(id).unwrap().unwrap().status,
            MeetingStatus::Recording
        );
        let seq_after: i64 = s
            .conn
            .query_row("SELECT local_seq FROM meeting WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(seq_before, seq_after, "no restamp");
        assert_eq!(count(&s), 0);
    }

    #[test]
    fn legacy_meeting_synthesizes_one_part() {
        let s = Store::open_in_memory().unwrap();
        let id = mk(&s, MeetingStatus::Recording, Some("/r/system-audio-A.caf"));
        s.complete_meeting(id, 9_000, Some("/r/system-audio-A.caf"))
            .unwrap();
        s.update_meeting_transcript_and_notes(id, "/r/t.txt", None)
            .unwrap();
        assert_eq!(
            s.meeting_parts(id).unwrap(),
            vec![MeetingPart {
                part_index: 1,
                start_unix_ms: 1_000,
                end_unix_ms: Some(9_000),
                audio_path: Some("/r/system-audio-A.caf".into()),
                transcript_path: Some("/r/t.txt".into()),
                transcribed: true,
            }]
        );
        assert!(s.meeting_parts(9999).unwrap().is_empty());
    }

    fn two_parts(s: &Store) -> i64 {
        let id = mk(s, MeetingStatus::Recording, None);
        s.begin_meeting_part(id, 1_000, Some("k")).unwrap();
        s.complete_meeting_part(id, 1, 2_000, Some("/r/system-audio-A.caf"))
            .unwrap();
        s.set_meeting_part_transcript(id, 1, "/r/a.txt").unwrap();
        s.complete_meeting(id, 2_000, Some("/r/system-audio-A.caf"))
            .unwrap();
        s.reopen_meeting(id).unwrap();
        s.begin_meeting_part(id, 3_000, None).unwrap();
        s.complete_meeting_part(id, 2, 4_000, Some("/r/system-audio-B.caf"))
            .unwrap();
        s.set_meeting_part_transcript(id, 2, "/r/b.txt").unwrap();
        s.complete_meeting(id, 4_000, Some("/r/system-audio-B.caf"))
            .unwrap();
        id
    }

    #[test]
    fn complete_meeting_keeps_part1_audio_path() {
        let s = Store::open_in_memory().unwrap();
        let id = two_parts(&s);
        assert_eq!(
            s.get_meeting(id).unwrap().unwrap().audio_path.as_deref(),
            Some("/r/system-audio-A.caf")
        );
        let parts = s.meeting_parts(id).unwrap();
        assert_eq!(parts.len(), 2);
        assert!(parts[1].transcribed);
    }

    #[test]
    fn delete_meeting_returns_every_part_file() {
        let s = Store::open_in_memory().unwrap();
        let id = two_parts(&s);
        let f = s.delete_meeting_returning_files(id).unwrap().unwrap();
        assert_eq!(
            f.audio_paths,
            ["/r/system-audio-A.caf", "/r/system-audio-B.caf"]
        );
        assert_eq!(f.transcript_paths, ["/r/a.txt", "/r/b.txt"]);
        assert!(s.meeting_parts(id).unwrap().is_empty());
        assert!(s.delete_meeting_returning_files(id).unwrap().is_none());
        let id2 = two_parts(&s);
        assert!(s.delete_meeting(id2).unwrap());
    }

    #[test]
    fn retention_returns_every_part_file() {
        let s = Store::open_in_memory().unwrap();
        let id = two_parts(&s);
        let (n, files) = s.delete_meetings_before(i64::MAX).unwrap();
        assert_eq!(n, 1);
        assert_eq!(
            files[0].audio_paths,
            ["/r/system-audio-A.caf", "/r/system-audio-B.caf"]
        );
        assert_eq!(files[0].transcript_paths, ["/r/a.txt", "/r/b.txt"]);
        assert_eq!(
            files[0].audio_path.as_deref(),
            Some("/r/system-audio-A.caf")
        );
        let left: i64 = s
            .conn
            .query_row(
                "SELECT COUNT(*) FROM meeting_part WHERE meeting_id=?1",
                [id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(left, 0);
    }

    #[test]
    fn orphan_scan_treats_part_audio_as_owned() {
        let s = Store::open_in_memory().unwrap();
        two_parts(&s);
        let mut paths = s.all_referenced_meeting_audio_paths().unwrap();
        paths.sort();
        assert_eq!(paths, ["/r/system-audio-A.caf", "/r/system-audio-B.caf"]);
    }

    #[test]
    fn find_continuable_within_honours_the_window() {
        let s = Store::open_in_memory().unwrap();
        let now = 100 * MIN;
        let id = done(&s, Some("app:us.zoom.xos|"), now - 4 * MIN);
        assert_eq!(
            s.find_continuable_meeting_within("app:us.zoom.xos|", now, 3 * MIN)
                .unwrap(),
            None
        );
        assert_eq!(
            s.find_continuable_meeting_within("app:us.zoom.xos|", now, 5 * MIN)
                .unwrap(),
            Some(id)
        );
        // The default keeps the 10-minute window.
        assert_eq!(
            s.find_continuable_meeting("app:us.zoom.xos|", now).unwrap(),
            Some(id)
        );
        assert_eq!(
            s.find_continuable_meeting_within("", now, 5 * MIN).unwrap(),
            None
        );
    }

    #[test]
    fn only_completed_meetings_continue() {
        let s = Store::open_in_memory().unwrap();
        let now = 100 * MIN;
        let a = done(&s, Some("k"), now - MIN);
        s.reopen_meeting(a).unwrap();
        let b = done(&s, Some("k"), now - MIN);
        s.update_meeting_status(b, MeetingStatus::Cancelled)
            .unwrap();
        assert_eq!(s.find_continuable_meeting("k", now).unwrap(), None);
    }

    #[test]
    fn complete_meeting_stores_part1_audio_when_meeting_path_null() {
        let s = Store::open_in_memory().unwrap();
        let id = mk(&s, MeetingStatus::Recording, None);
        s.begin_meeting_part(id, 1_000, Some("k")).unwrap();
        s.complete_meeting_part(id, 1, 2_000, Some("/r/system-audio-A.caf"))
            .unwrap();
        s.begin_meeting_part(id, 3_000, None).unwrap();
        s.complete_meeting(id, 4_000, Some("/r/system-audio-B.caf"))
            .unwrap();
        assert_eq!(
            s.get_meeting(id).unwrap().unwrap().audio_path.as_deref(),
            Some("/r/system-audio-A.caf")
        );
    }

    #[test]
    fn empty_key_is_no_key() {
        let s = Store::open_in_memory().unwrap();
        let id = mk(&s, MeetingStatus::Recording, None);
        s.begin_meeting_part(id, 1_000, Some("")).unwrap();
        assert_eq!(key_of(&s, id), None);
        s.complete_meeting(id, 2_000, None).unwrap();
        assert_eq!(s.find_continuable_meeting("", 3_000).unwrap(), None);
    }

    fn seg(meeting_id: i64, cs: i64, text: &str) -> NewTranscriptSegment {
        NewTranscriptSegment {
            meeting_id,
            start_cs: cs,
            speaker_label: "Me".into(),
            text: text.into(),
        }
    }

    fn cs_of(s: &Store, id: i64) -> Vec<(i64, String)> {
        let mut st = s
            .conn
            .prepare("SELECT start_cs, text FROM transcript_segment WHERE meeting_id=?1 ORDER BY start_cs")
            .unwrap();
        st.query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    /// Meeting starting at 1_000 ms; part 2 starts 60 s later (offset 6000 cs).
    fn two_part_meeting(s: &Store) -> i64 {
        let id = mk(s, MeetingStatus::Recording, None);
        s.begin_meeting_part(id, 1_000, Some("k")).unwrap();
        s.begin_meeting_part(id, 61_000, None).unwrap();
        id
    }

    #[test]
    fn part2_segments_are_offset() {
        let s = Store::open_in_memory().unwrap();
        let id = two_part_meeting(&s);
        assert_eq!(s.part_offset_cs(id, 1).unwrap(), 0);
        assert_eq!(s.part_offset_cs(id, 2).unwrap(), 6_000);
        assert_eq!(
            s.replace_part_transcript_segments(id, 2, &[seg(id, 0, "a"), seg(id, 50, "b")])
                .unwrap(),
            2
        );
        assert_eq!(
            cs_of(&s, id),
            [(6_000, "a".to_string()), (6_050, "b".to_string())]
        );
        let ts: i64 = s
            .conn
            .query_row(
                "SELECT ts_unix_ms FROM items WHERE kind='transcript' AND body_text='a'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(ts, 61_000);
    }

    #[test]
    fn replace_is_idempotent_no_duplicates() {
        let s = Store::open_in_memory().unwrap();
        let id = two_part_meeting(&s);
        for _ in 0..2 {
            s.replace_part_transcript_segments(id, 2, &[seg(id, 0, "a")])
                .unwrap();
        }
        assert_eq!(cs_of(&s, id).len(), 1);
        let items: i64 = s
            .conn
            .query_row(
                "SELECT COUNT(*) FROM items WHERE kind='transcript' AND thread_id=?1",
                [id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(items, 1);
    }

    #[test]
    fn replace_part2_leaves_part1_untouched() {
        let s = Store::open_in_memory().unwrap();
        let id = two_part_meeting(&s);
        s.replace_part_transcript_segments(id, 1, &[seg(id, 10, "one")])
            .unwrap();
        s.replace_part_transcript_segments(id, 2, &[seg(id, 10, "two")])
            .unwrap();
        s.replace_part_transcript_segments(id, 2, &[seg(id, 20, "two-b")])
            .unwrap();
        assert_eq!(
            cs_of(&s, id),
            [(10, "one".to_string()), (6_020, "two-b".to_string())]
        );
        // Re-replacing part 1 must not eat part 2.
        s.replace_part_transcript_segments(id, 1, &[seg(id, 11, "one-b")])
            .unwrap();
        assert_eq!(
            cs_of(&s, id),
            [(11, "one-b".to_string()), (6_020, "two-b".to_string())]
        );
    }

    #[test]
    fn transcript_text_has_divider_before_part2() {
        use chrono::{Local, TimeZone};
        let s = Store::open_in_memory().unwrap();
        let id = two_part_meeting(&s);
        s.replace_part_transcript_segments(id, 1, &[seg(id, 0, "hello")])
            .unwrap();
        s.replace_part_transcript_segments(id, 2, &[seg(id, 0, "back")])
            .unwrap();
        let hm = Local
            .timestamp_millis_opt(61_000)
            .unwrap()
            .format("%H:%M")
            .to_string();
        let hms = |ms| {
            Local
                .timestamp_millis_opt(ms)
                .unwrap()
                .format("%H:%M:%S")
                .to_string()
        };
        assert_eq!(
            s.meeting_transcript_text(id).unwrap(),
            format!(
                "[{}] Me: hello\n— rejoined {hm} —\n[{}] Me: back",
                hms(1_000),
                hms(61_000)
            )
        );
    }

    #[test]
    fn transcript_text_clock_starts_at_first_part_not_meeting_start() {
        use chrono::{Local, TimeZone};
        // Calendar meeting scheduled at 1_000 but recording began 5 minutes later.
        let s = Store::open_in_memory().unwrap();
        let id = mk(&s, MeetingStatus::Recording, None);
        s.begin_meeting_part(id, 301_000, Some("k")).unwrap();
        s.replace_part_transcript_segments(id, 1, &[seg(id, 100, "hi")])
            .unwrap();
        let hms = Local
            .timestamp_millis_opt(302_000)
            .unwrap()
            .format("%H:%M:%S")
            .to_string();
        assert_eq!(
            s.meeting_transcript_text(id).unwrap(),
            format!("[{hms}] Me: hi")
        );
    }

    #[test]
    fn legacy_single_part_text_has_no_divider() {
        let s = Store::open_in_memory().unwrap();
        let id = mk(&s, MeetingStatus::Recording, None);
        s.insert_transcript_segments(&[seg(id, 0, "x"), seg(id, 100, "y")])
            .unwrap();
        let t = s.meeting_transcript_text(id).unwrap();
        assert_eq!(t.lines().count(), 2);
        assert!(!t.contains("rejoined"));
    }

    // ---- split ----

    fn snap(meeting_id: i64, at: i64, path: &str) -> crate::NewMeetingSnapshot {
        crate::NewMeetingSnapshot {
            meeting_id,
            captured_unix_ms: at,
            file_path: path.into(),
            width: 1,
            height: 1,
            ocr_text: format!("ocr {path}"),
        }
    }

    /// Meeting starting 1_000 with three completed parts at 1_000 / 61_000 / 121_000
    /// (offsets 0 / 6000 / 12000 cs), one segment per part, one snapshot per part, a summary.
    fn three_parts(s: &Store) -> i64 {
        let id = mk(s, MeetingStatus::Recording, None);
        for (i, (start, end)) in [(1_000, 50_000), (61_000, 100_000), (121_000, 190_000)]
            .into_iter()
            .enumerate()
        {
            let n = i as i64 + 1;
            s.begin_meeting_part(id, start, Some("k")).unwrap();
            s.complete_meeting_part(id, n, end, Some(&format!("/r/audio-{n}.caf")))
                .unwrap();
            s.set_meeting_part_transcript(id, n, &format!("/r/t{n}.txt"))
                .unwrap();
            s.replace_part_transcript_segments(id, n, &[seg(id, 10, &format!("s{n}"))])
                .unwrap();
            s.insert_meeting_snapshot(&snap(id, start + 500, &format!("/snaps/o/{n}.png")))
                .unwrap();
        }
        s.set_meeting_snapshots_dir(id, "/snaps/o").unwrap();
        s.complete_meeting(id, 190_000, None).unwrap();
        s.update_meeting_summary(
            id,
            "tldr",
            &["d".into()],
            &["a".into()],
            &["q".into()],
            "p",
            "m",
        )
        .unwrap();
        id
    }

    fn snapshot_paths(s: &Store, id: i64) -> Vec<String> {
        s.list_meeting_snapshots(id)
            .unwrap()
            .into_iter()
            .map(|x| x.file_path)
            .collect()
    }

    #[test]
    fn split_moves_later_parts_segments_and_snapshots() {
        let s = Store::open_in_memory().unwrap();
        let id = three_parts(&s);
        let r = s.split_meeting(id, 2).unwrap();
        let new = r.new_meeting_id;
        assert_eq!(r.moved_snapshot_paths, ["/snaps/o/2.png", "/snaps/o/3.png"]);

        let (o, n) = (
            s.get_meeting(id).unwrap().unwrap(),
            s.get_meeting(new).unwrap().unwrap(),
        );
        assert_eq!(
            (o.end_unix_ms, o.status),
            (Some(50_000), MeetingStatus::Completed)
        );
        assert_eq!(
            (
                n.start_unix_ms,
                n.end_unix_ms,
                n.status,
                n.title.as_str(),
                n.device_id.as_str()
            ),
            (61_000, Some(190_000), MeetingStatus::Completed, "t", "d")
        );
        assert_eq!(n.trigger, o.trigger);
        assert_eq!(n.audio_path.as_deref(), Some("/r/audio-2.caf"));

        let idx = |id| {
            s.meeting_parts(id)
                .unwrap()
                .iter()
                .map(|p| (p.part_index, p.audio_path.clone().unwrap()))
                .collect::<Vec<_>>()
        };
        assert_eq!(idx(id), [(1, "/r/audio-1.caf".to_string())]);
        assert_eq!(
            idx(new),
            [
                (1, "/r/audio-2.caf".to_string()),
                (2, "/r/audio-3.caf".to_string())
            ]
        );

        assert_eq!(cs_of(&s, id), [(10, "s1".to_string())]);
        assert_eq!(
            cs_of(&s, new),
            [(10, "s2".to_string()), (6_010, "s3".to_string())]
        );
        assert_eq!(snapshot_paths(&s, id), ["/snaps/o/1.png"]);
        assert_eq!(
            snapshot_paths(&s, new),
            ["/snaps/o/2.png", "/snaps/o/3.png"]
        );

        // items mirrors follow their rows
        let items = |kind: &str, id: i64| -> i64 {
            s.conn
                .query_row(
                    "SELECT COUNT(*) FROM items WHERE kind=?1 AND thread_id=?2",
                    params![kind, id],
                    |r| r.get(0),
                )
                .unwrap()
        };
        assert_eq!((items("transcript", id), items("transcript", new)), (1, 2));
        assert_eq!((items("snapshot", id), items("snapshot", new)), (1, 2));

        // summaries cleared on both
        for m in [&o, &n] {
            let m = s.get_meeting(m.id).unwrap().unwrap();
            assert!(m.summary_tldr.is_none() && m.summary_decisions.is_empty());
        }
        assert!(s
            .get_meeting(id)
            .unwrap()
            .unwrap()
            .summary_provider
            .is_none());
    }

    #[test]
    fn split_offsets_restart_at_zero_for_new_meeting() {
        let s = Store::open_in_memory().unwrap();
        let id = three_parts(&s);
        let new = s.split_meeting(id, 2).unwrap().new_meeting_id;
        assert_eq!(s.part_offset_cs(new, 1).unwrap(), 0);
        assert_eq!(s.part_offset_cs(new, 2).unwrap(), 6_000);
        // item ts = new.start + start_cs*10 (61_000 + 10*10)
        let ts: i64 = s
            .conn
            .query_row(
                "SELECT ts_unix_ms FROM items WHERE kind='transcript' AND body_text='s2'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(ts, 61_100);
        // re-transcribing part 2 of the new meeting stays in its own range
        s.replace_part_transcript_segments(new, 2, &[seg(new, 20, "s3b")])
            .unwrap();
        assert_eq!(
            cs_of(&s, new),
            [(10, "s2".to_string()), (6_020, "s3b".to_string())]
        );
    }

    #[test]
    fn split_clears_keys_so_no_recontinue() {
        let s = Store::open_in_memory().unwrap();
        let id = three_parts(&s);
        let new = s.split_meeting(id, 3).unwrap().new_meeting_id;
        assert_eq!((key_of(&s, id), key_of(&s, new)), (None, None));
        assert_eq!(s.find_continuable_meeting("k", 191_000).unwrap(), None);
    }

    #[test]
    fn split_single_part_is_error() {
        let s = Store::open_in_memory().unwrap();
        let id = done(&s, Some("k"), 5_000);
        assert!(s.split_meeting(id, 1).is_err());
        assert!(s.split_meeting(id, 2).is_err());
        let id = three_parts(&s);
        assert!(
            s.split_meeting(id, 1).is_err(),
            "index 1 leaves nothing behind"
        );
        assert!(s.split_meeting(id, 4).is_err());
        s.reopen_meeting(id).unwrap();
        assert!(s.split_meeting(id, 2).is_err(), "recording");
        assert!(s.split_meeting(9999, 2).is_err());
        assert_eq!(s.meeting_parts(id).unwrap().len(), 3, "nothing changed");
    }

    #[test]
    fn deleting_one_split_meeting_keeps_the_other_files() {
        let s = Store::open_in_memory().unwrap();
        let id = three_parts(&s);
        let r = s.split_meeting(id, 2).unwrap();
        let new = r.new_meeting_id;
        // Before Swift moves the files, the new meeting owns no snapshot dir.
        assert_eq!(s.meeting_snapshots_info(new).unwrap().unwrap().0, None);
        let moves: Vec<(String, String)> = r
            .moved_snapshot_paths
            .iter()
            .map(|p| (p.clone(), p.replace("/snaps/o/", "/snaps/n/")))
            .collect();
        s.set_meeting_snapshot_paths(new, &moves, "/snaps/n")
            .unwrap();
        assert_eq!(
            snapshot_paths(&s, new),
            ["/snaps/n/2.png", "/snaps/n/3.png"]
        );

        let f = s.delete_meeting_returning_files(new).unwrap().unwrap();
        assert_eq!(f.snapshots_dir.as_deref(), Some("/snaps/n"));
        assert_eq!(f.audio_paths, ["/r/audio-2.caf", "/r/audio-3.caf"]);
        let f = s.delete_meeting_returning_files(id).unwrap().unwrap();
        assert_eq!(f.snapshots_dir.as_deref(), Some("/snaps/o"));
        assert_eq!(f.audio_paths, ["/r/audio-1.caf"]);
        assert_eq!(f.transcript_paths, ["/r/t1.txt"]);
    }

    #[test]
    fn split_restamps_both_meetings_and_moved_segments_for_sync() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("d");
        let id = three_parts(&s);
        let hi: i64 = s
            .conn
            .query_row("SELECT MAX(local_seq) FROM transcript_segment", [], |r| {
                r.get(0)
            })
            .unwrap();
        let new = s.split_meeting(id, 2).unwrap().new_meeting_id;
        let rows = s.unpushed_rows("d", hi).unwrap();
        let metas: Vec<i64> = rows
            .iter()
            .filter_map(|r| match r {
                crate::SyncableRow::Meeting {
                    source_meeting_id, ..
                } => Some(*source_meeting_id),
                _ => None,
            })
            .collect();
        assert!(metas.contains(&id) && metas.contains(&new));
        let segs: Vec<i64> = rows
            .iter()
            .filter_map(|r| match r {
                crate::SyncableRow::TranscriptSegment {
                    source_meeting_id, ..
                } => Some(*source_meeting_id),
                _ => None,
            })
            .collect();
        assert_eq!(segs, [new, new], "only the moved segments are re-sent");
        let outbox_new: i64 = s
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sync_outbox WHERE row_type='items' AND payload_json LIKE ?1",
                [format!("%\"source_meeting_id\":{new},%")],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(outbox_new, 2);
    }

    #[test]
    fn deleting_original_before_files_move_does_not_return_shared_snapshots_dir() {
        let s = Store::open_in_memory().unwrap();
        let id = three_parts(&s);
        let new = s.split_meeting(id, 2).unwrap().new_meeting_id;
        // Crash window: new meeting's rows still point into /snaps/o.
        let f = s.delete_meeting_returning_files(id).unwrap().unwrap();
        assert_eq!(f.snapshots_dir, None);
        // Now only the new meeting is left in that dir; it is the sole owner.
        let f = s.delete_meeting_returning_files(new).unwrap().unwrap();
        assert_eq!(
            f.snapshots_dir, None,
            "new meeting has no dir of its own yet"
        );
    }

    #[test]
    fn snapshots_dir_check_is_a_literal_prefix_not_a_like_pattern() {
        let s = Store::open_in_memory().unwrap();
        let a = mk(&s, MeetingStatus::Completed, None);
        s.set_meeting_snapshots_dir(a, "/snaps/a_b%").unwrap();
        let b = mk(&s, MeetingStatus::Completed, None);
        s.insert_meeting_snapshot(&snap(b, 1, "/snaps/aXbY/1.png"))
            .unwrap();
        let f = s.delete_meeting_returning_files(a).unwrap().unwrap();
        assert_eq!(f.snapshots_dir.as_deref(), Some("/snaps/a_b%"));
    }

    #[test]
    fn split_repoints_original_audio_path_that_named_a_moved_part() {
        let s = Store::open_in_memory().unwrap();
        let id = three_parts(&s);
        s.conn
            .execute(
                "UPDATE meeting SET audio_path='/r/audio-3.caf', transcript_path='/r/t3.txt' WHERE id=?1",
                [id],
            )
            .unwrap();
        s.split_meeting(id, 3).unwrap();
        // Rule: first remaining part that has the file.
        let m = s.get_meeting(id).unwrap().unwrap();
        assert_eq!(m.audio_path.as_deref(), Some("/r/audio-1.caf"));
        assert_eq!(m.transcript_path.as_deref(), Some("/r/t1.txt"));
    }

    #[test]
    fn snapshot_in_gap_between_parts_stays_with_original() {
        let s = Store::open_in_memory().unwrap();
        let id = three_parts(&s);
        // part 1 ends 50_000, part 2 starts 61_000
        s.insert_meeting_snapshot(&snap(id, 55_000, "/snaps/o/gap.png"))
            .unwrap();
        let r = s.split_meeting(id, 2).unwrap();
        assert!(!r
            .moved_snapshot_paths
            .contains(&"/snaps/o/gap.png".to_string()));
        assert!(snapshot_paths(&s, id).contains(&"/snaps/o/gap.png".to_string()));
    }
}
