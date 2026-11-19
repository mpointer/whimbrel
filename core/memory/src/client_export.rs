//! The reads and the local record behind "Export everything about this client" (docs/superpowers/specs/
//! 2026-11-12-client-controls-design.md section 6). `api::client_export` drives them.
//!
//! `client_export` records where each export folder went, so a later client delete can list and offer to
//! remove it. It holds the folder's path, when it was written and an opaque hash of the client's key (so one
//! client's delete never reaches another client's exports); never the client's name or any of its text. It is
//! local only: never synced.

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::client_forget::{hashed, merge_chain};
use crate::{ExportItemRow, Store};

pub(crate) fn ensure_client_export_table(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS client_export (
            path TEXT PRIMARY KEY,
            created_at_unix_ms INTEGER NOT NULL,
            client TEXT NOT NULL
        ) WITHOUT ROWID;
        CREATE INDEX IF NOT EXISTS idx_client_export_client ON client_export(client);",
    )
}

/// The opaque owner id of client `key`'s export rows.
fn owner(key: &str) -> String {
    hashed(&format!("client_export:{key}"))
}

/// Drops the export records of seed `key` (its client is purged; the folders stay the user's).
pub(crate) fn forget_client_exports_tx(conn: &Connection, key: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM client_export WHERE client = ?1", [owner(key)])?;
    Ok(())
}

/// One step of a client, as exported.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StepExportRow {
    pub key: String,
    pub source_kind: String,
    pub source_title: String,
    pub text: String,
    pub direction: String,
    pub asked_at_unix_ms: i64,
    pub due_date: Option<String>,
    pub status: String,
    pub confirmed: bool,
    pub note: Option<String>,
    pub closed_by: Option<String>,
    pub updated_at_unix_ms: i64,
}

/// One screen activity episode, as exported.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EpisodeExportRow {
    pub id: i64,
    pub device_id: String,
    pub app_bundle_id: String,
    pub window_title: Option<String>,
    pub start_ts_unix_ms: i64,
    pub end_ts_unix_ms: i64,
    pub item_count: i64,
    pub summary: Option<String>,
}

/// A meeting's media on disk, as its rows name them (not checked here).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MeetingMediaPaths {
    /// The meeting's and its parts' audio files.
    pub audio: Vec<String>,
    /// The slide image folder.
    pub slides_dir: Option<String>,
}

impl Store {
    /// Records that an export of client `key` was written to `path` (an absolute, canonical folder path).
    pub fn record_client_export(&self, key: &str, path: &str, at_unix_ms: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO client_export (path, created_at_unix_ms, client) VALUES (?1, ?2, ?3) \
             ON CONFLICT(path) DO UPDATE SET created_at_unix_ms = excluded.created_at_unix_ms, client = excluded.client",
            params![path, at_unix_ms, owner(key)],
        )?;
        Ok(())
    }

    /// The export folders recorded for client `key` and every seed merged into it, oldest first.
    pub fn client_exports(&self, key: &str) -> rusqlite::Result<Vec<String>> {
        let mut out: Vec<(i64, String)> = Vec::new();
        let mut stmt = self.conn.prepare("SELECT created_at_unix_ms, path FROM client_export WHERE client = ?1")?;
        for k in merge_chain(&self.conn, key)? {
            for row in stmt.query_map([owner(&k)], |r| Ok((r.get(0)?, r.get(1)?)))? {
                out.push(row?);
            }
        }
        out.sort();
        out.dedup();
        Ok(out.into_iter().map(|(_, p)| p).collect())
    }

    /// Whether `path` is a recorded export folder (of any client).
    pub fn is_client_export(&self, path: &str) -> rusqlite::Result<bool> {
        self.conn.query_row("SELECT EXISTS (SELECT 1 FROM client_export WHERE path = ?1)", [path], |r| r.get(0))
    }

    /// Drops the record of export folder `path` (it is gone).
    pub fn forget_client_export(&self, path: &str) -> rusqlite::Result<()> {
        self.conn.execute("DELETE FROM client_export WHERE path = ?1", [path])?;
        Ok(())
    }

    /// The `items` row `id` of any kind (a removed calendar event too), shaped as "Export everything" writes it.
    pub fn export_item_row(&self, id: i64) -> rusqlite::Result<Option<ExportItemRow>> {
        self.conn
            .query_row(
                "SELECT id, device_id, kind, source, ts_unix_ms, app_bundle_id, title, url, actors, body_text, refs, external_id \
                 FROM items WHERE id = ?1",
                [id],
                |row| {
                    let actors_json: Option<String> = row.get(8)?;
                    Ok(ExportItemRow {
                        id: row.get(0)?,
                        device_id: row.get(1)?,
                        kind: row.get(2)?,
                        source: row.get(3)?,
                        ts_unix_ms: row.get(4)?,
                        app_bundle_id: row.get(5)?,
                        title: row.get(6)?,
                        url: row.get(7)?,
                        actors: actors_json.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default(),
                        body_text: row.get(9)?,
                        refs: row.get(10)?,
                        external_id: row.get(11)?,
                    })
                },
            )
            .optional()
    }

    /// A capture held only in `capture_item` (never mirrored to `items`), shaped as an export row: `id` is
    /// the `capture_item.id`, `source` is `capture_item`.
    pub fn capture_item_export_row(&self, capture_item_id: i64) -> rusqlite::Result<Option<ExportItemRow>> {
        self.conn
            .query_row(
                "SELECT id, device_id, ts_unix_ms, app_bundle_id, window_title, url, text FROM capture_item WHERE id = ?1",
                [capture_item_id],
                |r| {
                    Ok(ExportItemRow {
                        id: r.get(0)?,
                        device_id: r.get(1)?,
                        kind: "capture".to_string(),
                        source: Some("capture_item".to_string()),
                        ts_unix_ms: r.get(2)?,
                        app_bundle_id: r.get(3)?,
                        title: r.get(4)?,
                        url: r.get(5)?,
                        actors: Vec::new(),
                        body_text: r.get(6)?,
                        refs: None,
                        external_id: None,
                    })
                },
            )
            .optional()
    }

    /// Step `key`, as exported.
    pub fn step_export_row(&self, key: &str) -> rusqlite::Result<Option<StepExportRow>> {
        self.conn
            .query_row(
                "SELECT key, source_kind, source_title, text, direction, asked_at_unix_ms, due_date, status, \
                 confirmed, note, closed_by, updated_at_unix_ms FROM topic_step WHERE key = ?1",
                [key],
                |r| {
                    Ok(StepExportRow {
                        key: r.get(0)?,
                        source_kind: r.get(1)?,
                        source_title: r.get(2)?,
                        text: r.get(3)?,
                        direction: r.get(4)?,
                        asked_at_unix_ms: r.get(5)?,
                        due_date: r.get(6)?,
                        status: r.get(7)?,
                        confirmed: r.get(8)?,
                        note: r.get(9)?,
                        closed_by: r.get(10)?,
                        updated_at_unix_ms: r.get(11)?,
                    })
                },
            )
            .optional()
    }

    /// Episode `id`, as exported.
    pub fn episode_export_row(&self, id: i64) -> rusqlite::Result<Option<EpisodeExportRow>> {
        self.conn
            .query_row(
                "SELECT id, device_id, app_bundle_id, window_title, start_ts_unix_ms, end_ts_unix_ms, item_count, summary \
                 FROM episodes WHERE id = ?1",
                [id],
                |r| {
                    Ok(EpisodeExportRow {
                        id: r.get(0)?,
                        device_id: r.get(1)?,
                        app_bundle_id: r.get(2)?,
                        window_title: r.get(3)?,
                        start_ts_unix_ms: r.get(4)?,
                        end_ts_unix_ms: r.get(5)?,
                        item_count: r.get(6)?,
                        summary: r.get(7)?,
                    })
                },
            )
            .optional()
    }

    /// The audio files and slide folder meeting `id`'s rows name.
    pub fn meeting_media_paths(&self, id: i64) -> rusqlite::Result<MeetingMediaPaths> {
        let row: Option<(Option<String>, Option<String>)> = self
            .conn
            .query_row("SELECT audio_path, snapshots_dir FROM meeting WHERE id = ?1", [id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        let Some((audio, slides_dir)) = row else { return Ok(MeetingMediaPaths::default()) };
        let mut out = MeetingMediaPaths { audio: audio.into_iter().collect(), slides_dir };
        let mut stmt = self.conn.prepare("SELECT audio_path FROM meeting_part WHERE meeting_id = ?1")?;
        for p in stmt.query_map([id], |r| r.get::<_, Option<String>>(0))? {
            out.audio.extend(p?);
        }
        out.audio.sort();
        out.audio.dedup();
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exports_are_kept_per_client_and_follow_merged_seeds() {
        let store = Store::open_in_memory().unwrap();
        let a = store.create_seed_topic("Acme", &[], None).unwrap();
        let b = store.create_seed_topic("Beta", &[], None).unwrap();
        let g = store.create_seed_topic("Globex", &[], None).unwrap();
        store.record_client_export(&a.key, "/x/1", 1).unwrap();
        store.record_client_export(&b.key, "/x/2", 2).unwrap();
        store.record_client_export(&g.key, "/x/3", 3).unwrap();
        store.conn.execute("UPDATE topic_seed SET merged_into_key = ?1 WHERE key = ?2", [&a.key, &b.key]).unwrap();
        assert_eq!(store.client_exports(&a.key).unwrap(), vec!["/x/1", "/x/2"]);
        assert_eq!(store.client_exports(&g.key).unwrap(), vec!["/x/3"]);
        let row: String = store.conn.query_row("SELECT client FROM client_export WHERE path = '/x/1'", [], |r| r.get(0)).unwrap();
        assert!(!row.contains(&a.key) && !row.contains("Acme"), "opaque owner only");
        store.forget_client_export("/x/1").unwrap();
        assert!(!store.is_client_export("/x/1").unwrap());
        assert!(store.is_client_export("/x/3").unwrap());
    }
}
