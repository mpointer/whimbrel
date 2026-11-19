//! "Mark this moment": a point the user flagged while a meeting was recording, with an optional
//! short label. `at_cs` is centiseconds from the start of the recording on the meeting-absolute
//! clock (the one live alerts and transcript seeking use), so it includes paused time and a
//! continued meeting's later parts. Device-local: the table is never enqueued for sync, and a
//! marker goes with its meeting wherever the meeting's other rows go.

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::Serialize;

use crate::Store;

/// The longest label kept, in characters.
pub const MAX_LABEL_CHARS: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MeetingMarker {
    pub id: i64,
    pub meeting_id: i64,
    pub at_cs: i64,
    pub label: Option<String>,
    pub created_unix_ms: i64,
}

pub(crate) fn ensure_meeting_marker_table(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS meeting_marker (
            id INTEGER PRIMARY KEY,
            meeting_id INTEGER NOT NULL,
            at_cs INTEGER NOT NULL,
            label TEXT,
            created_unix_ms INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_meeting_marker_meeting ON meeting_marker(meeting_id, at_cs);",
    )
}

fn bad(msg: &str) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(msg.into())
}

/// Trimmed, cut to [`MAX_LABEL_CHARS`] characters, `None` when nothing is left.
fn clean_label(label: Option<&str>) -> Option<String> {
    let t: String = label?.trim().chars().take(MAX_LABEL_CHARS).collect();
    let t = t.trim_end().to_string();
    (!t.is_empty()).then_some(t)
}

fn row_to_marker(r: &Row<'_>) -> rusqlite::Result<MeetingMarker> {
    Ok(MeetingMarker {
        id: r.get(0)?,
        meeting_id: r.get(1)?,
        at_cs: r.get(2)?,
        label: r.get(3)?,
        created_unix_ms: r.get(4)?,
    })
}

impl Store {
    /// Adds a marker and returns its id. An error, with nothing stored, for a negative `at_cs`
    /// or a meeting that does not exist.
    pub fn add_meeting_marker(
        &self,
        meeting_id: i64,
        at_cs: i64,
        label: Option<&str>,
        created_unix_ms: i64,
    ) -> rusqlite::Result<i64> {
        if at_cs < 0 {
            return Err(bad("marker time is negative"));
        }
        let n = self.conn.execute(
            "INSERT INTO meeting_marker (meeting_id, at_cs, label, created_unix_ms) \
             SELECT id, ?2, ?3, ?4 FROM meeting WHERE id = ?1",
            params![meeting_id, at_cs, clean_label(label), created_unix_ms],
        )?;
        if n == 0 {
            return Err(bad("no such meeting"));
        }
        Ok(self.conn.last_insert_rowid())
    }

    /// The meeting's markers, earliest first (ties by id).
    pub fn list_meeting_markers(&self, meeting_id: i64) -> rusqlite::Result<Vec<MeetingMarker>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, meeting_id, at_cs, label, created_unix_ms FROM meeting_marker \
             WHERE meeting_id = ?1 ORDER BY at_cs, id",
        )?;
        let rows = stmt.query_map(params![meeting_id], row_to_marker)?;
        rows.collect()
    }

    pub fn get_meeting_marker(&self, id: i64) -> rusqlite::Result<Option<MeetingMarker>> {
        self.conn
            .query_row(
                "SELECT id, meeting_id, at_cs, label, created_unix_ms FROM meeting_marker WHERE id = ?1",
                params![id],
                row_to_marker,
            )
            .optional()
    }

    /// `true` when a marker was removed.
    pub fn delete_meeting_marker(&self, id: i64) -> rusqlite::Result<bool> {
        Ok(self.conn.execute("DELETE FROM meeting_marker WHERE id = ?1", params![id])? > 0)
    }

    /// Replaces a marker's label (cleaned as on add; empty clears it). `false` for no such marker.
    pub fn update_meeting_marker_label(&self, id: i64, label: Option<&str>) -> rusqlite::Result<bool> {
        Ok(self.conn.execute(
            "UPDATE meeting_marker SET label = ?2 WHERE id = ?1",
            params![id, clean_label(label)],
        )? > 0)
    }

    /// Markers at or after `keep_ms` on the meeting clock go: a trim removes the audio they point at.
    pub(crate) fn delete_markers_past_tx(
        tx: &Connection,
        meeting_id: i64,
        keep_ms: i64,
    ) -> rusqlite::Result<()> {
        tx.execute(
            "DELETE FROM meeting_marker WHERE meeting_id = ?1 AND at_cs * 10 >= ?2",
            params![meeting_id, keep_ms],
        )?;
        Ok(())
    }

    /// A merge hands `from`'s markers to `into`, put later on `into`'s clock by how far `from`'s
    /// recording starts after `into`'s (the origin is part 1's start, else the meeting's start,
    /// as for `part_offset_cs`). Run before `from`'s rows are deleted.
    pub(crate) fn carry_markers_tx(tx: &Connection, from: i64, into: i64) -> rusqlite::Result<()> {
        let origin = |id: i64| -> rusqlite::Result<i64> {
            tx.query_row(
                "SELECT COALESCE((SELECT start_unix_ms FROM meeting_part WHERE meeting_id = ?1 \
                                  ORDER BY part_index LIMIT 1), \
                                 (SELECT start_unix_ms FROM meeting WHERE id = ?1), 0)",
                params![id],
                |r| r.get(0),
            )
        };
        let shift_cs = (origin(from)? - origin(into)?) / 10;
        tx.execute(
            "UPDATE meeting_marker SET meeting_id = ?2, at_cs = MAX(0, at_cs + ?3) WHERE meeting_id = ?1",
            params![from, into, shift_cs],
        )?;
        Ok(())
    }

    /// A split hands the markers at or after `offset_cs` (where the new meeting begins on the
    /// old one's clock) to `new_id`, re-timed from its own start.
    pub(crate) fn split_markers_tx(
        tx: &Connection,
        old_id: i64,
        new_id: i64,
        offset_cs: i64,
    ) -> rusqlite::Result<()> {
        tx.execute(
            "UPDATE meeting_marker SET meeting_id = ?2, at_cs = at_cs - ?3 \
             WHERE meeting_id = ?1 AND at_cs >= ?3",
            params![old_id, new_id, offset_cs],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::{MeetingStatus, MeetingTrigger, NewMeeting, Store};

    fn meeting(store: &Store, start: i64) -> i64 {
        store
            .insert_meeting(&NewMeeting {
                device_id: "d".into(),
                start_unix_ms: start,
                end_unix_ms: Some(start + 600_000),
                title: "Standup".into(),
                attendees: vec![],
                trigger: MeetingTrigger::Manual,
                audio_path: None,
                status: MeetingStatus::Completed,
            })
            .unwrap()
    }

    fn count(store: &Store) -> i64 {
        store.conn.query_row("SELECT COUNT(*) FROM meeting_marker", [], |r| r.get(0)).unwrap()
    }

    fn outbox(store: &Store) -> i64 {
        store.conn.query_row("SELECT COUNT(*) FROM sync_outbox", [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn markers_list_by_time_then_id() {
        let s = Store::open_in_memory().unwrap();
        let m = meeting(&s, 1_000);
        let other = meeting(&s, 900_000);
        let b = s.add_meeting_marker(m, 500, Some("budget"), 10).unwrap();
        let a = s.add_meeting_marker(m, 100, None, 11).unwrap();
        let c = s.add_meeting_marker(m, 500, None, 12).unwrap();
        s.add_meeting_marker(other, 1, None, 13).unwrap();
        let got = s.list_meeting_markers(m).unwrap();
        assert_eq!(got.iter().map(|x| x.id).collect::<Vec<_>>(), vec![a, b, c]);
        assert_eq!(got[1].label.as_deref(), Some("budget"));
        assert_eq!((got[0].meeting_id, got[0].at_cs, got[0].created_unix_ms), (m, 100, 11));
        assert!(s.list_meeting_markers(m + 99).unwrap().is_empty());
    }

    #[test]
    fn labels_are_trimmed_capped_and_empty_becomes_null() {
        let s = Store::open_in_memory().unwrap();
        let m = meeting(&s, 1_000);
        let id = s.add_meeting_marker(m, 0, Some("  hello  "), 1).unwrap();
        assert_eq!(s.get_meeting_marker(id).unwrap().unwrap().label.as_deref(), Some("hello"));
        let long = "é".repeat(250);
        let id = s.add_meeting_marker(m, 0, Some(&long), 1).unwrap();
        assert_eq!(s.get_meeting_marker(id).unwrap().unwrap().label.unwrap().chars().count(), 200);
        let id = s.add_meeting_marker(m, 0, Some("   \n "), 1).unwrap();
        let raw: Option<String> = s
            .conn
            .query_row("SELECT label FROM meeting_marker WHERE id = ?1", [id], |r| r.get(0))
            .unwrap();
        assert_eq!(raw, None, "stored as NULL, not an empty string");
        assert!(s.update_meeting_marker_label(id, Some(" later ")).unwrap());
        assert_eq!(s.get_meeting_marker(id).unwrap().unwrap().label.as_deref(), Some("later"));
        assert!(s.update_meeting_marker_label(id, Some("")).unwrap());
        assert_eq!(s.get_meeting_marker(id).unwrap().unwrap().label, None);
        assert!(!s.update_meeting_marker_label(id + 99, Some("x")).unwrap());
    }

    #[test]
    fn bad_input_is_rejected_and_stores_nothing() {
        let s = Store::open_in_memory().unwrap();
        let m = meeting(&s, 1_000);
        assert!(s.add_meeting_marker(m, -1, None, 1).is_err());
        assert!(s.add_meeting_marker(m + 99, 5, None, 1).is_err());
        assert_eq!(count(&s), 0);
        let id = s.add_meeting_marker(m, 5, None, 1).unwrap();
        assert!(s.delete_meeting_marker(id).unwrap());
        assert!(!s.delete_meeting_marker(id).unwrap());
    }

    #[test]
    fn deleting_the_meeting_deletes_its_markers_only() {
        let s = Store::open_in_memory().unwrap();
        let m = meeting(&s, 1_000);
        let other = meeting(&s, 900_000);
        s.add_meeting_marker(m, 5, Some("x"), 1).unwrap();
        s.add_meeting_marker(other, 5, None, 1).unwrap();
        assert!(s.delete_meeting(m).unwrap());
        assert!(s.list_meeting_markers(m).unwrap().is_empty());
        assert_eq!(s.list_meeting_markers(other).unwrap().len(), 1);
    }

    #[test]
    fn the_retention_sweep_deletes_the_markers_of_expired_meetings() {
        let s = Store::open_in_memory().unwrap();
        let old = meeting(&s, 1_000);
        let fresh = meeting(&s, 10_000_000);
        s.add_meeting_marker(old, 5, None, 1).unwrap();
        s.add_meeting_marker(fresh, 5, None, 1).unwrap();
        let (n, _) = s.delete_meetings_before(5_000_000).unwrap();
        assert_eq!(n, 1);
        assert!(s.list_meeting_markers(old).unwrap().is_empty());
        assert_eq!(s.list_meeting_markers(fresh).unwrap().len(), 1);
    }

    #[test]
    fn forgetting_the_meeting_deletes_its_markers() {
        let s = Store::open_in_memory().unwrap();
        let m = meeting(&s, 1_000);
        s.add_meeting_marker(m, 5, Some("x"), 1).unwrap();
        s.forget_meeting(m).unwrap();
        assert_eq!(count(&s), 0);
    }

    #[test]
    fn a_trim_drops_markers_past_the_cut() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("d");
        let m = meeting(&s, 1_000);
        s.add_meeting_marker(m, 1_000, Some("kept"), 1).unwrap(); // 10 s in
        let past = s.add_meeting_marker(m, 3_000, Some("cut"), 1).unwrap(); // 30 s in
        assert!(s.trim_meeting(m, 20_000).unwrap().is_some());
        let left = s.list_meeting_markers(m).unwrap();
        assert_eq!(left.len(), 1);
        assert_ne!(left[0].id, past);
    }

    #[test]
    fn splitting_a_meeting_moves_later_markers_to_the_new_one() {
        let s = Store::open_in_memory().unwrap();
        let m = meeting(&s, 1_000);
        s.begin_meeting_part(m, 1_000, None).unwrap();
        s.complete_meeting_part(m, 1, 61_000, None).unwrap();
        s.begin_meeting_part(m, 101_000, None).unwrap(); // part 2: 100 s in (10_000 cs)
        s.complete_meeting_part(m, 2, 161_000, None).unwrap();
        s.add_meeting_marker(m, 2_000, Some("early"), 1).unwrap();
        s.add_meeting_marker(m, 10_500, Some("late"), 1).unwrap();
        let split = s.split_meeting(m, 2).unwrap();
        let old = s.list_meeting_markers(m).unwrap();
        let new = s.list_meeting_markers(split.new_meeting_id).unwrap();
        assert_eq!(old.iter().map(|x| x.at_cs).collect::<Vec<_>>(), vec![2_000]);
        assert_eq!(new.iter().map(|x| (x.at_cs, x.label.clone())).collect::<Vec<_>>(), vec![(500, Some("late".to_string()))]);
    }

    #[test]
    fn merging_with_the_previous_meeting_carries_markers_onto_its_clock() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("d");
        let into = meeting(&s, 1_000); // ends 601_000
        let from = meeting(&s, 661_000); // starts 60 s after `into` ended
        s.add_meeting_marker(into, 100, Some("a"), 1).unwrap();
        s.add_meeting_marker(from, 200, Some("b"), 1).unwrap();
        s.merge_into_previous(into, from).unwrap();
        let got = s.list_meeting_markers(into).unwrap();
        // `from` started 660 s after `into`: 66_000 cs.
        assert_eq!(got.iter().map(|x| x.at_cs).collect::<Vec<_>>(), vec![100, 66_200]);
        assert!(s.list_meeting_markers(from).unwrap().is_empty());
    }

    #[test]
    fn keeping_a_back_to_back_split_together_carries_markers() {
        let s = Store::open_in_memory().unwrap();
        let into = meeting(&s, 1_000);
        let from = meeting(&s, 31_000); // starts 30 s after `into`
        s.add_meeting_marker(from, 50, Some("b"), 1).unwrap();
        s.absorb_split_meeting(into, from).unwrap();
        assert_eq!(s.list_meeting_markers(into).unwrap()[0].at_cs, 3_050);
        assert!(s.list_meeting_markers(from).unwrap().is_empty());
    }

    #[test]
    fn marker_labels_count_as_meeting_text_for_client_exclusion() {
        let s = Store::open_in_memory().unwrap();
        let m = meeting(&s, 1_000);
        s.add_meeting_marker(m, 5, Some("Initech pricing"), 1).unwrap();
        assert!(s.meeting_authored_text(m).unwrap().contains("Initech pricing"));
    }

    #[test]
    fn a_duplicate_copy_hands_its_markers_to_the_kept_meeting() {
        let s = Store::open_in_memory().unwrap();
        let keep = meeting(&s, 1_000);
        let drop = meeting(&s, 1_000);
        s.add_meeting_marker(drop, 700, Some("mine"), 1).unwrap();
        assert!(s.fold_duplicate_meeting(keep, drop).unwrap());
        assert_eq!(s.list_meeting_markers(keep).unwrap()[0].at_cs, 700);
        assert_eq!(count(&s), 1);
    }

    #[test]
    fn markers_never_enter_the_sync_outbox() {
        let s = Store::open_in_memory().unwrap();
        let m = meeting(&s, 1_000);
        let before = outbox(&s);
        let id = s.add_meeting_marker(m, 5, Some("a"), 1).unwrap();
        s.update_meeting_marker_label(id, Some("b")).unwrap();
        s.add_meeting_marker(m, 9, None, 2).unwrap();
        s.delete_meeting_marker(id).unwrap();
        assert_eq!(outbox(&s), before, "a marker is device-local: nothing is enqueued");
        let rows: i64 = s
            .conn
            .query_row("SELECT COUNT(*) FROM sync_outbox WHERE row_type LIKE '%marker%' OR payload_json LIKE '%meeting_marker%'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 0);
    }

    #[test]
    fn opening_an_old_database_creates_the_table() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("w.db");
        let m = {
            let s = Store::open(&path, dir.path()).unwrap();
            let m = meeting(&s, 1_000);
            s.conn.execute("DROP TABLE meeting_marker", []).unwrap();
            m
        };
        let s = Store::open(&path, dir.path()).unwrap();
        let id = s.add_meeting_marker(m, 5, Some("back"), 1).unwrap();
        assert_eq!(s.list_meeting_markers(m).unwrap()[0].id, id);
    }
}
