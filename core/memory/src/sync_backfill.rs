//! Local bookkeeping for a phone's newest-first pull (#572). Local only: nothing here is
//! pushed, and the relay's `sync_row` schema is unchanged.
//!
//! * A per-peer gap on `sync_cursor` (`gap_lo`, `gap_hi`, both exclusive, NULL = no gap):
//!   the peer rows between them are still to be pulled, downward from `gap_hi`. The live
//!   cursor (`last_pulled_seq`) moves to the peer's top in the same transaction that
//!   records the gap, so no row is ever left in neither range.
//! * `sync_applied_version`: the source `local_seq` (and origin database epoch) of the
//!   version stored for a peer's meeting, routine run or live alert. Rows can now arrive
//!   newer-before-older, so an apply checks it and an older version never overwrites a
//!   newer one.

use rusqlite::{params, Connection, OptionalExtension};

use crate::Store;

pub(crate) fn ensure_sync_backfill_schema(conn: &Connection) -> rusqlite::Result<()> {
    let columns = conn
        .prepare("PRAGMA table_info(sync_cursor)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for name in ["gap_lo", "gap_hi"] {
        if !columns.iter().any(|c| c == name) {
            conn.execute(
                &format!("ALTER TABLE sync_cursor ADD COLUMN {name} INTEGER"),
                [],
            )?;
        }
    }
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS sync_applied_version (
            peer_device_id TEXT NOT NULL,
            row_type TEXT NOT NULL,
            source_id INTEGER NOT NULL,
            origin_epoch TEXT NOT NULL DEFAULT '',
            local_seq INTEGER NOT NULL,
            PRIMARY KEY (peer_device_id, row_type, source_id)
        );",
    )
}

impl Store {
    /// The peer rows still to pull below the live range, as `(gap_lo, gap_hi)`, both
    /// exclusive; `None` when no gap is open.
    pub fn sync_gap(&self, peer_device_id: &str) -> rusqlite::Result<Option<(i64, i64)>> {
        Ok(self
            .conn
            .query_row(
                "SELECT gap_lo, gap_hi FROM sync_cursor WHERE peer_device_id = ?1",
                params![peer_device_id],
                |r| Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, Option<i64>>(1)?)),
            )
            .optional()?
            .and_then(|(lo, hi)| lo.zip(hi)))
    }

    /// Records the gap `(gap_lo, gap_hi)` and moves the live cursor to `live_cursor`, in one
    /// transaction: the rows above the old cursor are then either in the gap or above the
    /// new cursor, never neither.
    pub fn open_sync_gap(
        &self,
        peer_device_id: &str,
        gap_lo: i64,
        gap_hi: i64,
        live_cursor: i64,
    ) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO sync_cursor (peer_device_id, last_pulled_seq, gap_lo, gap_hi) VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(peer_device_id) DO UPDATE SET last_pulled_seq = excluded.last_pulled_seq, \
             gap_lo = excluded.gap_lo, gap_hi = excluded.gap_hi",
            params![peer_device_id, live_cursor, gap_lo, gap_hi],
        )?;
        tx.commit()
    }

    /// Moves an open gap's upper bound down to `gap_hi` (the lowest row a gap page handled).
    pub fn set_sync_gap_hi(&self, peer_device_id: &str, gap_hi: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE sync_cursor SET gap_hi = ?2 WHERE peer_device_id = ?1 AND gap_hi IS NOT NULL",
            params![peer_device_id, gap_hi],
        )?;
        Ok(())
    }

    /// The gap is pulled: every row of the peer below the live cursor is handled.
    pub fn close_sync_gap(&self, peer_device_id: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE sync_cursor SET gap_lo = NULL, gap_hi = NULL WHERE peer_device_id = ?1",
            params![peer_device_id],
        )?;
        Ok(())
    }

    /// Every peer with an open gap.
    pub fn sync_gap_peers(&self) -> rusqlite::Result<Vec<String>> {
        self.conn
            .prepare("SELECT peer_device_id FROM sync_cursor WHERE gap_hi IS NOT NULL ORDER BY peer_device_id")?
            .query_map([], |r| r.get(0))?
            .collect()
    }

    /// Whether older history is still loading: some peer's gap is open. Persisted, so it
    /// is right after a relaunch or a failed round too.
    pub fn older_sync_history_loading(&self) -> rusqlite::Result<bool> {
        self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM sync_cursor WHERE gap_hi IS NOT NULL)",
            [],
            |r| r.get(0),
        )
    }

    /// Whether a newer version of the peer's `row_type` entity `source_id` than `local_seq`
    /// is already stored here, so this one must not overwrite it. The relay's local_seq decides,
    /// whatever the origin epoch: that is what an ascending pull keeps (the higher seq applies
    /// last), so the result is the same in either order, also across a re-created peer
    /// database. Nothing recorded (applied before this bookkeeping existed) is older than
    /// anything. `origin_epoch` is kept for diagnosis only.
    pub fn newer_sync_version_applied(
        &self,
        peer_device_id: &str,
        row_type: &str,
        source_id: i64,
        origin_epoch: Option<&str>,
        local_seq: i64,
    ) -> rusqlite::Result<bool> {
        let _ = origin_epoch;
        let stored: Option<i64> = self
            .conn
            .query_row(
                "SELECT local_seq FROM sync_applied_version \
                 WHERE peer_device_id = ?1 AND row_type = ?2 AND source_id = ?3",
                params![peer_device_id, row_type, source_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(stored.is_some_and(|stored_seq| stored_seq > local_seq))
    }

    /// Records that the stored version of the peer's `row_type` entity `source_id` is the one
    /// pushed at `local_seq` (call only after applying it). An unknown epoch keeps a known one.
    pub fn record_sync_version(
        &self,
        peer_device_id: &str,
        row_type: &str,
        source_id: i64,
        origin_epoch: Option<&str>,
        local_seq: i64,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO sync_applied_version (peer_device_id, row_type, source_id, origin_epoch, local_seq) \
             VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(peer_device_id, row_type, source_id) DO UPDATE SET \
             local_seq = excluded.local_seq, \
             origin_epoch = CASE WHEN excluded.origin_epoch = '' THEN origin_epoch ELSE excluded.origin_epoch END",
            params![peer_device_id, row_type, source_id, origin_epoch.unwrap_or(""), local_seq],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::Store;

    #[test]
    fn a_gap_is_recorded_with_the_live_cursor_and_closes() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.sync_gap("mac").unwrap(), None);
        assert!(!store.older_sync_history_loading().unwrap());

        store.set_sync_cursor("mac", 40).unwrap();
        store.open_sync_gap("mac", 40, 501, 500).unwrap();
        assert_eq!(store.get_sync_cursor("mac").unwrap(), 500);
        assert_eq!(store.sync_gap("mac").unwrap(), Some((40, 501)));
        assert!(store.older_sync_history_loading().unwrap());

        // The live cursor moves on its own; the gap stays.
        store.set_sync_cursor("mac", 510).unwrap();
        assert_eq!(store.sync_gap("mac").unwrap(), Some((40, 501)));

        store.set_sync_gap_hi("mac", 300).unwrap();
        assert_eq!(store.sync_gap("mac").unwrap(), Some((40, 300)));

        store.open_sync_gap("pc", 0, 3, 2).unwrap();
        assert_eq!(store.sync_gap_peers().unwrap(), vec!["mac", "pc"]);
        store.close_sync_gap("pc").unwrap();
        assert_eq!(store.sync_gap_peers().unwrap(), vec!["mac"]);

        store.close_sync_gap("mac").unwrap();
        assert_eq!(store.sync_gap("mac").unwrap(), None);
        assert!(store.sync_gap_peers().unwrap().is_empty());
        assert_eq!(store.get_sync_cursor("mac").unwrap(), 510);
        assert!(!store.older_sync_history_loading().unwrap());

        // A fresh peer: the gap row is created with the cursor.
        store.open_sync_gap("pc", 0, 8, 7).unwrap();
        assert_eq!(store.get_sync_cursor("pc").unwrap(), 7);
        assert_eq!(store.sync_gap("pc").unwrap(), Some((0, 8)));
        // Moving the upper bound of a closed gap never reopens it.
        store.set_sync_gap_hi("mac", 5).unwrap();
        assert_eq!(store.sync_gap("mac").unwrap(), None);
    }

    #[test]
    fn an_old_sync_cursor_table_gets_the_gap_columns() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.sqlite");
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE sync_cursor (peer_device_id TEXT PRIMARY KEY, last_pulled_seq INTEGER NOT NULL DEFAULT 0);
                 INSERT INTO sync_cursor VALUES ('mac', 77);",
            )
            .unwrap();
        }
        let store = Store::open(&path, std::path::Path::new("")).unwrap();
        assert_eq!(store.get_sync_cursor("mac").unwrap(), 77);
        assert_eq!(
            store.sync_gap("mac").unwrap(),
            None,
            "an upgrade opens no gap"
        );
        drop(store);
        // Opening again is a no-op.
        let store = Store::open(&path, std::path::Path::new("")).unwrap();
        assert_eq!(store.get_sync_cursor("mac").unwrap(), 77);
    }

    #[test]
    fn an_older_version_is_recognized_by_relay_seq() {
        let store = Store::open_in_memory().unwrap();
        let newer = |epoch: Option<&str>, seq| {
            store
                .newer_sync_version_applied("mac", "meeting", 9, epoch, seq)
                .unwrap()
        };
        assert!(
            !newer(Some("e1"), 1),
            "nothing recorded is older than anything"
        );

        store
            .record_sync_version("mac", "meeting", 9, Some("e1"), 50)
            .unwrap();
        assert!(newer(Some("e1"), 49));
        assert!(newer(None, 49), "an older build's row compares by seq");
        assert!(!newer(Some("e1"), 50), "the same row again re-applies");
        assert!(!newer(Some("e1"), 51));
        assert!(
            newer(Some("e2"), 3),
            "seq decides across a re-created database too, as an ascending pull would"
        );
        assert!(!newer(Some("e2"), 51));
        // Other entities, types and peers are separate.
        for (peer, row_type, id) in [
            ("mac", "meeting", 10),
            ("mac", "routine_run", 9),
            ("pc", "meeting", 9),
        ] {
            assert!(!store
                .newer_sync_version_applied(peer, row_type, id, Some("e1"), 1)
                .unwrap());
        }
    }
}
