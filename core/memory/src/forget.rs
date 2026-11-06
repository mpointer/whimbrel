//! "Forget meeting" removes a meeting from every device
//! (docs/superpowers/specs/2026-11-05-deletion-sync-design.md).
//!
//! Forgetting deletes the meeting here, records its sync identity in `forgotten_meeting` and enqueues a
//! `meeting_forgotten` tombstone. A peer that pulls the tombstone runs `apply_meeting_forgotten`: the same
//! delete and record, no new tombstone. The identity is (origin device, origin database epoch, origin
//! meeting id); `''` is an unknown epoch, and an unknown epoch on either side matches any epoch, so only a
//! known, different epoch (a re-created database reusing ids) keeps a tombstone off a meeting. The recorder
//! itself always knows its epoch: it takes an unknown-epoch tombstone only when id, device and start match.
//! `pushed_meeting_row` remembers which server rows a meeting was pushed under, so the recording device
//! can delete them (sync-engine).

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::sync_outbox::{self, NewOutboxOperation, OutboxOperationKind};
use crate::{
    json_to_sql_error, now_unix_ms, DeletedMeetingFiles, Store, OUTBOX_CONNECTOR, OUTBOX_ROW_TYPE_ITEMS,
    OUTBOX_ROW_TYPE_LIVE_ALERT, OUTBOX_ROW_TYPE_MEETING, OUTBOX_ROW_TYPE_MEETING_FORGOTTEN, OUTBOX_ROW_TYPE_MEETING_SPEAKER,
    OUTBOX_ROW_TYPE_PROFILE_SUGGESTION,
};

/// A `meeting_forgotten` row on the wire, and the identity `forgotten_meeting` keeps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeetingForgotten {
    pub origin_device_id: String,
    /// The recording database's `sync_entity_epoch`; `None` when unknown (a meeting pulled from a build
    /// that didn't send it).
    pub origin_epoch: Option<String>,
    /// The meeting's id on the recording device.
    pub source_meeting_id: i64,
    pub forgotten_at_unix_ms: i64,
    /// The meeting's start as the forgetting device has it. With an unknown epoch, the recorder takes the
    /// tombstone only if its own meeting with this id also starts here (a pre-release copy); absent from older
    /// builds' tombstones.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_start_unix_ms: Option<i64>,
}

/// Device + id match, and the epochs are equal or at least one is unknown (`''`). `?1` device, `?2` id,
/// `?3` epoch.
const SAME_MEETING: &str =
    "origin_device_id = ?1 AND source_meeting_id = ?2 AND (origin_epoch = '' OR ?3 = '' OR origin_epoch = ?3)";

/// Copies meeting `?1`'s delivered outbox rows (its `?2` row, its transcript items' `?3` rows, its live
/// alerts' `?4` rows and this device's `?5` speaker rows, named `:<meeting>:<speaker>`) into
/// `pushed_meeting_row`. `+state` keeps the planner off the state index (delivered rows are never pruned)
/// and on `(row_type, aggregate_id)`.
const SNAPSHOT_DELIVERED_ROWS: &str = "INSERT OR IGNORE INTO pushed_meeting_row (meeting_id, local_seq, row_type) \
     SELECT ?1, local_seq, row_type FROM sync_outbox WHERE +state = 'delivered' AND ( \
     (row_type = ?2 AND aggregate_id = CAST(?1 AS TEXT)) \
     OR (row_type = ?3 AND aggregate_id IN (SELECT CAST(id AS TEXT) FROM items WHERE thread_id = ?1 AND kind = 'transcript')) \
     OR (row_type = ?4 AND aggregate_id IN (SELECT CAST(id AS TEXT) FROM live_alert WHERE meeting_id = ?1)) \
     OR (row_type = ?5 AND aggregate_id IN (SELECT ':' || ?1 || ':' || speaker_id FROM meeting_speaker WHERE meeting_id = ?1)))";

/// How long before a meeting's start one of its server rows may be stamped (an early join, a scheduled start
/// later than the recording's). ponytail: a fixed day; an earlier database's row at a reused `local_seq`
/// pushed within this day of the new meeting's start would still be deleted. Upgrade: record each row's
/// relay `created_at` when it is pushed.
const CLEANUP_CLOCK_MARGIN_MS: i64 = 24 * 60 * 60 * 1000;

/// For a meeting deleted without a forget (retention, merge, a duplicate fold, a plain delete): its
/// `pushed_meeting_row` records name server rows nothing will delete, so they go. Only that meeting's;
/// `meeting_server_cleanup` rows are never pruned (a row leased at a forget can be acknowledged and
/// recorded long after its cleanup looked finished).
pub(crate) fn prune_pushed_rows(conn: &Connection, meeting_id: i64) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM pushed_meeting_row WHERE meeting_id = ?1", params![meeting_id])?;
    Ok(())
}

pub(crate) fn ensure_forget_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS forgotten_meeting (
            origin_device_id TEXT NOT NULL,
            origin_epoch TEXT NOT NULL DEFAULT '',
            source_meeting_id INTEGER NOT NULL,
            forgotten_at_unix_ms INTEGER NOT NULL,
            PRIMARY KEY (origin_device_id, source_meeting_id, origin_epoch)
        );
        CREATE TABLE IF NOT EXISTS pushed_meeting_row (
            meeting_id INTEGER NOT NULL,
            local_seq INTEGER NOT NULL,
            row_type TEXT NOT NULL,
            PRIMARY KEY (meeting_id, local_seq, row_type)
        );
        CREATE TABLE IF NOT EXISTS meeting_server_cleanup (
            meeting_id INTEGER PRIMARY KEY,
            not_before_unix_ms INTEGER NOT NULL,
            tag_deleted INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_items_thread ON items(thread_id) WHERE thread_id IS NOT NULL;",
    )?;
    let has_epoch = conn
        .prepare("PRAGMA table_info(sync_meeting_map)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .iter()
        .any(|c| c == "origin_epoch");
    if !has_epoch {
        conn.execute("ALTER TABLE sync_meeting_map ADD COLUMN origin_epoch TEXT NOT NULL DEFAULT ''", [])?;
    }
    Ok(())
}

impl Store {
    /// Forgets meeting `id` everywhere: deletes it here (as `delete_meeting_returning_files`), cancels its
    /// unsent outbox rows, records it as forgotten and enqueues a tombstone naming its origin identity.
    /// Returns its files to remove, `None` if no such meeting. With no identity to name (no device id set,
    /// or a meeting pulled from a build that didn't send its source id) it is only deleted here.
    pub fn forget_meeting(&self, id: i64) -> rusqlite::Result<Option<DeletedMeetingFiles>> {
        let Some((device_id, start_unix_ms)) = self
            .conn
            .query_row("SELECT device_id, start_unix_ms FROM meeting WHERE id = ?1", params![id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            })
            .optional()?
        else {
            return Ok(None);
        };
        let own = self.own_device_id();
        let mapped: Option<(String, i64, String)> = self
            .conn
            .query_row(
                "SELECT peer_device_id, source_meeting_id, origin_epoch FROM sync_meeting_map WHERE local_meeting_id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let now = now_unix_ms();
        let identity = match mapped {
            Some((origin_device_id, source_meeting_id, epoch)) => Some(MeetingForgotten {
                origin_device_id,
                origin_epoch: (!epoch.is_empty()).then_some(epoch),
                source_meeting_id,
                forgotten_at_unix_ms: now,
                source_start_unix_ms: Some(start_unix_ms),
            }),
            None if own.as_deref() == Some(device_id.as_str()) => Some(MeetingForgotten {
                origin_device_id: device_id,
                origin_epoch: Some(self.sync_entity_epoch()?),
                source_meeting_id: id,
                forgotten_at_unix_ms: now,
                source_start_unix_ms: Some(start_unix_ms),
            }),
            None => None,
        };
        self.forget_tx(Some(id), identity.as_ref(), own.as_deref())
    }

    /// Applies a peer's tombstone: deletes the local meeting it names, if any, and records it as forgotten
    /// so a row for it arriving later can be dropped. Never enqueues a tombstone. Returns the deleted
    /// meeting's files, `None` when no local meeting matched.
    pub fn apply_meeting_forgotten(&self, t: &MeetingForgotten) -> rusqlite::Result<Option<DeletedMeetingFiles>> {
        let epoch = t.origin_epoch.as_deref().unwrap_or("");
        let local = if self.own_device_id().as_deref() == Some(t.origin_device_id.as_str()) {
            let own_epoch = self.sync_entity_epoch()?;
            // The recorder always knows its own epoch, so a different one never matches: a re-created database
            // reusing the id keeps its new meeting (and its server rows). An unknown epoch (a peer's pre-release
            // copy) matches only with the same start too; without a start, never.
            let start = if epoch == own_epoch {
                None
            } else if epoch.is_empty() && t.source_start_unix_ms.is_some() {
                t.source_start_unix_ms
            } else {
                return self.forget_tx(None, Some(t), None);
            };
            self.conn
                .query_row(
                    "SELECT id FROM meeting WHERE id = ?1 AND device_id = ?2 AND (?3 IS NULL OR start_unix_ms = ?3)",
                    params![t.source_meeting_id, t.origin_device_id, start],
                    |r| r.get(0),
                )
                .optional()?
        } else {
            self.conn
                .query_row(
                    "SELECT local_meeting_id FROM sync_meeting_map WHERE peer_device_id = ?1 AND source_meeting_id = ?2 \
                     AND (origin_epoch = '' OR ?3 = '' OR origin_epoch = ?3)",
                    params![t.origin_device_id, t.source_meeting_id, epoch],
                    |r| r.get(0),
                )
                .optional()?
        };
        self.forget_tx(local, Some(t), None)
    }

    /// Whether a tombstone names this meeting (see `SAME_MEETING` for how epochs match).
    pub fn is_meeting_forgotten(
        &self,
        origin_device_id: &str,
        origin_epoch: Option<&str>,
        source_meeting_id: i64,
    ) -> rusqlite::Result<bool> {
        self.conn.query_row(
            &format!("SELECT EXISTS (SELECT 1 FROM forgotten_meeting WHERE {SAME_MEETING})"),
            params![origin_device_id, source_meeting_id, origin_epoch.unwrap_or("")],
            |r| r.get(0),
        )
    }

    /// Records that this device pushed a server row for its meeting `meeting_id`.
    pub fn record_pushed_meeting_row(&self, meeting_id: i64, local_seq: i64, row_type: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO pushed_meeting_row (meeting_id, local_seq, row_type) VALUES (?1, ?2, ?3)",
            params![meeting_id, local_seq, row_type],
        )?;
        Ok(())
    }

    /// `(local_seq, row_type)` of every server row this device pushed for its meeting `meeting_id`: the
    /// recorded ones, plus the delivered outbox rows (the meeting and its transcript items) that forgetting
    /// it snapshotted before the delete.
    pub fn pushed_meeting_rows(&self, meeting_id: i64) -> rusqlite::Result<Vec<(i64, String)>> {
        let mut stmt =
            self.conn.prepare("SELECT local_seq, row_type FROM pushed_meeting_row WHERE meeting_id = ?1 ORDER BY local_seq")?;
        let rows = stmt.query_map(params![meeting_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    }

    /// [`Store::record_pushed_meeting_row`] for many rows `(meeting_id, local_seq, row_type)`, in one transaction.
    pub fn record_pushed_meeting_rows(&self, rows: &[(i64, i64, &str)]) -> rusqlite::Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        let tx = self.conn.unchecked_transaction()?;
        for (meeting_id, local_seq, row_type) in rows {
            self.record_pushed_meeting_row(*meeting_id, *local_seq, row_type)?;
        }
        tx.commit()
    }

    /// Up to `limit` server rows `(meeting_id, local_seq, row_type)` this device pushed for its own meetings
    /// that a tombstone forgot (this device's forget, or a peer's naming this device's meeting in this
    /// database's epoch): what the recorder deletes from the relay. Kept until
    /// [`Store::clear_pushed_meeting_rows`], so a failed delete is retried, and a row delivered after the
    /// forget shows up here too.
    ///
    /// Each row comes with its meeting's `not_before_unix_ms`: a server row created earlier can't be this
    /// meeting's (it is an earlier database's row at a reused `local_seq`), so the delete must skip it.
    pub fn forgotten_pushed_meeting_rows(&self, limit: i64) -> rusqlite::Result<Vec<(i64, i64, String, i64)>> {
        // Only meetings this database actually had and deleted (`meeting_server_cleanup`), never re-derived from
        // `forgotten_meeting` by id: a tombstone alone must not reach a meeting it didn't delete here.
        let mut stmt = self.conn.prepare(
            "SELECT p.meeting_id, p.local_seq, p.row_type, c.not_before_unix_ms FROM meeting_server_cleanup c \
             JOIN pushed_meeting_row p ON p.meeting_id = c.meeting_id LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
        rows.collect()
    }

    /// This device's forgotten meetings whose `meeting` server row is still to be deleted by its entity tag
    /// (covers versions pushed before `pushed_meeting_row` existed).
    pub fn meetings_awaiting_tag_delete(&self) -> rusqlite::Result<Vec<i64>> {
        let mut stmt = self.conn.prepare("SELECT meeting_id FROM meeting_server_cleanup WHERE tag_deleted = 0")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        rows.collect()
    }

    /// The tag delete for this meeting succeeded.
    pub fn mark_meeting_tag_deleted(&self, meeting_id: i64) -> rusqlite::Result<()> {
        self.conn
            .execute("UPDATE meeting_server_cleanup SET tag_deleted = 1 WHERE meeting_id = ?1", params![meeting_id])?;
        Ok(())
    }

    /// Drops these `pushed_meeting_row` records once their server rows are deleted.
    pub fn clear_pushed_meeting_rows(&self, rows: &[(i64, i64, String)]) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        for (meeting_id, local_seq, row_type) in rows {
            tx.execute(
                "DELETE FROM pushed_meeting_row WHERE meeting_id = ?1 AND local_seq = ?2 AND row_type = ?3",
                params![meeting_id, local_seq, row_type],
            )?;
        }
        tx.commit()
    }

    /// Shared by forget and apply, in one transaction: snapshots meeting `local`'s delivered rows into
    /// `pushed_meeting_row`, cancels its unsent outbox rows, deletes it and its sync map entry, records
    /// `identity` as forgotten, and enqueues its tombstone when `enqueue_as` (this device's id) is given.
    fn forget_tx(
        &self,
        local: Option<i64>,
        identity: Option<&MeetingForgotten>,
        enqueue_as: Option<&str>,
    ) -> rusqlite::Result<Option<DeletedMeetingFiles>> {
        let files = match local {
            Some(id) => self.meeting_files(id)?,
            None => None,
        };
        let tx = self.conn.unchecked_transaction()?;
        if let (Some(id), Some(_)) = (local, &files) {
            // Before the delete: the transcript items' ids name their outbox rows.
            tx.execute(
                SNAPSHOT_DELIVERED_ROWS,
                params![id, OUTBOX_ROW_TYPE_MEETING, OUTBOX_ROW_TYPE_ITEMS, OUTBOX_ROW_TYPE_LIVE_ALERT, OUTBOX_ROW_TYPE_MEETING_SPEAKER],
            )?;
            // This device's own meeting, deleted here: its server rows are the recorder's to remove (kept, so a
            // row delivered after the forget is removed too; ids are AUTOINCREMENT, never reused).
            if let Some(own) = self.own_device_id() {
                tx.execute(
                    "INSERT OR IGNORE INTO meeting_server_cleanup (meeting_id, not_before_unix_ms) \
                     SELECT id, start_unix_ms - ?3 FROM meeting WHERE id = ?1 AND device_id = ?2",
                    params![id, own, CLEANUP_CLOCK_MARGIN_MS],
                )?;
            }
            // Every unsent row about the meeting. A speaker row is named `<origin>:<source id>:<speaker>`,
            // with an empty origin for this device's own meeting.
            let own_speaker_prefix = format!(":{id}:");
            let speaker_prefix =
                identity.map_or(own_speaker_prefix.clone(), |t| format!("{}:{}:", t.origin_device_id, t.source_meeting_id));
            tx.execute(
                "DELETE FROM sync_outbox WHERE state IN ('pending', 'blocked') AND ( \
                 (row_type = ?2 AND aggregate_id = CAST(?1 AS TEXT)) \
                 OR (row_type = ?3 AND aggregate_id IN (SELECT CAST(id AS TEXT) FROM items WHERE thread_id = ?1 AND kind = 'transcript')) \
                 OR (row_type = ?4 AND aggregate_id IN (SELECT CAST(id AS TEXT) FROM live_alert WHERE meeting_id = ?1)) \
                 OR (row_type = ?5 AND aggregate_id IN (SELECT CAST(id AS TEXT) FROM profile_suggestion WHERE source_meeting_id = ?1)) \
                 OR (row_type = ?6 AND aggregate_id IN (SELECT ?7 || speaker_id FROM meeting_speaker WHERE meeting_id = ?1 \
                                                        UNION SELECT ?8 || speaker_id FROM meeting_speaker WHERE meeting_id = ?1)))",
                params![
                    id,
                    OUTBOX_ROW_TYPE_MEETING,
                    OUTBOX_ROW_TYPE_ITEMS,
                    OUTBOX_ROW_TYPE_LIVE_ALERT,
                    OUTBOX_ROW_TYPE_PROFILE_SUGGESTION,
                    OUTBOX_ROW_TYPE_MEETING_SPEAKER,
                    own_speaker_prefix,
                    speaker_prefix,
                ],
            )?;
            Self::delete_meeting_rows_tx(&tx, id)?;
        }
        // Even when the meeting row is already gone, its map entry must not outlive the forget.
        if let Some(id) = local {
            tx.execute("DELETE FROM sync_meeting_map WHERE local_meeting_id = ?1", params![id])?;
        }
        if let Some(t) = identity {
            tx.execute(
                "INSERT OR IGNORE INTO forgotten_meeting (origin_device_id, origin_epoch, source_meeting_id, forgotten_at_unix_ms) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![t.origin_device_id, t.origin_epoch.as_deref().unwrap_or(""), t.source_meeting_id, t.forgotten_at_unix_ms],
            )?;
            // Rows that reached this device before their meeting and wait for it: it never comes now.
            tx.execute(
                "DELETE FROM meeting_speaker_pending WHERE origin_device_id = ?1 AND source_meeting_id = ?2",
                params![t.origin_device_id, t.source_meeting_id],
            )?;
            tx.execute(
                "DELETE FROM live_alert WHERE meeting_id IS NULL AND origin_device_id = ?1 AND source_meeting_id = ?2",
                params![t.origin_device_id, t.source_meeting_id],
            )?;
            if let Some(own) = enqueue_as {
                let local_seq = Self::next_local_seq_tx(&tx)?;
                sync_outbox::enqueue_outbox_tx(
                    &tx,
                    NewOutboxOperation {
                        device_id: own.to_string(),
                        local_seq,
                        row_type: OUTBOX_ROW_TYPE_MEETING_FORGOTTEN.to_string(),
                        aggregate_type: OUTBOX_ROW_TYPE_MEETING_FORGOTTEN.to_string(),
                        // The dispatcher's identity is `<row_type>:<aggregate_id>`: `meeting_forgotten:<device>:<id>`.
                        aggregate_id: format!("{}:{}", t.origin_device_id, t.source_meeting_id),
                        operation_kind: OutboxOperationKind::Tombstone,
                        payload_json: serde_json::to_string(t).map_err(json_to_sql_error)?,
                        connector: OUTBOX_CONNECTOR.to_string(),
                    },
                    now_unix_ms(),
                )?;
            }
        }
        tx.commit()?;
        Ok(files)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MeetingStatus, MeetingTrigger, NewMeeting, NewTranscriptSegment, RetentionPolicy};

    fn meeting(store: &Store, device_id: &str, start_unix_ms: i64) -> i64 {
        store
            .insert_meeting(&NewMeeting {
                device_id: device_id.into(),
                start_unix_ms,
                end_unix_ms: None,
                title: "Standup".into(),
                attendees: Vec::new(),
                trigger: MeetingTrigger::Manual,
                audio_path: Some("/tmp/a.m4a".into()),
                status: MeetingStatus::Recording,
            })
            .unwrap()
    }

    fn segments(store: &Store, meeting_id: i64) {
        let seg = |start_cs: i64| NewTranscriptSegment { meeting_id, start_cs, speaker_label: "Me".into(), text: "hi".into() };
        store.insert_transcript_segments(&[seg(100), seg(200)]).unwrap();
    }

    /// `(row_type, aggregate_type, aggregate_id, operation_kind, state, device_id, payload)` of every outbox row.
    #[allow(clippy::type_complexity)]
    fn outbox(store: &Store) -> Vec<(String, String, String, String, String, String, String)> {
        let mut stmt = store
            .conn
            .prepare(
                "SELECT row_type, aggregate_type, aggregate_id, operation_kind, state, device_id, payload_json \
                 FROM sync_outbox ORDER BY local_seq",
            )
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    fn tombstones(store: &Store) -> Vec<(String, String, String, MeetingForgotten)> {
        outbox(store)
            .into_iter()
            .filter(|r| r.0 == OUTBOX_ROW_TYPE_MEETING_FORGOTTEN)
            .map(|r| (r.1, r.2, r.3, serde_json::from_str(&r.6).unwrap()))
            .collect()
    }

    fn forgotten_count(store: &Store) -> i64 {
        store.conn.query_row("SELECT COUNT(*) FROM forgotten_meeting", [], |r| r.get(0)).unwrap()
    }

    /// Final pass 3: without its own device id the store can't tell its meetings from a peer's, so it hands
    /// back no audio or transcript path at all; an empty id counts as none.
    #[test]
    fn without_an_own_device_id_no_paths_are_handed_back() {
        for own in [None, Some("")] {
            let store = Store::open_in_memory().unwrap();
            if let Some(own) = own {
                store.set_own_device_id(own);
            }
            assert_eq!(store.own_device_id(), None);
            let id = meeting(&store, "dev-a", 5000);
            store.conn.execute("UPDATE meeting SET transcript_path = '/r/transcript-A.txt' WHERE id = ?1", [id]).unwrap();
            let files = store.delete_meeting_returning_files(id).unwrap().expect("deleted");
            assert_eq!((files.audio_path, files.transcript_path), (None, None));
            assert!(files.audio_paths.is_empty() && files.transcript_paths.is_empty());
        }
    }

    #[test]
    fn a_pulled_meetings_synced_transcript_path_is_never_handed_back_for_deletion() {
        let store = Store::open_in_memory().unwrap();
        store.set_own_device_id("dev-b");
        let local = meeting(&store, "dev-a", 5000);
        store.conn.execute("UPDATE meeting SET audio_path = NULL, transcript_path = '/etc/hosts' WHERE id = ?1", [local]).unwrap();
        store.record_sync_meeting_map_with_epoch("dev-a", 42, local, Some("epoch-a")).unwrap();
        let t = MeetingForgotten { origin_device_id: "dev-a".into(), origin_epoch: Some("epoch-a".into()), source_meeting_id: 42, forgotten_at_unix_ms: 1, source_start_unix_ms: None };

        let files = store.apply_meeting_forgotten(&t).unwrap().expect("deleted");
        assert_eq!(files.transcript_path, None, "a peer wrote that path");
        assert!(files.transcript_paths.is_empty() && files.audio_paths.is_empty());
        assert!(store.get_meeting(local).unwrap().is_none());
    }

    #[test]
    fn forget_own_meeting_enqueues_tombstone_with_epoch() {
        let store = Store::open_in_memory().unwrap();
        store.set_own_device_id("dev-a");
        let id = meeting(&store, "dev-a", 5000);
        store.complete_meeting(id, 6000, Some("/tmp/a.m4a")).unwrap();
        let epoch = store.sync_entity_epoch().unwrap();

        let files = store.forget_meeting(id).unwrap().expect("files of the deleted meeting");
        assert_eq!(files.audio_path.as_deref(), Some("/tmp/a.m4a"));
        assert!(store.get_meeting(id).unwrap().is_none());

        let t = tombstones(&store);
        assert_eq!(t.len(), 1);
        let (aggregate_type, aggregate_id, kind, payload) = &t[0];
        assert_eq!(aggregate_type, "meeting_forgotten");
        assert_eq!(aggregate_id, &format!("dev-a:{id}"));
        assert_eq!(kind, "tombstone");
        assert_eq!(payload.origin_device_id, "dev-a");
        assert_eq!(payload.origin_epoch.as_deref(), Some(epoch.as_str()));
        assert_eq!(payload.source_meeting_id, id);
        assert!(store.is_meeting_forgotten("dev-a", Some(&epoch), id).unwrap());
        assert!(store.forget_meeting(id).unwrap().is_none(), "a second forget finds nothing");
        assert_eq!(tombstones(&store).len(), 1);
    }

    #[test]
    fn forget_pulled_meeting_uses_origin_identity() {
        let store = Store::open_in_memory().unwrap();
        store.set_own_device_id("dev-b");
        let local = meeting(&store, "dev-a", 5000);
        store.record_sync_meeting_map_with_epoch("dev-a", 42, local, Some("epoch-a")).unwrap();
        store.record_sync_meeting_map("dev-a", 42, local).unwrap(); // no epoch: keeps the known one

        assert!(store.forget_meeting(local).unwrap().is_some());

        let t = tombstones(&store);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].1, "dev-a:42");
        assert_eq!(
            t[0].3,
            MeetingForgotten {
                origin_device_id: "dev-a".into(),
                origin_epoch: Some("epoch-a".into()),
                source_meeting_id: 42,
                forgotten_at_unix_ms: t[0].3.forgotten_at_unix_ms,
                source_start_unix_ms: Some(5000),
            }
        );
        assert_eq!(outbox(&store).last().unwrap().5, "dev-b", "this device pushes the tombstone");
        assert_eq!(store.resolve_sync_meeting_map("dev-a", 42).unwrap(), None);
        assert!(store.is_meeting_forgotten("dev-a", Some("epoch-a"), 42).unwrap());
        assert!(!store.is_meeting_forgotten("dev-a", Some("epoch-a"), local).unwrap(), "not the local id");
    }

    #[test]
    fn forget_cancels_pending_outbox_ops() {
        let store = Store::open_in_memory().unwrap();
        store.set_own_device_id("dev-a");
        let id = meeting(&store, "dev-a", 5000);
        segments(&store, id);
        store.complete_meeting(id, 6000, None).unwrap();
        let other = meeting(&store, "dev-a", 7000);
        segments(&store, other);
        store.complete_meeting(other, 8000, None).unwrap();
        assert_eq!(outbox(&store).len(), 6);

        store.forget_meeting(id).unwrap();

        let left: Vec<(String, String)> = outbox(&store).into_iter().map(|r| (r.0, r.4)).collect();
        assert_eq!(
            left,
            vec![
                ("items".to_string(), "pending".to_string()),
                ("items".to_string(), "pending".to_string()),
                ("meeting".to_string(), "pending".to_string()),
                ("meeting_forgotten".to_string(), "pending".to_string()),
            ],
            "only the other meeting's rows and the tombstone remain"
        );
    }

    #[test]
    fn retention_delete_emits_no_tombstone() {
        let store = Store::open_in_memory().unwrap();
        store.set_own_device_id("dev-a");
        let old = meeting(&store, "dev-a", 5000);
        store.complete_meeting(old, 6000, None).unwrap();
        let deleted = meeting(&store, "dev-a", 5000);
        let policy = RetentionPolicy { meeting_days: Some(1), ..Default::default() };

        assert!(store.delete_meeting_returning_files(deleted).unwrap().is_some());
        assert_eq!(store.apply_retention(&policy, 10 * 86_400_000).unwrap().meetings_deleted, 1);

        assert!(tombstones(&store).is_empty());
        assert_eq!(forgotten_count(&store), 0);
    }

    /// A peer that only knows a pre-release meeting (map epoch '') forgets it: the tombstone carries the
    /// meeting's start, and the recorder takes an unknown epoch only when id, device and start all match.
    #[test]
    fn an_unknown_epoch_tombstone_reaches_the_recorder_only_with_the_same_start() {
        let peer = Store::open_in_memory().unwrap();
        peer.set_own_device_id("dev-b");
        let copy = meeting(&peer, "dev-a", 5000);
        peer.record_sync_meeting_map("dev-a", 7, copy).unwrap();
        peer.forget_meeting(copy).unwrap().unwrap();
        let sent = &tombstones(&peer)[0].3;
        assert_eq!((sent.origin_epoch.as_deref(), sent.source_start_unix_ms), (None, Some(5000)));

        let recorder = Store::open_in_memory().unwrap();
        recorder.set_own_device_id("dev-a");
        let own = meeting(&recorder, "dev-a", 5000);
        let at = |start: Option<i64>| MeetingForgotten { source_meeting_id: own, source_start_unix_ms: start, ..sent.clone() };
        assert!(recorder.apply_meeting_forgotten(&at(None)).unwrap().is_none(), "no start: never");
        assert!(recorder.apply_meeting_forgotten(&at(Some(5001))).unwrap().is_none(), "another start: another meeting");
        assert!(recorder.get_meeting(own).unwrap().is_some());
        assert!(recorder.apply_meeting_forgotten(&at(Some(5000))).unwrap().is_some());
        assert!(recorder.get_meeting(own).unwrap().is_none());
        assert_eq!(recorder.meetings_awaiting_tag_delete().unwrap(), vec![own], "its server rows go next");
    }

    #[test]
    fn tombstone_does_not_match_new_epoch() {
        let store = Store::open_in_memory().unwrap();
        store.set_own_device_id("dev-a");
        let own = meeting(&store, "dev-a", 5000);
        let pulled = meeting(&store, "dev-c", 5000);
        store.record_sync_meeting_map_with_epoch("dev-c", 9, pulled, Some("new-epoch")).unwrap();
        let stale = |device: &str, source_meeting_id: i64, epoch: Option<&str>| MeetingForgotten {
            origin_device_id: device.into(),
            origin_epoch: epoch.map(String::from),
            source_meeting_id,
            forgotten_at_unix_ms: 1,
            source_start_unix_ms: None,
        };

        // A tombstone from an earlier database (same device, same ids, other epoch) deletes nothing.
        assert!(store.apply_meeting_forgotten(&stale("dev-a", own, Some("old-epoch"))).unwrap().is_none());
        assert!(store.apply_meeting_forgotten(&stale("dev-c", 9, Some("old-epoch"))).unwrap().is_none());
        assert!(store.get_meeting(own).unwrap().is_some());
        assert!(store.get_meeting(pulled).unwrap().is_some());
        assert!(!store.is_meeting_forgotten("dev-c", Some("new-epoch"), 9).unwrap());
        assert!(store.is_meeting_forgotten("dev-c", Some("old-epoch"), 9).unwrap());
        assert!(store.is_meeting_forgotten("dev-c", None, 9).unwrap(), "an unknown epoch matches");

        // The recorder never takes an unknown epoch for its own meeting (its own epoch is always known).
        assert!(store.apply_meeting_forgotten(&stale("dev-a", own, None)).unwrap().is_none());
        assert!(store.get_meeting(own).unwrap().is_some());

        // The same epoch, or an unknown one from a peer, does match; applying never enqueues a tombstone.
        let epoch = store.sync_entity_epoch().unwrap();
        assert!(store.apply_meeting_forgotten(&stale("dev-a", own, Some(&epoch))).unwrap().is_some());
        assert!(store.apply_meeting_forgotten(&stale("dev-c", 9, None)).unwrap().is_some());
        assert!(store.get_meeting(own).unwrap().is_none());
        assert!(store.get_meeting(pulled).unwrap().is_none());
        assert!(tombstones(&store).is_empty());
        // Recorded once per identity, however often it arrives.
        let before = forgotten_count(&store);
        store.apply_meeting_forgotten(&stale("dev-c", 9, None)).unwrap();
        assert_eq!(forgotten_count(&store), before);
    }

    /// Claims every due outbox row as worker "w" and acknowledges it.
    fn deliver_all(store: &Store) -> Vec<crate::ClaimedOutboxOperation> {
        let ops = store.claim_due_outbox_batch(OUTBOX_CONNECTOR, "w", i64::MAX / 2, i64::MAX / 2 + 1, 100).unwrap();
        for op in &ops {
            assert!(store.acknowledge_outbox_operation(&op.operation_id, "w", "ok", i64::MAX / 2).unwrap());
        }
        ops
    }

    /// `(local_seq, row_type)` of the delivered `ops` about meeting `id`: its meeting row and its items.
    fn rows_of(store: &Store, ops: &[crate::ClaimedOutboxOperation], id: i64) -> Vec<(i64, String)> {
        let items: Vec<String> = {
            let mut stmt = store.conn.prepare("SELECT CAST(id AS TEXT) FROM items WHERE thread_id = ?1").unwrap();
            stmt.query_map(params![id], |r| r.get(0)).unwrap().map(Result::unwrap).collect()
        };
        let mut rows: Vec<(i64, String)> = ops
            .iter()
            .filter(|op| {
                (op.row_type == OUTBOX_ROW_TYPE_MEETING && op.aggregate_id == id.to_string())
                    || (op.row_type == OUTBOX_ROW_TYPE_ITEMS && items.contains(&op.aggregate_id))
            })
            .map(|op| (op.local_seq, op.row_type.clone()))
            .collect();
        rows.sort();
        rows
    }

    #[test]
    fn pushed_rows_include_delivered_outbox_rows() {
        let store = Store::open_in_memory().unwrap();
        store.set_own_device_id("dev-a");
        let id = meeting(&store, "dev-a", 5000);
        segments(&store, id);
        store.complete_meeting(id, 6000, None).unwrap();
        let ops = deliver_all(&store);
        let mut expected = rows_of(&store, &ops, id);
        assert_eq!(expected.len(), 3, "two items and the meeting");
        store.record_pushed_meeting_row(id, 999, "transcript_segment").unwrap();
        store.record_pushed_meeting_row(id, 999, "transcript_segment").unwrap();
        assert_eq!(store.pushed_meeting_rows(id).unwrap(), vec![(999, "transcript_segment".to_string())]);

        store.forget_meeting(id).unwrap();
        expected.push((999, "transcript_segment".into()));
        expected.sort();
        assert_eq!(store.pushed_meeting_rows(id).unwrap(), expected, "snapshotted before the delete");
    }

    fn pushed_count(store: &Store) -> i64 {
        store.conn.query_row("SELECT COUNT(*) FROM pushed_meeting_row", [], |r| r.get(0)).unwrap()
    }

    /// Final pass 7: retention, merge and a plain delete (the split's discard) leave no `pushed_meeting_row`
    /// behind for the meeting they delete, and only for it.
    #[test]
    fn other_meeting_deletes_prune_pushed_rows() {
        let store = Store::open_in_memory().unwrap();
        store.set_own_device_id("dev-a");
        let old = meeting(&store, "dev-a", 5000);
        store.complete_meeting(old, 6000, None).unwrap();
        let kept = meeting(&store, "dev-a", 20 * 86_400_000);
        for m in [old, kept] {
            store.record_pushed_meeting_row(m, 1, "items").unwrap();
        }
        let policy = RetentionPolicy { meeting_days: Some(1), ..Default::default() };
        assert_eq!(store.apply_retention(&policy, 10 * 86_400_000).unwrap().meetings_deleted, 1);
        assert!(store.pushed_meeting_rows(old).unwrap().is_empty(), "retention");
        assert_eq!(store.pushed_meeting_rows(kept).unwrap().len(), 1);

        assert!(store.delete_meeting_returning_files(kept).unwrap().is_some());
        assert_eq!(pushed_count(&store), 0, "a plain delete");
    }

    #[test]
    fn a_merge_prunes_the_merged_meetings_pushed_rows() {
        let store = Store::open_in_memory().unwrap();
        store.set_own_device_id("dev-a");
        let into = meeting(&store, "dev-a", 5000);
        store.complete_meeting(into, 6000, None).unwrap();
        let from = meeting(&store, "dev-a", 7000);
        store.complete_meeting(from, 8000, None).unwrap();
        store.record_pushed_meeting_row(from, 1, "items").unwrap();
        assert_eq!(store.previous_mergeable_meeting(from, "dev-a").unwrap(), Some(into));
        store.merge_into_previous(into, from).unwrap();
        assert!(store.get_meeting(from).unwrap().is_none());
        assert_eq!(pushed_count(&store), 0);
    }

    /// Final pass 2: the meeting's delivered live alerts and own speaker rows are snapshotted too.
    #[test]
    fn pushed_rows_include_delivered_alert_and_speaker_rows() {
        let store = Store::open_in_memory().unwrap();
        store.set_own_device_id("dev-a");
        let id = meeting(&store, "dev-a", 5000);
        let other = meeting(&store, "dev-a", 7000);
        for m in [id, other] {
            store
                .insert_live_alert(&crate::live_alert::NewLiveAlert {
                    meeting_id: m,
                    meeting_title: "T".into(),
                    quote: "q".into(),
                    restatement: "r".into(),
                    asker: None,
                    at_cs: 0,
                    created_at_unix_ms: 1,
                })
                .unwrap();
            store.complete_meeting(m, 8000, None).unwrap();
            store.upsert_person("sam@x.com", "Sam", None).unwrap();
            store.set_meeting_speaker(m, 0, Some("sam@x.com"), None).unwrap();
        }
        let ops = deliver_all(&store);
        let alert_id: String = store.conn.query_row("SELECT CAST(id AS TEXT) FROM live_alert WHERE meeting_id = ?1", [id], |r| r.get(0)).unwrap();
        let mut extra: Vec<(i64, String)> = ops
            .iter()
            .filter(|op| {
                (op.row_type == OUTBOX_ROW_TYPE_LIVE_ALERT && op.aggregate_id == alert_id)
                    || (op.row_type == OUTBOX_ROW_TYPE_MEETING_SPEAKER && op.aggregate_id == format!(":{id}:0"))
            })
            .map(|op| (op.local_seq, op.row_type.clone()))
            .collect();
        assert_eq!(extra.len(), 2, "{ops:?}");
        store.conn.execute("DELETE FROM pushed_meeting_row", []).unwrap(); // only the snapshot names them now

        store.forget_meeting(id).unwrap();
        let mut expected = rows_of(&store, &ops, id);
        expected.append(&mut extra);
        expected.sort();
        assert_eq!(store.pushed_meeting_rows(id).unwrap(), expected);
    }

    /// The snapshot takes the delivered meeting and transcript rows of that meeting only, on apply too.
    #[test]
    fn forget_snapshots_only_that_meetings_delivered_rows() {
        let store = Store::open_in_memory().unwrap();
        store.set_own_device_id("dev-a");
        let id = meeting(&store, "dev-a", 5000);
        segments(&store, id);
        store.complete_meeting(id, 6000, None).unwrap();
        let other = meeting(&store, "dev-a", 7000);
        segments(&store, other);
        store.complete_meeting(other, 8000, None).unwrap();
        let ops = deliver_all(&store);
        let expected = rows_of(&store, &ops, id);
        assert_eq!(expected.len(), 3);

        let epoch = store.sync_entity_epoch().unwrap();
        let t = MeetingForgotten { origin_device_id: "dev-a".into(), origin_epoch: Some(epoch), source_meeting_id: id, forgotten_at_unix_ms: 1, source_start_unix_ms: None };
        assert!(store.apply_meeting_forgotten(&t).unwrap().is_some());

        assert_eq!(store.pushed_meeting_rows(id).unwrap(), expected);
        assert!(store.pushed_meeting_rows(other).unwrap().is_empty());
        let all: i64 = store.conn.query_row("SELECT COUNT(*) FROM pushed_meeting_row", [], |r| r.get(0)).unwrap();
        assert_eq!(all, 3);
    }

    /// The snapshot's outbox and items lookups are index seeks, not table scans.
    #[test]
    fn snapshot_lookups_use_indexes() {
        let store = Store::open_in_memory().unwrap();
        let mut stmt = store.conn.prepare(&format!("EXPLAIN QUERY PLAN {SNAPSHOT_DELIVERED_ROWS}")).unwrap();
        let p = stmt
            .query_map(
                params![1, OUTBOX_ROW_TYPE_MEETING, OUTBOX_ROW_TYPE_ITEMS, OUTBOX_ROW_TYPE_LIVE_ALERT, OUTBOX_ROW_TYPE_MEETING_SPEAKER],
                |r| r.get::<_, String>(3),
            )
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(p.contains("idx_sync_outbox_aggregate"), "{p}");
        assert!(p.contains("idx_items_thread"), "{p}");
        assert!(p.contains("idx_live_alert_meeting"), "{p}");
        assert!(p.contains("meeting_speaker"), "{p}");
        assert!(
            !p.contains("SCAN sync_outbox") && !p.contains("SCAN items") && !p.contains("SCAN live_alert")
                && !p.contains("SCAN meeting_speaker"),
            "{p}"
        );
    }

    #[test]
    fn forget_without_identity_records_nothing() {
        // No device id set: deleted here only.
        let store = Store::open_in_memory().unwrap();
        let id = meeting(&store, "dev-a", 5000);
        assert!(store.forget_meeting(id).unwrap().is_some());
        assert_eq!(forgotten_count(&store), 0);
        assert!(tombstones(&store).is_empty());

        // A peer's meeting with no map entry (a build that didn't send its source id): same.
        store.set_own_device_id("dev-b");
        let legacy = meeting(&store, "dev-a", 5000);
        assert!(store.forget_meeting(legacy).unwrap().is_some());
        assert!(store.get_meeting(legacy).unwrap().is_none());
        assert_eq!(forgotten_count(&store), 0);
        assert!(tombstones(&store).is_empty());
    }

    #[test]
    fn applying_clears_the_map_entry_of_a_meeting_already_gone() {
        let store = Store::open_in_memory().unwrap();
        store.set_own_device_id("dev-b");
        let local = meeting(&store, "dev-a", 5000);
        store.record_sync_meeting_map_with_epoch("dev-a", 42, local, Some("e")).unwrap();
        store.delete_meeting(local).unwrap();
        let t = MeetingForgotten { origin_device_id: "dev-a".into(), origin_epoch: Some("e".into()), source_meeting_id: 42, forgotten_at_unix_ms: 1, source_start_unix_ms: None };

        assert!(store.apply_meeting_forgotten(&t).unwrap().is_none());
        assert_eq!(store.resolve_sync_meeting_map("dev-a", 42).unwrap(), None);
        assert!(store.is_meeting_forgotten("dev-a", Some("e"), 42).unwrap());
    }

    #[test]
    fn forget_cancels_blocked_ops_and_keeps_leased_ones() {
        let store = Store::open_in_memory().unwrap();
        store.set_own_device_id("dev-a");
        let id = meeting(&store, "dev-a", 5000);
        segments(&store, id);
        store.complete_meeting(id, 6000, None).unwrap();
        let ops = store.claim_due_outbox_batch(OUTBOX_CONNECTOR, "w", i64::MAX / 2, i64::MAX / 2 + 1, 100).unwrap();
        assert_eq!(ops.len(), 3);
        for op in &ops[..2] {
            store
                .release_outbox_after_failure(&op.operation_id, "w", crate::DeliveryFailureClass::NeedsAuthentication, "auth", "", None, i64::MAX / 2)
                .unwrap();
        }
        let states: Vec<String> = outbox(&store).into_iter().map(|r| r.4).collect();
        assert_eq!(states, ["blocked", "blocked", "leased"]);

        store.forget_meeting(id).unwrap();

        let left: Vec<(String, String)> = outbox(&store).into_iter().map(|r| (r.0, r.4)).collect();
        assert_eq!(
            left,
            vec![(ops[2].row_type.clone(), "leased".to_string()), ("meeting_forgotten".to_string(), "pending".to_string())]
        );
    }

    #[test]
    fn origin_epoch_column_is_added_to_an_existing_map() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE items (id INTEGER PRIMARY KEY, thread_id INTEGER);
             CREATE TABLE sync_meeting_map (peer_device_id TEXT NOT NULL, source_meeting_id INTEGER NOT NULL, \
             local_meeting_id INTEGER NOT NULL, PRIMARY KEY (peer_device_id, source_meeting_id));
             INSERT INTO sync_meeting_map VALUES ('dev-a', 42, 7);",
        )
        .unwrap();
        ensure_forget_tables(&conn).unwrap();
        ensure_forget_tables(&conn).unwrap();
        let epoch: String = conn.query_row("SELECT origin_epoch FROM sync_meeting_map", [], |r| r.get(0)).unwrap();
        assert_eq!(epoch, "");
    }

    #[test]
    fn cleanup_lists_only_own_meetings_forgotten_in_this_epoch() {
        let store = Store::open_in_memory().unwrap();
        store.set_own_device_id("dev-a");
        let id = meeting(&store, "dev-a", 5000);
        store.record_pushed_meeting_rows(&[(id, 7, "meeting"), (id, 8, "items")]).unwrap();
        store.record_pushed_meeting_rows(&[(id + 1, 9, "meeting")]).unwrap();
        let tombstone = |device: &str, epoch: Option<&str>| MeetingForgotten {
            origin_device_id: device.into(),
            origin_epoch: epoch.map(Into::into),
            source_meeting_id: id,
            forgotten_at_unix_ms: 1,
            source_start_unix_ms: None,
        };
        store.apply_meeting_forgotten(&tombstone("dev-b", None)).unwrap();
        store.apply_meeting_forgotten(&tombstone("dev-a", Some("an-older-database"))).unwrap();
        store.apply_meeting_forgotten(&tombstone("dev-a", None)).unwrap();
        assert!(store.get_meeting(id).unwrap().is_some());
        assert!(
            store.forgotten_pushed_meeting_rows(10).unwrap().is_empty(),
            "another device's, another epoch's, or an unknown epoch's"
        );

        store.apply_meeting_forgotten(&tombstone("dev-a", Some(&store.sync_entity_epoch().unwrap()))).unwrap();
        let rows = store.forgotten_pushed_meeting_rows(10).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.3 == 5000 - CLEANUP_CLOCK_MARGIN_MS), "bounded by the meeting's start");
        let keys: Vec<(i64, i64, String)> = rows.into_iter().map(|r| (r.0, r.1, r.2)).collect();
        store.clear_pushed_meeting_rows(&keys).unwrap();
        assert!(store.forgotten_pushed_meeting_rows(10).unwrap().is_empty());
        assert_eq!(store.meetings_awaiting_tag_delete().unwrap(), vec![id]);
        store.mark_meeting_tag_deleted(id).unwrap();
        assert!(store.meetings_awaiting_tag_delete().unwrap().is_empty());
        assert_eq!(store.pushed_meeting_rows(id + 1).unwrap().len(), 1, "another meeting's record stays");
    }
}
