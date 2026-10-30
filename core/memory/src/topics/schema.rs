//! Topic tables: sources, entities, topics and membership. `foreign_keys` is OFF in this
//! codebase, so cleanup is done by triggers (source gone => its rows gone) and by
//! [`prune_orphans`] (entities with no mentions, topics with no member rows).
//! Retention keeps a source as a stub (`ref_id = NULL, stub = 1`) so memberships survive;
//! manual deletes call nothing and the triggers wipe the source.

use rusqlite::{params, Connection};

use super::index::name_key;
use crate::EMBEDDING_DIM;

/// Set (by triggers and re-indexing) when rows were deleted that may leave orphans;
/// `topic_cluster` prunes only then.
pub(crate) const PRUNE_DIRTY_KEY: &str = "topic_prune_dirty";

/// Called at the end of `Store::init`, after every `ALTER TABLE` the triggers depend on
/// (`meeting.summary_tldr`, `episodes.hidden`).
pub(crate) fn ensure_topic_tables(conn: &Connection) -> rusqlite::Result<()> {
    // One write transaction: two connections opening at once (the app, its slow lane, the
    // MCP server) would otherwise interleave the DROP TRIGGER / CREATE TRIGGER pairs below,
    // and the second create fails with "trigger ... already exists". Rolled back on error.
    let tx = rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)?;
    tx.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS topic_source (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            kind TEXT NOT NULL,
            ref_id INTEGER,
            ts_unix_ms INTEGER NOT NULL,
            title TEXT NOT NULL DEFAULT '',
            stub INTEGER NOT NULL DEFAULT 0,
            indexed_at_unix_ms INTEGER NOT NULL,
            assigned_at_unix_ms INTEGER
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_topic_source_ref
            ON topic_source(kind, ref_id) WHERE ref_id IS NOT NULL;
        CREATE VIRTUAL TABLE IF NOT EXISTS topic_source_vec USING vec0(embedding float[{EMBEDDING_DIM}]);
        CREATE TABLE IF NOT EXISTS entity (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            kind TEXT NOT NULL,
            key TEXT NOT NULL,
            display TEXT NOT NULL,
            updated_at_unix_ms INTEGER NOT NULL,
            -- `name:` key form of an email person's real display name (NULL otherwise), so a
            -- name-only attendee finds them without loading every person.
            name_key TEXT,
            UNIQUE(kind, key)
        );
        CREATE TABLE IF NOT EXISTS entity_mention (
            entity_id INTEGER NOT NULL,
            source_id INTEGER NOT NULL,
            weight REAL NOT NULL DEFAULT 1.0,
            PRIMARY KEY (entity_id, source_id)
        );
        CREATE INDEX IF NOT EXISTS idx_entity_mention_source ON entity_mention(source_id);
        CREATE TABLE IF NOT EXISTS topic (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL DEFAULT '',
            name_is_user INTEGER NOT NULL DEFAULT 0,
            name_dirty INTEGER NOT NULL DEFAULT 1,
            status_line TEXT,
            merged_into INTEGER,
            created_at_unix_ms INTEGER NOT NULL,
            updated_at_unix_ms INTEGER NOT NULL,
            last_member_ts_unix_ms INTEGER
        );
        CREATE TABLE IF NOT EXISTS topic_member (
            topic_id INTEGER NOT NULL,
            source_id INTEGER NOT NULL,
            origin TEXT NOT NULL,
            score REAL NOT NULL DEFAULT 0,
            PRIMARY KEY (topic_id, source_id)
        );
        CREATE INDEX IF NOT EXISTS idx_topic_member_source ON topic_member(source_id);
        CREATE INDEX IF NOT EXISTS idx_topic_source_unassigned
            ON topic_source(ts_unix_ms, id) WHERE assigned_at_unix_ms IS NULL;
        -- topic_cluster's cache: sum of non-removed member vectors (f32 LE blob, NULL when
        -- none) and how many. `named_members` is the member count when the topic was last
        -- named; the naming job writes it when it clears `name_dirty`.
        CREATE TABLE IF NOT EXISTS topic_stats (
            topic_id INTEGER PRIMARY KEY,
            vec_n INTEGER NOT NULL DEFAULT 0,
            vec_sum BLOB,
            named_members INTEGER NOT NULL DEFAULT 0
        );
        -- Idea topics remember the phrase that seeded them, so later mentions join them.
        CREATE TABLE IF NOT EXISTS topic_idea (
            entity_id INTEGER PRIMARY KEY,
            topic_id INTEGER NOT NULL
        );
        -- Meetings and episodes edited after indexing; topic_index re-indexes and clears them.
        -- Trigger inserts use WHERE NOT EXISTS, not OR IGNORE: an outer UPSERT's conflict
        -- handling overrides a trigger's.
        CREATE TABLE IF NOT EXISTS topic_dirty (
            kind TEXT NOT NULL,
            ref_id INTEGER NOT NULL,
            PRIMARY KEY (kind, ref_id)
        ) WITHOUT ROWID;
        DROP TRIGGER IF EXISTS topic_source_items_ad;
        CREATE TRIGGER topic_source_items_ad AFTER DELETE ON items
        WHEN old.kind IN ('email', 'calendar_event', 'snapshot') BEGIN
            DELETE FROM topic_source WHERE kind = old.kind AND ref_id = old.id;
        END;
        CREATE TRIGGER IF NOT EXISTS topic_source_meeting_ad AFTER DELETE ON meeting BEGIN
            DELETE FROM topic_source WHERE kind = 'meeting' AND ref_id = old.id;
        END;
        CREATE TRIGGER IF NOT EXISTS topic_source_episodes_ad AFTER DELETE ON episodes BEGIN
            DELETE FROM topic_source WHERE kind = 'episode' AND ref_id = old.id;
        END;
        -- A deleted source (never a retention stub, which is an UPDATE) changes what its
        -- topics are about: auto names are redone, and orphans may be left to prune.
        DROP TRIGGER IF EXISTS topic_source_ad;
        CREATE TRIGGER topic_source_ad AFTER DELETE ON topic_source BEGIN
            UPDATE topic SET name_dirty = 1
             WHERE name_is_user = 0
               AND id IN (SELECT topic_id FROM topic_member WHERE source_id = old.id);
            INSERT INTO kv_meta (key, value) SELECT '{PRUNE_DIRTY_KEY}', '1'
             WHERE NOT EXISTS (SELECT 1 FROM kv_meta WHERE key = '{PRUNE_DIRTY_KEY}');
            DELETE FROM entity_mention WHERE source_id = old.id;
            DELETE FROM topic_member WHERE source_id = old.id;
            DELETE FROM topic_source_vec WHERE rowid = old.id;
        END;
        -- Any membership change made outside topic_cluster (a deleted source, a removed or
        -- user-added member) marks the topic's cached centroid stale (vec_n = -1);
        -- topic_cluster rebuilds it on its next load. named_members is kept.
        -- A re-indexed source (edited meeting, re-summarized episode) has new text and a new
        -- vector: its topics' centroids go stale and it is queued to re-cluster.
        CREATE TRIGGER IF NOT EXISTS topic_source_reindexed
        AFTER UPDATE OF indexed_at_unix_ms ON topic_source
        WHEN new.indexed_at_unix_ms IS NOT old.indexed_at_unix_ms BEGIN
            UPDATE topic_stats SET vec_n = -1, vec_sum = NULL
             WHERE topic_id IN (SELECT topic_id FROM topic_member WHERE source_id = new.id);
            UPDATE topic_source SET assigned_at_unix_ms = NULL WHERE id = new.id;
        END;
        CREATE TRIGGER IF NOT EXISTS topic_member_ad AFTER DELETE ON topic_member BEGIN
            UPDATE topic_stats SET vec_n = -1, vec_sum = NULL WHERE topic_id = old.topic_id;
        END;
        CREATE TRIGGER IF NOT EXISTS topic_member_au AFTER UPDATE OF origin, topic_id ON topic_member BEGIN
            UPDATE topic_stats SET vec_n = -1, vec_sum = NULL
             WHERE topic_id IN (old.topic_id, new.topic_id);
        END;
        CREATE TRIGGER IF NOT EXISTS topic_member_ai AFTER INSERT ON topic_member
        WHEN new.origin != 'auto' BEGIN
            UPDATE topic_stats SET vec_n = -1, vec_sum = NULL WHERE topic_id = new.topic_id;
        END;
        CREATE TRIGGER IF NOT EXISTS topic_dirty_meeting
        AFTER UPDATE OF end_unix_ms, title, attendees, summary_tldr ON meeting BEGIN
            INSERT INTO topic_dirty SELECT 'meeting', new.id
             WHERE NOT EXISTS (SELECT 1 FROM topic_dirty WHERE kind = 'meeting' AND ref_id = new.id);
        END;
        CREATE TRIGGER IF NOT EXISTS topic_dirty_note_ai AFTER INSERT ON meeting_note_draft BEGIN
            INSERT INTO topic_dirty SELECT 'meeting', new.meeting_id
             WHERE NOT EXISTS (SELECT 1 FROM topic_dirty WHERE kind = 'meeting' AND ref_id = new.meeting_id);
        END;
        CREATE TRIGGER IF NOT EXISTS topic_dirty_note_au
        AFTER UPDATE OF summary_edited ON meeting_note_draft BEGIN
            INSERT INTO topic_dirty SELECT 'meeting', new.meeting_id
             WHERE NOT EXISTS (SELECT 1 FROM topic_dirty WHERE kind = 'meeting' AND ref_id = new.meeting_id);
        END;
        CREATE TRIGGER IF NOT EXISTS topic_dirty_action_ai AFTER INSERT ON meeting_action_item BEGIN
            INSERT INTO topic_dirty SELECT 'meeting', new.meeting_id
             WHERE NOT EXISTS (SELECT 1 FROM topic_dirty WHERE kind = 'meeting' AND ref_id = new.meeting_id);
        END;
        CREATE TRIGGER IF NOT EXISTS topic_dirty_action_au
        AFTER UPDATE OF text ON meeting_action_item BEGIN
            INSERT INTO topic_dirty SELECT 'meeting', new.meeting_id
             WHERE NOT EXISTS (SELECT 1 FROM topic_dirty WHERE kind = 'meeting' AND ref_id = new.meeting_id);
        END;
        CREATE TRIGGER IF NOT EXISTS topic_dirty_action_ad AFTER DELETE ON meeting_action_item BEGIN
            INSERT INTO topic_dirty SELECT 'meeting', old.meeting_id
             WHERE NOT EXISTS (SELECT 1 FROM topic_dirty WHERE kind = 'meeting' AND ref_id = old.meeting_id);
        END;
        CREATE TRIGGER IF NOT EXISTS topic_dirty_episode
        AFTER UPDATE OF status, summary, window_title, hidden ON episodes BEGIN
            INSERT INTO topic_dirty SELECT 'episode', new.id
             WHERE NOT EXISTS (SELECT 1 FROM topic_dirty WHERE kind = 'episode' AND ref_id = new.id);
        END;"
    ))?;
    tx.commit()?;
    ensure_entity_name_key(conn)
}

/// Adds and backfills `entity.name_key` on databases created before it existed.
fn ensure_entity_name_key(conn: &Connection) -> rusqlite::Result<()> {
    let has: bool = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('entity') WHERE name = 'name_key'",
        [],
        |r| r.get(0),
    )?;
    if !has {
        conn.execute("ALTER TABLE entity ADD COLUMN name_key TEXT", [])?;
        let rows: Vec<(i64, String)> = {
            let mut stmt = conn.prepare(
                "SELECT id, display FROM entity \
                 WHERE kind = 'person' AND key LIKE 'email:%' AND display != substr(key, 7)",
            )?;
            let rows = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            rows
        };
        for (id, display) in rows {
            conn.execute(
                "UPDATE entity SET name_key = ?1 WHERE id = ?2",
                params![name_key(&display), id],
            )?;
        }
    }
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_entity_name_key ON entity(name_key) WHERE name_key IS NOT NULL",
        [],
    )?;
    Ok(())
}

/// Deletes entities with no mentions and topics with no member rows of any origin.
/// A topic whose members are all `removed` tombstones is kept (hidden from lists elsewhere).
pub(crate) fn prune_orphans(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "DELETE FROM entity WHERE id NOT IN (SELECT entity_id FROM entity_mention);
         DELETE FROM topic WHERE id NOT IN (SELECT topic_id FROM topic_member);
         DELETE FROM topic_stats WHERE topic_id NOT IN (SELECT id FROM topic);
         DELETE FROM topic_idea WHERE topic_id NOT IN (SELECT id FROM topic)
            OR entity_id NOT IN (SELECT id FROM entity);",
    )
}

/// Turns the sources for `ref_ids` of `kind` into stubs: `ref_id = NULL, stub = 1`, title and
/// ts kept, so memberships outlive the deleted content. Returns the number stubbed.
pub(crate) fn stub_sources_for_retention(
    conn: &Connection,
    kind: &str,
    ref_ids: &[i64],
) -> rusqlite::Result<usize> {
    let mut stmt = conn.prepare_cached(
        "UPDATE topic_source SET ref_id = NULL, stub = 1 WHERE kind = ?1 AND ref_id = ?2",
    )?;
    let mut n = 0;
    for id in ref_ids {
        n += stmt.execute(params![kind, id])?;
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        MeetingStatus, MeetingTrigger, NewMeeting, NewMeetingSnapshot, RetentionPolicy, Store,
    };

    const DAY: i64 = 86_400_000;

    fn count(s: &Store, sql: &str) -> i64 {
        s.conn.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    fn add_source(s: &Store, kind: &str, ref_id: i64, title: &str, ts: i64) -> i64 {
        s.conn
            .execute(
                "INSERT INTO topic_source (kind, ref_id, ts_unix_ms, title, indexed_at_unix_ms) VALUES (?1, ?2, ?3, ?4, 0)",
                params![kind, ref_id, ts, title],
            )
            .unwrap();
        s.conn.last_insert_rowid()
    }

    /// One entity mention, one topic, one membership for `source_id`.
    fn attach(s: &Store, source_id: i64) {
        s.conn
            .execute_batch(&format!(
                "INSERT INTO entity (kind, key, display, updated_at_unix_ms) VALUES ('person', 'a{source_id}', 'A', 0);
                 INSERT INTO entity_mention (entity_id, source_id, weight) VALUES (last_insert_rowid(), {source_id}, 1.0);
                 INSERT INTO topic (name, created_at_unix_ms, updated_at_unix_ms) VALUES ('t', 0, 0);
                 INSERT INTO topic_member (topic_id, source_id, origin, score) VALUES (last_insert_rowid(), {source_id}, 'auto', 1.0);
                 INSERT INTO topic_source_vec (rowid, embedding) VALUES ({source_id}, zeroblob({EMBEDDING_DIM} * 4));"
            ))
            .unwrap();
    }

    fn meeting(s: &Store, start: i64) -> i64 {
        s.insert_meeting(&NewMeeting {
            device_id: "d".into(),
            start_unix_ms: start,
            end_unix_ms: Some(start + 1000),
            title: "m".into(),
            attendees: vec![],
            trigger: MeetingTrigger::Manual,
            audio_path: None,
            status: MeetingStatus::Completed,
        })
        .unwrap()
    }

    #[test]
    fn init_is_idempotent_with_topic_tables() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("m.db");
        drop(Store::open(&p, dir.path()).unwrap());
        let s = Store::open(&p, dir.path()).unwrap();
        assert_eq!(count(&s, "SELECT COUNT(*) FROM topic_source"), 0);
    }

    #[test]
    fn manual_item_delete_leaves_nothing() {
        let s = Store::open_in_memory().unwrap();
        s.conn
            .execute(
                "INSERT INTO items (device_id, kind, source, ts_unix_ms, body_text, external_id) VALUES ('d','email','gmail',1,'b','x1')",
                [],
            )
            .unwrap();
        let item_id = s.conn.last_insert_rowid();
        let sid = add_source(&s, "email", item_id, "hi", 1);
        attach(&s, sid);
        s.delete_email_items(&["x1".to_string()]).unwrap();
        prune_orphans(&s.conn).unwrap();
        for t in [
            "topic_source",
            "entity_mention",
            "topic_member",
            "topic_source_vec",
            "entity",
            "topic",
        ] {
            assert_eq!(count(&s, &format!("SELECT COUNT(*) FROM {t}")), 0, "{t}");
        }
    }

    #[test]
    fn delete_meeting_wipes_its_source() {
        let s = Store::open_in_memory().unwrap();
        let m = meeting(&s, 1000);
        let sid = add_source(&s, "meeting", m, "m", 1000);
        attach(&s, sid);
        assert!(s.delete_meeting(m).unwrap());
        assert_eq!(count(&s, "SELECT COUNT(*) FROM topic_source"), 0);
        assert_eq!(count(&s, "SELECT COUNT(*) FROM topic_member"), 0);
    }

    #[test]
    fn retention_leaves_a_stub() {
        let s = Store::open_in_memory().unwrap();
        let m = meeting(&s, 1000);
        let snap_item: i64 = {
            s.insert_meeting_snapshot(&NewMeetingSnapshot {
                meeting_id: m,
                captured_unix_ms: 1100,
                file_path: "/x.png".into(),
                width: 1,
                height: 1,
                ocr_text: "slide text".into(),
            })
            .unwrap()
            .unwrap();
            count(&s, "SELECT id FROM items WHERE kind = 'snapshot'")
        };
        let ms = add_source(&s, "meeting", m, "Standup", 1000);
        let ss = add_source(&s, "snapshot", snap_item, "Slide", 1100);
        attach(&s, ms);
        attach(&s, ss);
        s.apply_retention(
            &RetentionPolicy {
                capture_item_days: None,
                meeting_days: Some(1),
                conversation_days: None,
            },
            1000 + 10 * DAY,
        )
        .unwrap();
        assert_eq!(count(&s, "SELECT COUNT(*) FROM meeting"), 0);
        assert_eq!(
            count(&s, "SELECT COUNT(*) FROM items WHERE kind = 'snapshot'"),
            0
        );
        assert_eq!(
            count(&s, "SELECT COUNT(*) FROM topic_source WHERE stub = 1 AND ref_id IS NULL AND title IN ('Standup','Slide') AND ts_unix_ms IN (1000, 1100)"),
            2
        );
        assert_eq!(count(&s, "SELECT COUNT(*) FROM topic_member"), 2);
        assert_eq!(count(&s, "SELECT COUNT(*) FROM entity_mention"), 2);
    }

    #[test]
    fn prune_removes_orphan_entities_and_empty_topics() {
        let s = Store::open_in_memory().unwrap();
        s.conn
            .execute_batch(
                "INSERT INTO entity (kind, key, display, updated_at_unix_ms) VALUES ('person','z','Z',0);
                 INSERT INTO topic (name, created_at_unix_ms, updated_at_unix_ms) VALUES ('t',0,0);",
            )
            .unwrap();
        prune_orphans(&s.conn).unwrap();
        assert_eq!(count(&s, "SELECT COUNT(*) FROM entity"), 0);
        assert_eq!(count(&s, "SELECT COUNT(*) FROM topic"), 0);
    }

    #[test]
    fn deleting_a_source_marks_auto_named_topics_for_renaming() {
        let s = Store::open_in_memory().unwrap();
        let a = add_source(&s, "meeting", 1, "a", 1);
        let b = add_source(&s, "meeting", 2, "b", 2);
        s.conn
            .execute_batch(&format!(
                "INSERT INTO topic (id, name, name_dirty, name_is_user, created_at_unix_ms, updated_at_unix_ms)
                   VALUES (1,'auto',0,0,0,0), (2,'mine',0,1,0,0), (3,'stubbed',0,0,0,0);
                 INSERT INTO topic_member (topic_id, source_id, origin)
                   VALUES (1,{a},'auto'), (2,{a},'auto'), (3,{b},'auto');
                 DELETE FROM kv_meta WHERE key = 'topic_prune_dirty';"
            ))
            .unwrap();
        // Retention keeps a stub: nothing was deleted, names stay.
        stub_sources_for_retention(&s.conn, "meeting", &[2]).unwrap();
        let flag = "SELECT COUNT(*) FROM kv_meta WHERE key = 'topic_prune_dirty'";
        assert_eq!(count(&s, flag), 0);
        s.conn
            .execute("DELETE FROM topic_source WHERE id = ?1", [a])
            .unwrap();
        assert_eq!(
            count(
                &s,
                "SELECT CAST(group_concat(name_dirty, '') AS INTEGER) FROM (SELECT name_dirty FROM topic ORDER BY id)"
            ),
            100,
            "auto-named topic only"
        );
        assert_eq!(count(&s, flag), 1);
    }

    #[test]
    fn changed_triggers_replace_old_ones_and_items_trigger_filters_kind() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("m.db");
        {
            let s = Store::open(&p, dir.path()).unwrap();
            s.conn
                .execute_batch(
                    "DROP TRIGGER topic_source_items_ad;
                     CREATE TRIGGER topic_source_items_ad AFTER DELETE ON items BEGIN SELECT 1; END;",
                )
                .unwrap();
        }
        let s = Store::open(&p, dir.path()).unwrap();
        let sql: String = s
            .conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name = 'topic_source_items_ad'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            sql.contains("WHEN old.kind IN ('email', 'calendar_event', 'snapshot')"),
            "{sql}"
        );
    }

    #[test]
    fn prune_keeps_tombstone_only_topics() {
        let s = Store::open_in_memory().unwrap();
        s.conn
            .execute_batch(
                "INSERT INTO topic (name, created_at_unix_ms, updated_at_unix_ms) VALUES ('t',0,0);
                 INSERT INTO topic_member (topic_id, source_id, origin, score) VALUES (1, 99, 'removed', 0);",
            )
            .unwrap();
        prune_orphans(&s.conn).unwrap();
        assert_eq!(count(&s, "SELECT COUNT(*) FROM topic"), 1);
        assert_eq!(count(&s, "SELECT COUNT(*) FROM topic_member"), 1);
    }
}
