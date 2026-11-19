//! The per-item deletes behind "Delete this client" (docs/superpowers/specs/
//! 2026-11-12-client-controls-design.md section 4.2). `api::client_delete` drives them; each item goes in
//! its own transaction, together with its tombstone ([`Store::enqueue_client_item_forgotten`] is read
//! from the row before it is deleted), so a failure leaves a re-runnable state and never a half-deleted
//! item.
//!
//! What each item takes with it, beyond its own row:
//! - every source-backed item (email, calendar event, episode): the steps, step reads and verdicts filed
//!   from it on any topic, each with its own tombstone (a pulled step has no local `source_id`, so the
//!   delete trigger alone would miss it);
//! - email: the Gmail thread it belongs to (its summary was built from the mail), with its priority score
//!   and jobs;
//! - calendar event: its `calendar_presence` rows and (through the tombstone) its meeting briefs;
//! - capture: its `capture_item` twin and the episodes on that device and app whose time covers it;
//! - episode: the episode alone. The captures it was built over are listed by the client inventory as
//!   captures of their own ([`Store::episode_captures`]) and deleted as such, so the review and the delete
//!   cover the same items.
//!
//! [`Store::purge_client_text`] removes derived text that names the client and has no source link
//! (routine runs, the digest cache, job payloads, profile suggestions, briefs, follow-ups, other topics'
//! names and page snapshots, entities, people), and [`Store::finish_client_delete`] prunes orphans and
//! rewrites the full-text indexes so no deleted term stays in them. Nothing here logs or returns text.

use rusqlite::{params, Connection, OptionalExtension, Transaction};

use crate::client_forget::{
    self, delete_episode, drop_unsent, hashed, take_delivered, take_delivered_aggregate_in, ClientItemKind, Reach,
    TombstoneOutcome,
};
use crate::{
    now_unix_ms, Store, OUTBOX_ROW_TYPE_MEETING_PREP, OUTBOX_ROW_TYPE_PERSON, OUTBOX_ROW_TYPE_PROFILE_SUGGESTION,
    OUTBOX_ROW_TYPE_PROFILE_SUGGESTION_STATUS, OUTBOX_ROW_TYPE_ROUTINE_RUN, OUTBOX_ROW_TYPE_TOPIC_PAGE_SNAPSHOT,
    OUTBOX_ROW_TYPE_TOPIC_VERDICT,
};

/// What a client's seed reaches before it is deleted: the ids its vault notes are filed under.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClientScope {
    /// The seed's topic, the seeds merged into it and the unseeded topics merged into those.
    pub topic_ids: Vec<i64>,
    /// People and organizations its members mention (`entity` ids).
    pub entity_ids: Vec<i64>,
}

/// Synced row types that only ever hold derived text: a pending or delivered op that names the client is
/// cancelled or taken (its relay row recorded for deletion).
const DERIVED_ROW_TYPES: [&str; 6] = [
    OUTBOX_ROW_TYPE_ROUTINE_RUN,
    OUTBOX_ROW_TYPE_MEETING_PREP,
    OUTBOX_ROW_TYPE_PROFILE_SUGGESTION,
    OUTBOX_ROW_TYPE_PROFILE_SUGGESTION_STATUS,
    OUTBOX_ROW_TYPE_PERSON,
    OUTBOX_ROW_TYPE_TOPIC_PAGE_SNAPSHOT,
];

fn ids(conn: &Connection, sql: &str, p: impl rusqlite::Params) -> rusqlite::Result<Vec<i64>> {
    conn.prepare(sql)?.query_map(p, |r| r.get(0))?.collect()
}

fn strings(conn: &Connection, sql: &str, p: impl rusqlite::Params) -> rusqlite::Result<Vec<String>> {
    conn.prepare(sql)?.query_map(p, |r| r.get(0))?.collect()
}

/// The worse of two outcomes for the receipt: anything not sent beats sent; local-only is the mildest.
fn worse(a: TombstoneOutcome, b: TombstoneOutcome) -> TombstoneOutcome {
    use TombstoneOutcome::*;
    match (a, b) {
        (NoDeviceId, _) | (_, NoDeviceId) => NoDeviceId,
        (Queued, _) | (_, Queued) => Queued,
        _ => LocalOnly,
    }
}

/// What [`Store::purge_client_text`] removed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PurgedText {
    pub removed: usize,
    /// Entities it deleted (by name): their vault notes go too.
    pub entity_ids: Vec<i64>,
}

/// `client_delete_pending.kind` values: vault note owners (opaque row ids) and meeting files.
pub const PENDING_TOPIC: &str = "topic";
pub const PENDING_ENTITY: &str = "entity";
pub const PENDING_MEETING: &str = "meeting";
/// A meeting file the forget handed back for deletion (audio, transcript), or its slide folder.
pub const PENDING_AUDIO: &str = "audio";
pub const PENDING_TRANSCRIPT: &str = "transcript";
pub const PENDING_SLIDES: &str = "slides";
/// Any path a deleted meeting's rows named: checked afterwards and reported if still on disk.
pub const PENDING_NAMED: &str = "named";
/// A capture (its `items.id`) an episode was built over, recorded when the episode is deleted: once the
/// episode is gone the inventory no longer lists it, so the delete (or its re-run) takes it from here.
pub const PENDING_CAPTURE: &str = "capture";

pub(crate) fn ensure_client_delete_tables(conn: &Connection) -> rusqlite::Result<()> {
    // Work a client delete still owes outside its rows (vault notes, meeting files), recorded before the rows
    // go so a crash or a failed step is finished by the re-run. `run` is an opaque hash of the client's key;
    // rows are cleared when done, before the client record itself is purged.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS client_delete_pending (
            run TEXT NOT NULL,
            kind TEXT NOT NULL,
            value TEXT NOT NULL,
            PRIMARY KEY (run, kind, value)
        ) WITHOUT ROWID;",
    )
}

fn add_pending(conn: &Connection, run: &str, kind: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO client_delete_pending (run, kind, value) VALUES (?1, ?2, ?3)",
        params![run, kind, value],
    )?;
    Ok(())
}

impl Store {
    /// The opaque id a client delete's pending work is kept under: a hash of the client's key.
    pub fn client_delete_run_id(key: &str) -> String {
        hashed(&format!("client_delete:{key}"))
    }

    /// The kv prefix this device keeps client `key`'s keep-for-N-months decisions under (`Keep`, `Snooze`):
    /// opaque, cleared when the client is purged. Never synced.
    pub fn client_retention_prefix(key: &str) -> String {
        format!("client_retention:{}:", hashed(&format!("client_retention:{key}")))
    }

    /// The opaque id a keep-for-N-months expiry's pending work is kept under: its own record, never the
    /// client delete's ([`Self::client_delete_run_id`]).
    pub fn client_expiry_run_id(key: &str) -> String {
        hashed(&format!("client_expiry:{key}"))
    }

    /// kv key of the count of client `key`'s items an expiry removed on this device only (opaque).
    fn client_expired_key(key: &str) -> String {
        format!("client_expired:{}", hashed(&format!("client_expired:{key}")))
    }

    /// Adds `n` to the count of client `key`'s items removed here only by an expiry (their copies on other
    /// devices stay; a later client delete says so). Cleared when the client is purged.
    pub fn add_client_expired_count(&self, key: &str, n: usize) -> rusqlite::Result<()> {
        if n == 0 {
            return Ok(());
        }
        let k = Self::client_expired_key(key);
        let now: usize = self.get_meta(&k)?.and_then(|v| v.parse().ok()).unwrap_or(0);
        self.set_meta(&k, &(now + n).to_string())
    }

    /// See [`Self::add_client_expired_count`].
    pub fn client_expired_count(&self, key: &str) -> rusqlite::Result<usize> {
        Ok(self.get_meta(&Self::client_expired_key(key))?.and_then(|v| v.parse().ok()).unwrap_or(0))
    }

    pub(crate) fn clear_client_expired_count_tx(conn: &Connection, key: &str) -> rusqlite::Result<()> {
        conn.execute("DELETE FROM kv_meta WHERE key = ?1", [Self::client_expired_key(key)])?;
        Ok(())
    }

    /// A keep-for-N-months expiry of one item: [`Self::delete_client_item_in`] with
    /// `Reach::ThisDevice { sync_on }`, built here so an expiry can never ask for more.
    pub fn expire_client_item(
        &self,
        kind: ClientItemKind,
        local_id: &str,
        sync_on: bool,
    ) -> rusqlite::Result<Option<TombstoneOutcome>> {
        let out = self.delete_client_item_in(kind, local_id, Reach::ThisDevice { sync_on })?;
        assert_ne!(out, Some(TombstoneOutcome::Queued), "an expiry queued a tombstone");
        Ok(out)
    }

    /// A keep-for-N-months expiry of an episode and (recorded as pending under `run`) its captures, on this
    /// device only.
    pub fn expire_client_episode(
        &self,
        run: &str,
        local_id: &str,
        sync_on: bool,
    ) -> rusqlite::Result<Option<TombstoneOutcome>> {
        let out = self.delete_client_episode_in(run, local_id, Reach::ThisDevice { sync_on })?;
        assert_ne!(out, Some(TombstoneOutcome::Queued), "an expiry queued a tombstone");
        Ok(out)
    }

    /// Records pending work for delete `run`.
    pub fn add_client_delete_pending(&self, run: &str, kind: &str, values: &[String]) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        for v in values {
            add_pending(&tx, run, kind, v)?;
        }
        tx.commit()
    }

    /// Moves `from`'s pending work to `to` (a client delete taking over what an expiry still owes).
    pub fn adopt_client_delete_pending(&self, from: &str, to: &str) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT OR IGNORE INTO client_delete_pending (run, kind, value) \
             SELECT ?2, kind, value FROM client_delete_pending WHERE run = ?1",
            params![from, to],
        )?;
        tx.execute("DELETE FROM client_delete_pending WHERE run = ?1", [from])?;
        tx.commit()
    }

    /// Delete `run`'s pending work: `(kind, value)`.
    pub fn client_delete_pending(&self, run: &str) -> rusqlite::Result<Vec<(String, String)>> {
        self.conn
            .prepare("SELECT kind, value FROM client_delete_pending WHERE run = ?1 ORDER BY kind, value")?
            .query_map([run], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect()
    }

    /// Marks these pending items of `run` done.
    pub fn clear_client_delete_pending(&self, run: &str, items: &[(String, String)]) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        for (kind, value) in items {
            tx.execute(
                "DELETE FROM client_delete_pending WHERE run = ?1 AND kind = ?2 AND value = ?3",
                params![run, kind, value],
            )?;
        }
        tx.commit()
    }

    /// The files deleting meeting `id` for a client delete removes: as [`Self::forget_meeting`] hands back,
    /// plus its slide folder even when the user chose to keep slides (the client delete overrides that; the
    /// folder is still only removed when it carries Whimbrel's marker). `None` if no such meeting.
    pub fn meeting_files_for_client_delete(&self, id: i64) -> rusqlite::Result<Option<crate::DeletedMeetingFiles>> {
        let row: Option<(Option<String>, Option<String>, Option<String>)> = self
            .conn
            .query_row(
                "SELECT audio_path, transcript_path, snapshots_dir FROM meeting WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        row.map(|(a, t, s)| self.meeting_deleted_files(id, a, t, s)).transpose()
    }

    /// The ids the seed `key` (and its merge chain) reaches, read before anything is deleted.
    pub fn client_scope(&self, key: &str) -> rusqlite::Result<ClientScope> {
        let c = &self.conn;
        let mut topic_ids = Vec::new();
        for k in client_forget::merge_chain(c, key)? {
            let Some(id) = c
                .query_row("SELECT topic_id FROM topic_seed WHERE key = ?1", [&k], |r| r.get::<_, i64>(0))
                .optional()?
            else {
                continue;
            };
            topic_ids.extend(ids(
                c,
                "WITH RECURSIVE t(id, depth) AS (SELECT ?1, 0 UNION \
                 SELECT topic.id, t.depth + 1 FROM topic JOIN t ON topic.merged_into = t.id WHERE t.depth < 64) \
                 SELECT id FROM t",
                [id],
            )?);
        }
        topic_ids.sort_unstable();
        topic_ids.dedup();
        let mut entity_ids = Vec::new();
        for t in &topic_ids {
            entity_ids.extend(ids(
                c,
                "SELECT DISTINCT e.id FROM topic_member m JOIN entity_mention em ON em.source_id = m.source_id \
                 JOIN entity e ON e.id = em.entity_id WHERE m.topic_id = ?1 AND e.kind IN ('person', 'org')",
                [t],
            )?);
        }
        entity_ids.sort_unstable();
        entity_ids.dedup();
        Ok(ClientScope { topic_ids, entity_ids })
    }

    /// The names and aliases of seed `key` and every seed merged into it: what derived text is matched against.
    /// Held only in memory for the delete. An alias list that does not parse is an error (fail closed: a
    /// sweep without the aliases would leave text naming the client).
    pub fn seed_chain_terms(&self, key: &str) -> rusqlite::Result<Vec<String>> {
        let mut terms = Vec::new();
        for k in client_forget::merge_chain(&self.conn, key)? {
            let row: Option<(String, String)> = self
                .conn
                .query_row(
                    "SELECT t.name, s.aliases FROM topic_seed s JOIN topic t ON t.id = s.topic_id WHERE s.key = ?1",
                    [&k],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let Some((name, aliases)) = row else { continue };
            let aliases: Vec<String> = serde_json::from_str(&aliases).map_err(crate::json_to_sql_error)?;
            terms.push(name);
            terms.extend(aliases);
        }
        terms.retain(|t| !t.trim().is_empty());
        Ok(terms)
    }

    /// Every file path meeting `id`'s rows name (audio, transcript, its parts', and the slide folder), whoever
    /// recorded it. The forget hands back only the ones it may delete; the client delete checks all of these
    /// afterwards, so a file it was not allowed to touch is reported as left, never silently kept.
    pub fn meeting_file_paths(&self, id: i64) -> rusqlite::Result<Vec<String>> {
        let mut out: Vec<String> = Vec::new();
        let row: Option<(Option<String>, Option<String>, Option<String>)> = self
            .conn
            .query_row(
                "SELECT audio_path, transcript_path, snapshots_dir FROM meeting WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        if let Some((a, t, s)) = row {
            out.extend([a, t, s].into_iter().flatten());
        }
        let mut stmt = self.conn.prepare("SELECT audio_path, transcript_path FROM meeting_part WHERE meeting_id = ?1")?;
        let parts = stmt.query_map([id], |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?)))?;
        for p in parts {
            let (a, t) = p?;
            out.extend([a, t].into_iter().flatten());
        }
        out.sort();
        out.dedup();
        Ok(out)
    }

    /// Which of `entity_ids` no longer exist (pruned): their vault notes can go.
    pub fn missing_entities(&self, entity_ids: &[i64]) -> rusqlite::Result<Vec<i64>> {
        let mut out = Vec::new();
        for id in entity_ids {
            let here: bool =
                self.conn.query_row("SELECT EXISTS (SELECT 1 FROM entity WHERE id = ?1)", [id], |r| r.get(0))?;
            if !here {
                out.push(*id);
            }
        }
        Ok(out)
    }

    /// The `items.id` of capture `capture_item_id` (the client inventory lists captures by `capture_item.id`).
    pub fn capture_items_id(&self, capture_item_id: i64) -> rusqlite::Result<Option<i64>> {
        self.conn
            .query_row(
                "SELECT id FROM items WHERE kind = 'capture' AND source_table = 'capture_item' AND source_id = ?1",
                [capture_item_id],
                |r| r.get(0),
            )
            .optional()
    }

    /// Deletes a capture the client inventory lists by `capture_item.id`: through its `items` row when it has
    /// one ([`Self::delete_client_item`]), else (a capture never mirrored) the `capture_item` row alone, which
    /// never synced through `items`. `None` when it is already gone.
    pub fn delete_client_capture(&self, capture_item_id: i64) -> rusqlite::Result<Option<TombstoneOutcome>> {
        self.delete_client_capture_in(capture_item_id, Reach::Everywhere)
    }

    /// [`Self::delete_client_capture`] under `reach` (see [`Reach`]).
    pub fn delete_client_capture_in(
        &self,
        capture_item_id: i64,
        reach: Reach,
    ) -> rusqlite::Result<Option<TombstoneOutcome>> {
        if let Some(items_id) = self.capture_items_id(capture_item_id)? {
            return self.delete_client_item_in(ClientItemKind::Capture, &items_id.to_string(), reach);
        }
        let tx = self.conn.unchecked_transaction()?;
        let row: Option<(String, Option<i64>)> = tx
            .query_row(
                "SELECT device_id, local_seq FROM capture_item WHERE id = ?1",
                [capture_item_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((device, seq)) = row else {
            return Ok(None);
        };
        // A legacy row pushed as `capture_item` has the same identity as its `items` twin would.
        let outcome = match (reach, seq) {
            (Reach::Everywhere, Some(seq)) => self.enqueue_raw_capture_tombstone_tx(&tx, &device, seq)?,
            (Reach::ThisDevice { sync_on: true }, seq) if self.expiry_must_check_push(true, &device)? => {
                let cursor: i64 =
                    tx.query_row("SELECT last_pushed_seq FROM sync_push_cursor WHERE id = 1", [], |r| r.get(0))?;
                if seq.is_some_and(|seq| seq > cursor) {
                    return Err(client_forget::unsent_ops());
                }
                TombstoneOutcome::LocalOnly
            }
            _ => TombstoneOutcome::LocalOnly,
        };
        tx.execute("DELETE FROM capture_item WHERE id = ?1", [capture_item_id])?;
        tx.commit()?;
        Ok(Some(outcome))
    }

    /// Deletes one item of client material here, with its tombstone for other devices, in one transaction.
    /// `local_id` is as [`Self::enqueue_client_item_forgotten`] takes it. `None` when the item is already gone
    /// (a re-run), never an error for that.
    pub fn delete_client_item(
        &self,
        kind: ClientItemKind,
        local_id: &str,
    ) -> rusqlite::Result<Option<TombstoneOutcome>> {
        self.delete_client_item_in(kind, local_id, Reach::Everywhere)
    }

    /// [`Self::delete_client_item`] under `reach`. `Reach::ThisDevice` is the keep-for-N-months expiry: the
    /// item and what was built from it go here only (see [`Reach`]); an item still waiting to be sent is
    /// `Err` ([`client_forget::is_unsent_ops`]) with nothing changed.
    pub fn delete_client_item_in(
        &self,
        kind: ClientItemKind,
        local_id: &str,
        reach: Reach,
    ) -> rusqlite::Result<Option<TombstoneOutcome>> {
        let tx = self.conn.unchecked_transaction()?;
        let outcome = self.delete_client_item_tx(&tx, kind, local_id, reach)?;
        tx.commit()?;
        Ok(outcome)
    }

    /// Deletes episode `local_id` for client delete `run` ([`Self::delete_client_item`]), recording the
    /// captures it was built over as pending in the same transaction, so they are still deleted if the run
    /// stops or fails before reaching them.
    pub fn delete_client_episode(&self, run: &str, local_id: &str) -> rusqlite::Result<Option<TombstoneOutcome>> {
        self.delete_client_episode_in(run, local_id, Reach::Everywhere)
    }

    /// [`Self::delete_client_episode`] under `reach`.
    pub fn delete_client_episode_in(
        &self,
        run: &str,
        local_id: &str,
        reach: Reach,
    ) -> rusqlite::Result<Option<TombstoneOutcome>> {
        let tx = self.conn.unchecked_transaction()?;
        if let Ok(id) = local_id.parse::<i64>() {
            for (item, _, _) in self.episode_captures(id)? {
                add_pending(&tx, run, PENDING_CAPTURE, &item.to_string())?;
            }
        }
        let outcome = self.delete_client_item_tx(&tx, ClientItemKind::Episode, local_id, reach)?;
        tx.commit()?;
        Ok(outcome)
    }

    /// Whether the client item the inventory lists as `kind`/`local_id` is still stored. `kind: None` is a
    /// capture by the inventory's id: its `capture_item.id`, or `i:<items.id>`.
    pub fn client_item_exists(&self, kind: Option<ClientItemKind>, local_id: &str) -> rusqlite::Result<bool> {
        let one = |sql: &str, id: &str| -> rusqlite::Result<bool> {
            self.conn.query_row(sql, [id], |r| r.get(0))
        };
        match kind {
            Some(k) => match client_forget::identity(&self.conn, k, local_id) {
                Ok(_) => Ok(true),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(false),
                Err(e) => Err(e),
            },
            None => match local_id.strip_prefix("i:") {
                Some(item) => one("SELECT EXISTS (SELECT 1 FROM items WHERE id = CAST(?1 AS INTEGER))", item),
                None => one("SELECT EXISTS (SELECT 1 FROM capture_item WHERE id = CAST(?1 AS INTEGER))", local_id),
            },
        }
    }

    fn delete_client_item_tx(
        &self,
        tx: &Transaction<'_>,
        kind: ClientItemKind,
        local_id: &str,
        reach: Reach,
    ) -> rusqlite::Result<Option<TombstoneOutcome>> {
        match client_forget::identity(tx, kind, local_id) {
            Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
            Err(e) => return Err(e),
            Ok(_) => {}
        }
        let id = || local_id.parse::<i64>().map_err(|_| crate::topics::seeds::invalid("client item id is not a row id"));
        // Before the source row goes: what was filed from it (its key is read from the row).
        let mut outcome = match kind {
            ClientItemKind::Email => self.forget_filed_from_tx(tx, "email", id()?, reach)?,
            ClientItemKind::CalendarEvent => self.forget_filed_from_tx(tx, "calendar_event", id()?, reach)?,
            ClientItemKind::Episode => self.forget_filed_from_tx(tx, "episode", id()?, reach)?,
            _ => TombstoneOutcome::LocalOnly,
        };
        outcome = worse(outcome, self.forget_client_item_in_tx(tx, kind, local_id, reach)?);
        match kind {
            ClientItemKind::Email => {
                let thread: Option<String> = tx.query_row(
                    "SELECT json_extract(refs, '$.gmail_thread_id') FROM items WHERE id = ?1",
                    [id()?],
                    |r| r.get(0),
                )?;
                tx.execute("DELETE FROM items WHERE id = ?1 AND kind = 'email'", [id()?])?;
                // An expiry takes one old message: the thread stays while newer mail in it is still here.
                let thread = match (reach, thread) {
                    (Reach::ThisDevice { .. }, Some(t)) => {
                        let left: bool = tx.query_row(
                            "SELECT EXISTS (SELECT 1 FROM items WHERE kind = 'email' \
                             AND json_extract(refs, '$.gmail_thread_id') = ?1)",
                            [&t],
                            |r| r.get(0),
                        )?;
                        (!left).then_some(t)
                    }
                    (_, t) => t,
                };
                if let Some(thread) = thread {
                    for t in ids(tx, "SELECT id FROM threads WHERE source = 'gmail' AND external_thread_id = ?1", [&thread])? {
                        if let Some(o) = self.delete_client_item_tx(tx, ClientItemKind::Thread, &t.to_string(), reach)? {
                            outcome = worse(outcome, o);
                        }
                    }
                }
            }
            ClientItemKind::Thread => {
                let t = id()?;
                tx.execute("DELETE FROM thread_priority WHERE thread_id = ?1", [t])?;
                tx.execute(
                    "DELETE FROM jobs WHERE (job_type = 'thread_summarize' AND content_hash = ?1) \
                     OR (job_type = 'thread_priority' AND content_hash = ?2)",
                    params![format!("thread_summarize:{t}"), format!("thread_priority:{t}")],
                )?;
                tx.execute("DELETE FROM threads WHERE id = ?1", [t])?;
            }
            ClientItemKind::CalendarEvent => {
                let (source, ext): (Option<String>, Option<String>) =
                    tx.query_row("SELECT source, external_id FROM items WHERE id = ?1", [id()?], |r| {
                        Ok((r.get(0)?, r.get(1)?))
                    })?;
                if let (Some(s), Some(x)) = (source, ext) {
                    tx.execute(
                        "DELETE FROM calendar_presence WHERE source = ?1 AND external_id = ?2",
                        params![s, x],
                    )?;
                }
                tx.execute("DELETE FROM items WHERE id = ?1 AND kind = 'calendar_event'", [id()?])?;
            }
            ClientItemKind::Capture => {
                let (device, app, ts, twin): (String, Option<String>, i64, Option<i64>) = tx.query_row(
                    "SELECT device_id, app_bundle_id, ts_unix_ms, CASE WHEN source_table = 'capture_item' THEN source_id END \
                     FROM items WHERE id = ?1",
                    [id()?],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )?;
                // The episodes built over it: their summary is text from the capture. An expiry deletes
                // captures only as part of their own episode (already gone), and never another episode,
                // which may be kept or another client's.
                let covering = match reach {
                    Reach::Everywhere => ids(
                        tx,
                        "SELECT id FROM episodes WHERE device_id = ?1 AND app_bundle_id = COALESCE(?2, '(unknown)') \
                         AND start_ts_unix_ms <= ?3 AND end_ts_unix_ms >= ?3",
                        params![device, app, ts],
                    )?,
                    Reach::ThisDevice { .. } => Vec::new(),
                };
                for e in covering {
                    self.forget_filed_from_tx(tx, "episode", e, reach)?;
                    delete_episode(tx, e)?;
                }
                if let Some(twin) = twin {
                    tx.execute("DELETE FROM capture_item WHERE id = ?1", [twin])?;
                }
                tx.execute("DELETE FROM items WHERE id = ?1 AND kind = 'capture'", [id()?])?;
            }
            ClientItemKind::Episode => delete_episode(tx, id()?)?,
            ClientItemKind::Conversation => {
                crate::topics::exclusion::drop_conversation_stamps_tx(
                    tx,
                    "SELECT answer FROM conversation_exchange WHERE conversation_id = ?1",
                    [id()?],
                )?;
                tx.execute("DELETE FROM conversation_exchange WHERE conversation_id = ?1", [id()?])?;
                tx.execute("DELETE FROM conversation WHERE id = ?1", [id()?])?;
            }
            ClientItemKind::Step => {
                tx.execute("DELETE FROM topic_step WHERE key = ?1", [local_id])?;
            }
            ClientItemKind::Extraction => {
                tx.execute("DELETE FROM topic_extraction WHERE source_key = ?1", [local_id])?;
            }
        }
        Ok(Some(outcome))
    }

    /// A never-mirrored legacy capture's tombstone, by its `(device, local_seq)` identity.
    fn enqueue_raw_capture_tombstone_tx(
        &self,
        tx: &Transaction<'_>,
        device: &str,
        seq: i64,
    ) -> rusqlite::Result<TombstoneOutcome> {
        Store::enqueue_opaque_tombstone_tx(self, tx, ClientItemKind::Capture, &format!("{device}:{seq}"))
    }

    /// The steps, step reads and verdicts filed from source `kind`/`ref_id`, on any topic: each step and step
    /// read goes with its tombstone, each verdict here with its unsent and delivered ops. Run before the source
    /// row is deleted (its key is read from it). Returns the worst tombstone outcome.
    fn forget_filed_from_tx(
        &self,
        tx: &Transaction<'_>,
        kind: &str,
        ref_id: i64,
        reach: Reach,
    ) -> rusqlite::Result<TombstoneOutcome> {
        let mut outcome = TombstoneOutcome::LocalOnly;
        let sid: Option<i64> = tx
            .query_row(
                "SELECT id FROM topic_source WHERE kind = ?1 AND ref_id = ?2",
                params![kind, ref_id],
                |r| r.get(0),
            )
            .optional()?;
        let Some(sid) = sid else {
            return Ok(outcome);
        };
        let own = self.own_device_id();
        let key = crate::topics::judge::source_key(tx, own.as_deref(), sid)?;
        let mut steps = strings(tx, "SELECT key FROM topic_step WHERE source_id = ?1", [sid])?;
        let mut reads = strings(tx, "SELECT source_key FROM topic_extraction WHERE source_id = ?1", [sid])?;
        if let Some(k) = &key {
            steps.extend(strings(tx, "SELECT key FROM topic_step WHERE source_key = ?1", [k])?);
            reads.extend(strings(tx, "SELECT source_key FROM topic_extraction WHERE source_key = ?1", [k])?);
            let at = now_unix_ms();
            drop_unsent(tx, reach, OUTBOX_ROW_TYPE_TOPIC_VERDICT, k)?;
            take_delivered_aggregate_in(tx, reach, OUTBOX_ROW_TYPE_TOPIC_VERDICT, k, at)?;
            tx.execute("DELETE FROM topic_verdict WHERE source_key = ?1", [k])?;
        }
        steps.sort();
        steps.dedup();
        reads.sort();
        reads.dedup();
        for s in steps {
            if let Some(o) = self.delete_client_item_tx(tx, ClientItemKind::Step, &s, reach)? {
                outcome = worse(outcome, o);
            }
        }
        for r in reads {
            if let Some(o) = self.delete_client_item_tx(tx, ClientItemKind::Extraction, &r, reach)? {
                outcome = worse(outcome, o);
            }
        }
        Ok(outcome)
    }

    /// Removes derived text that names the client (`names` says whether a text does: the client's name or an
    /// alias as whole words) and has no source link the item deletes could follow: routine runs, the MCP digest
    /// cache, job payloads, profile suggestions and their resolutions, meeting briefs, follow-ups, people,
    /// other topics' automatic names and page snapshots, entities, calendar presence rows, and the unsent or
    /// delivered sync ops of those (a delivered op's relay row is recorded for deletion). Runs while the
    /// client's own rows are already gone, so it only meets mixed rows. Returns the rows removed and the
    /// entities deleted (their vault notes go too); with `run`, those entity ids are also recorded as that
    /// delete's pending vault owners in the same transaction, so a crash right after cannot lose them.
    pub fn purge_client_text(
        &self,
        run: Option<&str>,
        names: &dyn Fn(&str) -> bool,
    ) -> rusqlite::Result<PurgedText> {
        let tx = self.conn.unchecked_transaction()?;
        let mut entity_ids: Vec<i64> = Vec::new();
        let at = now_unix_ms();
        let mut removed = 0;
        // Rows matched by a text column: (table, rowid column, text expression).
        let scan = |table: &str, id_col: &str, text: &str| -> rusqlite::Result<Vec<String>> {
            let mut stmt = tx.prepare(&format!("SELECT CAST({id_col} AS TEXT), {text} FROM {table}"))?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)))?;
            let mut out = Vec::new();
            for row in rows {
                let (id, t) = row?;
                // JSON columns are matched decoded: a name after an escaped newline is a whole
                // word (X6 fix round 6, J1).
                if t.as_deref().is_some_and(|t| names(&crate::words::name_text(t))) {
                    out.push(id);
                }
            }
            Ok(out)
        };
        let cat = |cols: &[&str]| cols.iter().map(|c| format!("coalesce({c}, '')")).collect::<Vec<_>>().join(" || ' ' || ");

        let mut run_ids = scan("routine_run", "id", "output_summary")?;
        run_ids.extend(scan("routine_run", "id", "output_json")?);
        run_ids.extend(scan("routine_run", "id", "pending_action_json")?);
        run_ids.sort();
        run_ids.dedup();
        for id in run_ids {
            tx.execute("UPDATE meeting_followup SET routine_run_id = NULL WHERE routine_run_id = ?1", [&id])?;
            removed += tx.execute("DELETE FROM routine_run WHERE id = ?1", [&id])?;
        }
        // A cache of whole days built from many sources, some of them deleted: dropped whole, it is built again.
        removed += tx.execute("DELETE FROM mcp_daily_digest_cache", [])?;
        for id in scan("jobs", "id", "payload_json")? {
            removed += tx.execute("DELETE FROM jobs WHERE id = ?1", [&id])?;
        }
        for id in scan("profile_suggestion", "id", "text")? {
            removed += tx.execute("DELETE FROM profile_suggestion WHERE id = ?1", [&id])?;
        }
        for id in scan("profile_suggestion_resolution", "rowid", "text_key")? {
            removed += tx.execute("DELETE FROM profile_suggestion_resolution WHERE rowid = ?1", [&id])?;
        }
        let mut prep_ids = scan("meeting_prep", "event_id", "title")?;
        prep_ids.extend(scan("meeting_prep", "event_id", "brief_json")?);
        prep_ids.sort();
        prep_ids.dedup();
        for id in prep_ids {
            removed += tx.execute("DELETE FROM meeting_prep WHERE event_id = ?1", [&id])?;
        }
        for id in scan("meeting_followup", "id", "text")? {
            removed += tx.execute("DELETE FROM meeting_followup WHERE id = ?1", [&id])?;
        }
        for id in scan("person", "person_key", &cat(&["name", "email"]))? {
            tx.execute("UPDATE meeting_speaker SET person_key = NULL WHERE person_key = ?1", [&id])?;
            removed += tx.execute("DELETE FROM person WHERE person_key = ?1", [&id])?;
        }
        for id in scan("calendar_presence", "rowid", "external_id")? {
            removed += tx.execute("DELETE FROM calendar_presence WHERE rowid = ?1", [&id])?;
        }
        for id in scan("topic_page_snapshot", "topic_key", "payload")? {
            removed += tx.execute("DELETE FROM topic_page_snapshot WHERE topic_key = ?1", [&id])?;
        }
        // Another topic's automatic name or status line: blanked and named again from what is left.
        // Seeds keep the names the user gave them.
        for id in scan("topic WHERE id NOT IN (SELECT topic_id FROM topic_seed)", "id", &cat(&["name", "status_line"]))? {
            removed += tx.execute(
                "UPDATE topic SET name = '', name_is_user = 0, name_dirty = 1, status_line = NULL WHERE id = ?1",
                [&id],
            )?;
        }
        for id in scan("entity", "id", &cat(&["key", "display"]))? {
            if let Ok(n) = id.parse::<i64>() {
                entity_ids.push(n);
                if let Some(run) = run {
                    add_pending(&tx, run, PENDING_ENTITY, &id)?;
                }
            }
            tx.execute("DELETE FROM entity_mention WHERE entity_id = ?1", [&id])?;
            tx.execute("DELETE FROM topic_idea WHERE entity_id = ?1", [&id])?;
            removed += tx.execute("DELETE FROM entity WHERE id = ?1", [&id])?;
        }
        // The sync ops of derived rows: unsent ones are cancelled; a delivered one's relay row is this
        // device's to delete, and its payload (the text) goes.
        for row_type in DERIVED_ROW_TYPES {
            let mut stmt = tx.prepare(
                "SELECT local_seq, payload_json, state FROM sync_outbox WHERE row_type = ?1 \
                 AND state IN ('pending', 'blocked', 'delivered')",
            )?;
            let ops: Vec<(i64, String, String)> = stmt
                .query_map([row_type], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<rusqlite::Result<_>>()?;
            drop(stmt);
            for (seq, payload, state) in ops {
                if !names(&crate::words::name_text(&payload)) {
                    continue;
                }
                if state == "delivered" {
                    take_delivered(&tx, row_type, "local_seq = CAST(?1 AS INTEGER)", &seq.to_string(), at)?;
                } else {
                    tx.execute(
                        "DELETE FROM sync_outbox WHERE row_type = ?1 AND local_seq = ?2 AND state IN ('pending', 'blocked')",
                        params![row_type, seq],
                    )?;
                }
                removed += 1;
            }
        }
        tx.commit()?;
        Ok(PurgedText { removed, entity_ids })
    }

    /// The last step of a client delete: entities and topics left without members are pruned, and the
    /// full-text indexes are rewritten so a deleted term is no longer stored in them (a delete only adds a
    /// tombstone entry to an FTS5 index). Run with secure delete on, so freed pages are zeroed, then the
    /// write-ahead log is checkpointed.
    pub fn finish_client_delete(&self) -> rusqlite::Result<()> {
        self.finish_client_delete_with("TRUNCATE")
    }

    /// [`Self::finish_client_delete`] with WAL checkpoint `mode`. `TRUNCATE` (a delete or expiry the user is
    /// waiting on) waits for readers through the busy handler; `PASSIVE` (after a sync round) never waits: it copies
    /// what it can now and leaves the rest to SQLite's next automatic checkpoint.
    pub(crate) fn finish_client_delete_with(&self, mode: &str) -> rusqlite::Result<()> {
        crate::topics::schema::prune_orphans(&self.conn)?;
        client_forget::clear_model_error_notes(&self.conn)?;
        for fts in ["items_fts", "transcript_segment_fts", "capture_item_fts"] {
            self.conn.execute(&format!("INSERT INTO {fts}({fts}) VALUES ('optimize')"), [])?;
        }
        // In memory or not in WAL mode this is a no-op.
        let _ = self.conn.query_row(&format!("PRAGMA wal_checkpoint({mode})"), [], |_| Ok(()));
        Ok(())
    }

    /// How many stored full-text index pages (`items_fts`, `transcript_segment_fts`, `capture_item_fts`) hold the
    /// bytes of `term` (lower case, as indexed). For tests and diagnostics: a plain delete leaves a term in its
    /// segment until the index is merged.
    pub fn fts_pages_naming(&self, term: &str) -> rusqlite::Result<i64> {
        let mut n = 0;
        for fts in ["items_fts", "transcript_segment_fts", "capture_item_fts"] {
            n += self.conn.query_row(
                &format!("SELECT COUNT(*) FROM {fts}_data WHERE instr(block, CAST(?1 AS BLOB)) > 0"),
                [term],
                |r| r.get::<_, i64>(0),
            )?;
        }
        Ok(n)
    }

    /// The main database file's path; `None` for an in-memory store.
    pub fn db_path(&self) -> Option<std::path::PathBuf> {
        self.conn.path().filter(|p| !p.is_empty()).map(std::path::PathBuf::from)
    }

    /// Turns SQLite's secure delete on or off for this connection (deleted content is overwritten with
    /// zeros), returning the previous setting.
    pub fn set_secure_delete(&self, on: bool) -> rusqlite::Result<bool> {
        let before: i64 = self.conn.query_row("PRAGMA secure_delete", [], |r| r.get(0))?;
        let _: i64 = self.conn.query_row(&format!("PRAGMA secure_delete = {}", on as i64), [], |r| r.get(0))?;
        Ok(before != 0)
    }

    /// Whether a pulled or polled email / calendar event is one a client delete forgot here, or a
    /// keep-for-N-months expiry deleted here (so ingestion skips it instead of bringing it back).
    pub(crate) fn is_forgotten_email(&self, external_id: &str) -> rusqlite::Result<bool> {
        Ok(client_forget::is_forgotten(&self.conn, ClientItemKind::Email.as_str(), external_id)?
            || self.is_expired_here(ClientItemKind::Email, external_id)?)
    }

    pub(crate) fn is_forgotten_calendar_event(&self, source: &str, external_id: &str) -> rusqlite::Result<bool> {
        let id = hashed(&format!("{source}:{external_id}"));
        Ok(client_forget::is_forgotten(&self.conn, ClientItemKind::CalendarEvent.as_str(), &id)?
            || self.is_expired_here(ClientItemKind::CalendarEvent, &id)?)
    }

    fn is_expired_here(&self, kind: ClientItemKind, opaque_id: &str) -> rusqlite::Result<bool> {
        self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM expired_client_item WHERE kind = ?1 AND opaque_id = ?2)",
            params![kind.as_str(), opaque_id],
            |r| r.get(0),
        )
    }
}

