//! Deleting a client's material reaches every device (docs/superpowers/specs/2026-11-12-client-controls-design.md
//! sections 4.3 and 13.3, extending docs/superpowers/specs/2026-11-05-deletion-sync-design.md).
//!
//! Before an item is deleted here, [`Store::enqueue_client_item_forgotten`] reads the identity it already has
//! on every device, records it in `forgotten_client_item` and queues a `client_item_forgotten` tombstone
//! carrying only `{kind, opaque_id, forgotten_at_unix_ms}`. A peer that pulls the tombstone deletes its copy and
//! records the identity too, so a row for it that arrives later (an older device's copy, a relay row from
//! before) is dropped at apply, and an unsent outbox row for it is acknowledged unsent. A seed topic goes as a
//! `topic_seed_purged` row (a [`TopicSeedPayload`] with `purged: true` and no name): every device hard-deletes
//! the seed, the seeds merged into it, and their topic content.
//!
//! Tombstones are sent at once. A device on an older build skips them; once it updates, its first pull reads
//! every tombstone row again (sync-engine's catch-up). `client_controls_capability` only feeds the receipt's
//! "N devices still need to update". Nothing here holds a name, a title or text; where an identity could carry
//! one (a calendar UID such as `x@client.com`, and the `calendar:` step keys built from it) the opaque id is a
//! hash of it ([`hashed`]).
//!
//! The identity per kind (`opaque_id`):
//! - email: the Gmail message id (`items.external_id`). Mail is not row-synced; each computer fetches its
//!   own, so the id is the same everywhere and the tombstone always goes.
//! - thread: `{source}:{external_thread_id}`, the Gmail thread every device derives from its mail.
//! - calendar_event: `hashed("{source}:{external_id}")`, of the identity `entity_identity` tags calendar rows with.
//! - capture: `{origin device}:{local_seq}`, the `(device_id, local_seq)` a pulled capture keeps here
//!   (`sync_row_map`); a capture with no `local_seq` never synced.
//! - episode: `[device_id, app_bundle_id, start_ts_unix_ms]` as JSON, the identity hidden-episode marks sync
//!   under; each device derives episodes from the same synced captures.
//! - step: `topic_step.key`; extraction: `topic_extraction.source_key` (both the outbox aggregate ids), hashed
//!   when they name a calendar source.
//! - conversation: none. Ask conversations never leave the device.

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::sync_outbox::{self, ClaimedOutboxOperation, NewOutboxOperation, OutboxOperationKind};
use crate::topics::seeds::TopicSeedPayload;
use crate::{
    json_to_sql_error, now_unix_ms, Store, OUTBOX_CONNECTOR, OUTBOX_ROW_TYPE_CLIENT_ITEM_FORGOTTEN,
    OUTBOX_ROW_TYPE_MEETING_PREP, OUTBOX_ROW_TYPE_TOPIC_EXTRACTION, OUTBOX_ROW_TYPE_TOPIC_PAGE_SNAPSHOT, OUTBOX_ROW_TYPE_TOPIC_ROLE,
    OUTBOX_ROW_TYPE_TOPIC_SEED, OUTBOX_ROW_TYPE_TOPIC_SEED_PURGED, OUTBOX_ROW_TYPE_TOPIC_STEP,
    OUTBOX_ROW_TYPE_TOPIC_VERDICT,
};

/// The `forgotten_client_item.kind` of a purged seed topic (its key is the id).
pub const FORGOTTEN_SEED_KIND: &str = "seed";
/// The `forgotten_client_item.kind` of a forgotten calendar event's meeting prep: `hashed(event id)`.
const FORGOTTEN_PREP_KIND: &str = "prep_event";

/// kv: how many devices on the relay had not announced `client_controls_capability` at the last outbox pass.
const PENDING_DEVICES_KEY: &str = "client_controls_pending_devices";
/// kv: those devices' ids (JSON array), for naming them under the count.
const PENDING_DEVICE_IDS_KEY: &str = "client_controls_pending_device_ids";
/// kv: how many devices on the relay had not announced the topics marker at the last outbox pass (they hold back
/// every seed row, and with it the exclusion setting).
const TOPIC_HELD_DEVICES_KEY: &str = "client_controls_topic_held_devices";
/// kv: devices the user removed from the count (JSON array), until a desktop sync round publishes the removal.
const RETIRE_QUEUE_KEY: &str = "client_controls_retire_queue";

/// How long before an own capture's time one of its relay rows may be stamped. A relay row created earlier
/// is an earlier database's at a reused `local_seq`, and is left.
const CAPTURE_CLEANUP_MARGIN_MS: i64 = 24 * 60 * 60 * 1000;

/// A kind of client material that [`Store::enqueue_client_item_forgotten`] can name on other devices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClientItemKind {
    /// `items` row of kind `email` (local id: `items.id`).
    Email,
    /// `threads` row (local id: `threads.id`).
    Thread,
    /// `items` row of kind `calendar_event` (local id: `items.id`).
    CalendarEvent,
    /// `items` row of kind `capture` (local id: `items.id`). Every capture has one, this device's own (mirrored
    /// from `capture_item`, `source_table = 'capture_item'`) and a peer's alike; its `capture_item` twin goes too.
    Capture,
    /// `episodes` row (local id: `episodes.id`).
    Episode,
    /// `conversation` row (local id: `conversation.id`). Never synced: no tombstone.
    Conversation,
    /// `topic_step` row (local id: its `key`).
    Step,
    /// `topic_extraction` row (local id: its `source_key`).
    Extraction,
    /// `items` row of kind `mcp_item` (local id: `items.id`): an item a local MCP client added. Never
    /// synced: no tombstone. Forgotten here under a keyed hash (`mcp_items::mcp_forget_id`).
    McpItem,
}

impl ClientItemKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Email => "email",
            Self::Thread => "thread",
            Self::CalendarEvent => "calendar_event",
            Self::Capture => "capture",
            Self::Episode => "episode",
            Self::Conversation => "conversation",
            Self::Step => "step",
            Self::Extraction => "extraction",
            Self::McpItem => "mcp_item",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "email" => Self::Email,
            "thread" => Self::Thread,
            "calendar_event" => Self::CalendarEvent,
            "capture" => Self::Capture,
            "episode" => Self::Episode,
            "conversation" => Self::Conversation,
            "step" => Self::Step,
            "extraction" => Self::Extraction,
            "mcp_item" => Self::McpItem,
            _ => return None,
        })
    }
}

/// What [`Store::enqueue_client_item_forgotten`] and [`Store::forget_seed_topic`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TombstoneOutcome {
    /// A tombstone is queued (now, or by an earlier call).
    Queued,
    /// The item has no identity on other devices (a capture that never synced, a conversation): it is only here.
    LocalOnly,
    /// The item has an identity, but this device has no sync id yet, so nothing could be queued. Recorded as
    /// forgotten here. This is permanent for that item: no tombstone is queued later, even once sync is set up
    /// (the item is gone here, so its identity can't be read again). Task 4's receipt must say the item was
    /// "not sent to other devices".
    NoDeviceId,
}

/// A `client_item_forgotten` row on the wire. Ids and a kind only, never a name or text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientItemForgotten {
    pub kind: String,
    pub opaque_id: String,
    pub forgotten_at_unix_ms: i64,
}

pub(crate) fn ensure_client_forget_tables(conn: &Connection) -> rusqlite::Result<()> {
    crate::client_delete::ensure_client_delete_tables(conn)?;
    crate::client_export::ensure_client_export_table(conn)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS forgotten_client_item (
            kind TEXT NOT NULL,
            opaque_id TEXT NOT NULL,
            forgotten_at_unix_ms INTEGER NOT NULL,
            PRIMARY KEY (kind, opaque_id)
        ) WITHOUT ROWID;
        -- This device's own relay rows of forgotten items, still to delete, each created between the two
        -- times (`not_after` is the forget, so a row re-filed later survives). `seq:<row_type>:<local_seq>`
        -- (by primary key), `tag:<hex>` (by sealed entity tag), or briefly `raw:<entity identity>` (an own
        -- calendar event, sealed into a `tag:` at the start of the next sync round). Cleared once deleted.
        -- Emails and calendar events a keep-for-N-months expiry deleted on this device only: ingestion skips
        -- them so a mail resync or calendar poll does not bring them back here. Ids only; never synced.
        CREATE TABLE IF NOT EXISTS expired_client_item (
            kind TEXT NOT NULL,
            opaque_id TEXT NOT NULL,
            expired_at_unix_ms INTEGER NOT NULL,
            PRIMARY KEY (kind, opaque_id)
        ) WITHOUT ROWID;
        CREATE TABLE IF NOT EXISTS client_relay_cleanup (
            target TEXT PRIMARY KEY,
            not_before_unix_ms INTEGER NOT NULL,
            not_after_unix_ms INTEGER NOT NULL
        ) WITHOUT ROWID;",
    )?;
    hash_raw_step_records(conn)?;
    // A pre-release shape of this table (no time bound, raw identities): its rows are dropped.
    let columns: Vec<String> = conn
        .prepare("PRAGMA table_info(client_relay_cleanup)")?
        .query_map([], |r| r.get(1))?
        .collect::<rusqlite::Result<_>>()?;
    if !columns.iter().any(|c| c == "not_after_unix_ms") {
        conn.execute_batch(
            "DROP TABLE client_relay_cleanup;
             CREATE TABLE client_relay_cleanup (
                target TEXT PRIMARY KEY,
                not_before_unix_ms INTEGER NOT NULL,
                not_after_unix_ms INTEGER NOT NULL
             ) WITHOUT ROWID;",
        )?;
    }
    Ok(())
}

/// Rewrites step and step-read records kept by their raw key where the key is now hashed ([`topic_opaque`]): the
/// forgotten records, and this device's tombstone ops in any state (payload and aggregate id): unsent, leased at a
/// crash (sent later in the new form), and delivered (the local copy). A tombstone already on the relay stays as it
/// was sent (pre-release builds only); every device that applies it remembers the opaque id.
fn hash_raw_step_records(conn: &Connection) -> rusqlite::Result<()> {
    let raw: Vec<(String, String, i64)> = conn
        .prepare(
            "SELECT kind, opaque_id, forgotten_at_unix_ms FROM forgotten_client_item \
             WHERE kind IN ('step', 'extraction') AND substr(opaque_id, 1, 2) != 'h:'",
        )?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (kind, id, at) in raw {
        let opaque = topic_opaque(&id);
        if opaque != id {
            record_forgotten(conn, &kind, &opaque, at)?;
            conn.execute("DELETE FROM forgotten_client_item WHERE kind = ?1 AND opaque_id = ?2", params![kind, id])?;
        }
    }
    // On a new database the outbox comes later in the schema setup: nothing queued yet.
    let outbox: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'sync_outbox')",
        [],
        |r| r.get(0),
    )?;
    if !outbox {
        return Ok(());
    }
    let unsent: Vec<(String, String)> = conn
        .prepare(
            "SELECT operation_id, payload_json FROM sync_outbox WHERE row_type = ?1",
        )?
        .query_map([OUTBOX_ROW_TYPE_CLIENT_ITEM_FORGOTTEN], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (op, payload) in unsent {
        let Ok(mut t) = serde_json::from_str::<ClientItemForgotten>(&payload) else { continue };
        if !matches!(t.kind.as_str(), "step" | "extraction") {
            continue;
        }
        let opaque = topic_opaque(&t.opaque_id);
        if opaque == t.opaque_id {
            continue;
        }
        t.opaque_id = opaque;
        conn.execute(
            "UPDATE sync_outbox SET payload_json = ?2, aggregate_id = ?3 WHERE operation_id = ?1",
            params![op, serde_json::to_string(&t).map_err(json_to_sql_error)?, format!("{}:{}", t.kind, t.opaque_id)],
        )?;
    }
    Ok(())
}

/// The opaque form of an identity that may carry a domain or a name: `h:` and SHA-256 hex. Fixed length.
pub fn hashed(raw: &str) -> String {
    format!("h:{}", hex::encode(Sha256::digest(raw.as_bytes())))
}

/// Whether a step key ends in a hash of the step's text (`topics::steps`: `{source_key}#h{16 hex}`, a meeting's
/// action item).
fn carries_text_hash(raw: &str) -> bool {
    raw.rsplit_once("#h").is_some_and(|(_, h)| h.len() == 16 && h.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// The opaque id of a step key or step-read source key: hashed when it names a calendar source (its id is a
/// calendar UID) or carries a hash of the step's text (X6 final review, Major 3: no hash of client text goes into
/// a tombstone or a forgotten record), else as is. Every device derives the same id from its own copy of the key.
/// Inherent limit: the hash is not keyed (the tombstone is matched on devices that hold no shared secret at
/// enqueue time), so someone holding the source key and guessing the step's exact text could test a guess.
fn topic_opaque(raw: &str) -> String {
    if raw.starts_with("h:") {
        raw.to_string()
    } else if raw.starts_with("calendar:") || carries_text_hash(raw) {
        hashed(raw)
    } else {
        raw.to_string()
    }
}

/// The opaque id a step (`topic_step.key`) or extraction (`source_key`) key has in a tombstone.
pub fn topic_key_opaque_id(raw: &str) -> String {
    topic_opaque(raw)
}

fn bad_id() -> rusqlite::Error {
    // No id in the message: a step key can carry a source key.
    crate::topics::seeds::invalid("client item id is not a row id")
}

fn row_id(local_id: &str) -> rusqlite::Result<i64> {
    local_id.parse().map_err(|_| bad_id())
}

/// The identity of local item `local_id` on every device, `None` when it has none (never synced).
/// `Err(QueryReturnedNoRows)` when there is no such row: the caller must ask before deleting it.
pub(crate) fn identity(conn: &Connection, kind: ClientItemKind, local_id: &str) -> rusqlite::Result<Option<String>> {
    match kind {
        ClientItemKind::Email => {
            let (source, ext): (Option<String>, Option<String>) = conn.query_row(
                "SELECT source, external_id FROM items WHERE id = ?1 AND kind = 'email'",
                [row_id(local_id)?],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            Ok(ext.filter(|_| source.as_deref() == Some("gmail")))
        }
        ClientItemKind::Thread => {
            let (source, ext): (String, String) = conn.query_row(
                "SELECT source, external_thread_id FROM threads WHERE id = ?1",
                [row_id(local_id)?],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            Ok(Some(format!("{source}:{ext}")))
        }
        ClientItemKind::CalendarEvent => {
            let (source, ext): (Option<String>, Option<String>) = conn.query_row(
                "SELECT source, external_id FROM items WHERE id = ?1 AND kind = 'calendar_event'",
                [row_id(local_id)?],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            Ok(source.zip(ext).map(|(s, x)| calendar_id(&s, &x)))
        }
        ClientItemKind::Capture => {
            let (device, seq): (String, Option<i64>) = conn.query_row(
                "SELECT device_id, local_seq FROM items WHERE id = ?1 AND kind = 'capture'",
                [row_id(local_id)?],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            Ok(seq.map(|s| capture_id(&device, s)))
        }
        ClientItemKind::Episode => {
            let (device, app, start): (String, String, i64) = conn.query_row(
                "SELECT device_id, app_bundle_id, start_ts_unix_ms FROM episodes WHERE id = ?1",
                [row_id(local_id)?],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
            Ok(Some(episode_id(&device, &app, start)?))
        }
        ClientItemKind::Conversation => {
            conn.query_row(
                "SELECT 1 FROM conversation WHERE id = ?1",
                [row_id(local_id)?],
                |_| Ok(()),
            )?;
            Ok(None)
        }
        // Never on another device: no identity there (the forgotten record is the keyed hash).
        ClientItemKind::McpItem => {
            conn.query_row(
                "SELECT 1 FROM items WHERE id = ?1 AND kind = 'mcp_item'",
                [row_id(local_id)?],
                |_| Ok(()),
            )?;
            Ok(None)
        }
        ClientItemKind::Step => conn
            .query_row("SELECT key FROM topic_step WHERE key = ?1", [local_id], |r| r.get::<_, String>(0))
            .map(|k| Some(topic_opaque(&k))),
        ClientItemKind::Extraction => conn
            .query_row(
                "SELECT source_key FROM topic_extraction WHERE source_key = ?1",
                [local_id],
                |r| r.get::<_, String>(0),
            )
            .map(|k| Some(topic_opaque(&k))),
    }
}

fn calendar_id(source: &str, external_id: &str) -> String {
    hashed(&format!("{source}:{external_id}"))
}

/// The calendar event id of an occurrence's `items.external_id`, `"{event id}#{start secs}"`
/// (`api::connectors::calendar::calendar_external_id`): what a meeting brief is keyed by.
fn calendar_event_id(external_id: &str) -> &str {
    match external_id.rsplit_once('#') {
        Some((id, secs))
            if !secs.is_empty() && secs.trim_start_matches('-').bytes().all(|b| b.is_ascii_digit()) =>
        {
            id
        }
        _ => external_id,
    }
}

fn capture_id(device: &str, local_seq: i64) -> String {
    format!("{device}:{local_seq}")
}

fn episode_id(device: &str, app: &str, start: i64) -> rusqlite::Result<String> {
    serde_json::to_string(&(device, app, start)).map_err(json_to_sql_error)
}

/// The identity a pulled `items` row (from `origin`, at its `local_seq`) has as a forgettable client item.
pub fn pulled_item_identity(
    origin_device_id: &str,
    local_seq: i64,
    item: &crate::AppliedItem,
) -> Option<(&'static str, String)> {
    match item.kind.as_str() {
        "capture" => Some(("capture", capture_id(origin_device_id, local_seq))),
        "calendar_event" => {
            let (s, x) = item.source.as_deref().zip(item.external_id.as_deref())?;
            Some(("calendar_event", calendar_id(s, x)))
        }
        "email" if item.source.as_deref() == Some("gmail") => {
            Some(("email", item.external_id.clone()?))
        }
        _ => None,
    }
}

/// The identity of a pulled legacy `capture_item` row.
pub fn pulled_capture_identity(origin_device_id: &str, local_seq: i64) -> (&'static str, String) {
    ("capture", capture_id(origin_device_id, local_seq))
}

fn record_forgotten(conn: &Connection, kind: &str, opaque_id: &str, at: i64) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO forgotten_client_item (kind, opaque_id, forgotten_at_unix_ms) VALUES (?1, ?2, ?3)",
        params![kind, opaque_id, at],
    )?;
    Ok(())
}

pub(crate) fn is_forgotten(conn: &Connection, kind: &str, opaque_id: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM forgotten_client_item WHERE kind = ?1 AND opaque_id = ?2)",
        params![kind, opaque_id],
        |r| r.get(0),
    )
}

fn record_cleanup(conn: &Connection, target: &str, not_before: i64, not_after: i64) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO client_relay_cleanup (target, not_before_unix_ms, not_after_unix_ms) VALUES (?1, ?2, ?3)",
        params![target, not_before, not_after],
    )?;
    Ok(())
}

/// How long before an outbox op was delivered its relay row may be stamped.
const DELIVERED_CLEANUP_MARGIN_MS: i64 = 24 * 60 * 60 * 1000;

/// This device's delivered outbox ops of `row_type` matching `condition` (`?1` is `value`): their relay rows
/// are recorded for deletion by primary key, and the ops themselves (whose payloads hold the text) go. An
/// op acknowledged unsent (`dropped:`) has no relay row. Only rows this device sent, so never on a store that
/// only pulls.
pub(crate) fn take_delivered(conn: &Connection, row_type: &str, condition: &str, value: &str, at: i64) -> rusqlite::Result<()> {
    let rows: Vec<(i64, Option<i64>, Option<String>)> = conn
        .prepare(&format!(
            "SELECT local_seq, delivered_at_unix_ms, remote_receipt FROM sync_outbox \
             WHERE state = 'delivered' AND row_type = ?2 AND {condition}"
        ))?
        .query_map(params![value, row_type], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (seq, delivered_at, receipt) in rows {
        if receipt.as_deref().is_some_and(|r| r.starts_with("sync_row:")) {
            let not_before = delivered_at.unwrap_or(0) - DELIVERED_CLEANUP_MARGIN_MS;
            record_cleanup(conn, &format!("seq:{row_type}:{seq}"), not_before, at)?;
        }
    }
    conn.execute(
        &format!("DELETE FROM sync_outbox WHERE state = 'delivered' AND row_type = ?2 AND {condition}"),
        params![value, row_type],
    )?;
    Ok(())
}

/// [`take_delivered`] for the ops named `aggregate_id`.
pub(crate) fn take_delivered_aggregate(conn: &Connection, row_type: &str, aggregate_id: &str, at: i64) -> rusqlite::Result<()> {
    take_delivered(conn, row_type, "aggregate_id = ?1", aggregate_id, at)
}

/// [`take_delivered`] for the ops whose payload files them on topic `topic_key`.
fn take_delivered_on_topic(conn: &Connection, row_type: &str, topic_key: &str, at: i64) -> rusqlite::Result<()> {
    take_delivered(conn, row_type, "json_extract(payload_json, '$.topic_key') = ?1", topic_key, at)
}

/// The local rows `opaque_id` names, by their raw identity, found by hash where it is hashed.
/// Calendar: `(source, external_id)`; step/extraction: the key, in `.0`.
fn local_calendar_events(conn: &Connection, opaque_id: &str) -> rusqlite::Result<Vec<(String, String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT source, external_id, device_id FROM items WHERE kind = 'calendar_event' \
         AND source IS NOT NULL AND external_id IS NOT NULL",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
    let mut out = Vec::new();
    for row in rows {
        let (s, x, d) = row?;
        if calendar_id(&s, &x) == opaque_id {
            out.push((s, x, d));
        }
    }
    Ok(out)
}

/// Keys in `table.column` whose opaque id is `opaque_id`: a point lookup (also an old tombstone's raw key), or a
/// scan of the keys that are hashed (`calendar:` keys, keys ending in a text hash).
fn local_topic_keys(conn: &Connection, table: &str, column: &str, opaque_id: &str) -> rusqlite::Result<Vec<String>> {
    if !opaque_id.starts_with("h:") {
        return Ok(vec![opaque_id.to_string()]);
    }
    let mut stmt = conn.prepare(&format!(
        "SELECT {column} FROM {table} WHERE substr({column}, 1, 9) = 'calendar:' \
         OR {column} GLOB '*#h[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]'"
    ))?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    let mut out = Vec::new();
    for row in rows {
        let k = row?;
        if hashed(&k) == opaque_id {
            out.push(k);
        }
    }
    Ok(out)
}

/// Strips a meeting prep key's `@{start}` suffix (`prep::prep_key`), leaving the calendar event id.
fn prep_event_id(event_id: &str) -> &str {
    match event_id.rsplit_once('@') {
        Some((id, start)) if !start.is_empty() && start.bytes().all(|b| b.is_ascii_digit()) => id,
        _ => event_id,
    }
}

/// Deletes this device's copy of the item `opaque_id` names, its unsent outbox rows, and records the relay
/// rows this device pushed for it for deletion (`own`: this device's id). Returns the rows removed.
fn delete_local(
    conn: &Connection,
    own: Option<&str>,
    kind: ClientItemKind,
    opaque_id: &str,
    at: i64,
) -> rusqlite::Result<usize> {
    Ok(match kind {
        ClientItemKind::Email => conn.execute(
            "DELETE FROM items WHERE kind = 'email' AND source = 'gmail' AND external_id = ?1",
            [opaque_id],
        )?,
        ClientItemKind::Thread => {
            let Some((source, ext)) = opaque_id.split_once(':') else {
                return Ok(0);
            };
            let ids: Vec<i64> = conn
                .prepare("SELECT id FROM threads WHERE source = ?1 AND external_thread_id = ?2")?
                .query_map(params![source, ext], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            for id in &ids {
                conn.execute("DELETE FROM thread_priority WHERE thread_id = ?1", [id])?;
                conn.execute(
                    "DELETE FROM jobs WHERE (job_type = 'thread_summarize' AND content_hash = ?1) \
                     OR (job_type = 'thread_priority' AND content_hash = ?2)",
                    params![format!("thread_summarize:{id}"), format!("thread_priority:{id}")],
                )?;
                conn.execute("DELETE FROM threads WHERE id = ?1", [id])?;
            }
            ids.len()
        }
        ClientItemKind::CalendarEvent => {
            let mut removed = 0;
            for (source, ext, device) in local_calendar_events(conn, opaque_id)? {
                calendar_side_rows(conn, &source, &ext, own == Some(device.as_str()), at)?;
                removed += conn.execute(
                    "DELETE FROM items WHERE kind = 'calendar_event' AND source = ?1 AND external_id = ?2",
                    params![source, ext],
                )?;
            }
            removed
        }
        ClientItemKind::Capture => {
            let Some((device, seq)) = opaque_id.rsplit_once(':') else {
                return Ok(0);
            };
            let Ok(seq) = seq.parse::<i64>() else {
                return Ok(0);
            };
            let found: Option<(i64, Option<String>)> = conn
                .query_row(
                    "SELECT ts_unix_ms, app_bundle_id FROM items WHERE kind = 'capture' AND device_id = ?1 AND local_seq = ?2",
                    params![device, seq],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if let Some((ts, app)) = &found {
                // The episode built over it: its summary is text from the capture.
                let episodes: Vec<i64> = conn
                    .prepare(
                        "SELECT id FROM episodes WHERE device_id = ?1 AND app_bundle_id = COALESCE(?2, '(unknown)') \
                         AND start_ts_unix_ms <= ?3 AND end_ts_unix_ms >= ?3",
                    )?
                    .query_map(params![device, app, ts], |r| r.get(0))?
                    .collect::<rusqlite::Result<_>>()?;
                for id in episodes {
                    delete_episode(conn, id)?;
                }
                if own == Some(device) {
                    own_capture_cleanup(conn, seq, *ts, at)?;
                }
            }
            conn.execute(
                "DELETE FROM capture_item WHERE device_id = ?1 AND local_seq = ?2",
                params![device, seq],
            )? + conn.execute(
                "DELETE FROM items WHERE kind = 'capture' AND device_id = ?1 AND local_seq = ?2",
                params![device, seq],
            )?
        }
        ClientItemKind::Episode => {
            let Ok((device, app, start)) = serde_json::from_str::<(String, String, i64)>(opaque_id) else {
                return Ok(0);
            };
            let ids: Vec<i64> = conn
                .prepare(
                    "SELECT id FROM episodes WHERE device_id = ?1 AND app_bundle_id = ?2 AND start_ts_unix_ms = ?3",
                )?
                .query_map(params![device, app, start], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            for id in &ids {
                delete_episode(conn, *id)?;
            }
            ids.len()
        }
        // Never synced: no peer names one.
        ClientItemKind::Conversation | ClientItemKind::McpItem => 0,
        ClientItemKind::Step => {
            let mut removed = 0;
            for key in local_topic_keys(conn, "topic_step", "key", opaque_id)? {
                cancel_unsent(conn, OUTBOX_ROW_TYPE_TOPIC_STEP, &key)?;
                take_delivered_aggregate(conn, OUTBOX_ROW_TYPE_TOPIC_STEP, &key, at)?;
                removed += conn.execute("DELETE FROM topic_step WHERE key = ?1", [&key])?;
            }
            removed
        }
        ClientItemKind::Extraction => {
            let mut removed = 0;
            for key in local_topic_keys(conn, "topic_extraction", "source_key", opaque_id)? {
                cancel_unsent(conn, OUTBOX_ROW_TYPE_TOPIC_EXTRACTION, &key)?;
                take_delivered_aggregate(conn, OUTBOX_ROW_TYPE_TOPIC_EXTRACTION, &key, at)?;
                removed += conn.execute("DELETE FROM topic_extraction WHERE source_key = ?1", [&key])?;
            }
            removed
        }
    })
}

pub(crate) fn delete_episode(conn: &Connection, id: i64) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM jobs WHERE job_type = 'episode_summarize' AND content_hash = ?1",
        [format!("episode_summarize:{id}")],
    )?;
    conn.execute("DELETE FROM episodes WHERE id = ?1", [id])?;
    Ok(())
}

/// This device's own capture's relay rows (`items`, or legacy `capture_item`, at its `local_seq`).
fn own_capture_cleanup(conn: &Connection, seq: i64, ts: i64, at: i64) -> rusqlite::Result<()> {
    for row_type in ["items", "capture_item"] {
        record_cleanup(conn, &format!("seq:{row_type}:{seq}"), ts - CAPTURE_CLEANUP_MARGIN_MS, at)?;
    }
    Ok(())
}

/// A calendar event's rows beside it: its meeting briefs and, when `own` (this device pushed the event), its
/// relay rows. A brief is keyed by the event id (`prep::prep_key`: `{event id}@{start ms}`, or the bare id),
/// so every occurrence of a recurring event loses its brief: briefs are per series here. Briefs another
/// device wrote for the same meeting and this device copied under its own id (`alias_synced_meeting_prep`:
/// same start and title) go too. The event id is remembered hashed, so a late brief is dropped.
fn calendar_side_rows(conn: &Connection, source: &str, ext: &str, own: bool, at: i64) -> rusqlite::Result<()> {
    let id = calendar_event_id(ext);
    record_forgotten(conn, FORGOTTEN_PREP_KIND, &hashed(id), at)?;
    const MATCH: &str = "(event_id = ?1 OR substr(event_id, 1, length(?1) + 1) = ?1 || '@')";
    let briefs: Vec<(i64, String)> = conn
        .prepare(&format!("SELECT start_unix_ms, title FROM meeting_prep WHERE {MATCH}"))?
        .query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut keys: Vec<String> = conn
        .prepare(&format!("SELECT event_id FROM meeting_prep WHERE {MATCH}"))?
        .query_map([id], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let queued: Vec<String> = conn
        .prepare(
            "SELECT DISTINCT aggregate_id FROM sync_outbox WHERE row_type = ?2 \
             AND (aggregate_id = ?1 OR substr(aggregate_id, 1, length(?1) + 1) = ?1 || '@')",
        )?
        .query_map(params![id, OUTBOX_ROW_TYPE_MEETING_PREP], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    keys.extend(queued);
    keys.sort();
    keys.dedup();
    for key in &keys {
        cancel_unsent(conn, OUTBOX_ROW_TYPE_MEETING_PREP, key)?;
        take_delivered_aggregate(conn, OUTBOX_ROW_TYPE_MEETING_PREP, key, at)?;
    }
    conn.execute(&format!("DELETE FROM meeting_prep WHERE {MATCH}"), [id])?;
    for (start, title) in briefs {
        conn.execute(
            "DELETE FROM meeting_prep WHERE start_unix_ms = ?1 AND lower(trim(title)) = lower(trim(?2))",
            params![start, title],
        )?;
    }
    if own {
        // Raw only until the next sync round seals it into its entity tag (`seal_client_relay_cleanup`).
        record_cleanup(conn, &format!("raw:items:calendar_event:{source}:{ext}"), 0, at)?;
    }
    Ok(())
}

/// [`calendar_side_rows`] under `reach`. `ThisDevice` (an expiry of one old occurrence) takes only the briefs
/// of this occurrence (`start` is its start time), never the series' later ones, and records nothing:
/// no forgotten event id and no relay row.
fn calendar_side_rows_in(
    conn: &Connection,
    reach: Reach,
    source: &str,
    ext: &str,
    start: i64,
    own: bool,
    at: i64,
) -> rusqlite::Result<()> {
    if reach == Reach::Everywhere {
        return calendar_side_rows(conn, source, ext, own, at);
    }
    let id = calendar_event_id(ext);
    let keys: Vec<String> = conn
        .prepare(
            "SELECT event_id FROM meeting_prep WHERE start_unix_ms = ?2 \
             AND (event_id = ?1 OR substr(event_id, 1, length(?1) + 1) = ?1 || '@')",
        )?
        .query_map(params![id, start], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut all = keys.clone();
    all.push(crate::prep::prep_key(id, start));
    all.sort();
    all.dedup();
    for key in &all {
        drop_unsent(conn, reach, OUTBOX_ROW_TYPE_MEETING_PREP, key)?;
        take_delivered_aggregate_in(conn, reach, OUTBOX_ROW_TYPE_MEETING_PREP, key, at)?;
    }
    for key in keys {
        conn.execute("DELETE FROM meeting_prep WHERE event_id = ?1", [key])?;
    }
    Ok(())
}

/// Drops unsent outbox rows of `row_type` named `aggregate_id` (a queued step would carry its text out).
pub(crate) fn cancel_unsent(conn: &Connection, row_type: &str, aggregate_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM sync_outbox WHERE state IN ('pending', 'blocked') AND row_type = ?1 AND aggregate_id = ?2",
        params![row_type, aggregate_id],
    )?;
    Ok(())
}

/// How far a delete of client material reaches. Spec 4.3: "Retention remains per device and stays local;
/// Forget-style deletes go everywhere."
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// A client delete: a tombstone for every other device, the item recorded as forgotten (a later copy is
    /// dropped), this device's relay rows of it recorded for deletion and its unsent ops cancelled.
    Everywhere,
    /// A keep-for-N-months expiry: deleted on this device only. Nothing is queued, nothing is recorded as
    /// forgotten and no relay row is recorded for deletion; a delivered op's local row (its payload holds the
    /// text) goes. With `sync_on`, an item with an op not yet sent, or an own row the push cursor has not
    /// passed, is refused with [`UnsentOps`] (rolled back, left for a later run) so the other devices still
    /// get it; without sync there is no device to reach, so its unsent ops are dropped.
    ThisDevice { sync_on: bool },
}

/// A [`Reach::ThisDevice`] delete met an item this device has not sent to the other devices yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnsentOps;

impl std::fmt::Display for UnsentOps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("not sent to the other devices yet")
    }
}

impl std::error::Error for UnsentOps {}

pub(crate) fn unsent_ops() -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(UnsentOps))
}

/// Whether `e` is [`UnsentOps`]: the item is still here and waits for sync.
pub fn is_unsent_ops(e: &rusqlite::Error) -> bool {
    matches!(e, rusqlite::Error::ToSqlConversionFailure(inner) if inner.is::<UnsentOps>())
}

/// [`cancel_unsent`], or under `ThisDevice { sync_on: true }` a refusal ([`UnsentOps`]) when there is one
/// (pending, blocked or in flight).
pub(crate) fn drop_unsent(conn: &Connection, reach: Reach, row_type: &str, aggregate_id: &str) -> rusqlite::Result<()> {
    drop_unsent_where(conn, reach, row_type, "aggregate_id = ?1", aggregate_id)
}

/// [`drop_unsent`] for the ops of `row_type` matching `condition` (`?1` is `value`).
pub(crate) fn drop_unsent_where(
    conn: &Connection,
    reach: Reach,
    row_type: &str,
    condition: &str,
    value: &str,
) -> rusqlite::Result<()> {
    if reach == (Reach::ThisDevice { sync_on: true }) {
        let any: bool = conn.query_row(
            &format!(
                "SELECT EXISTS (SELECT 1 FROM sync_outbox WHERE state IN ('pending', 'blocked', 'leased') \
                 AND row_type = ?2 AND {condition})"
            ),
            params![value, row_type],
            |r| r.get(0),
        )?;
        return if any { Err(unsent_ops()) } else { Ok(()) };
    }
    conn.execute(
        &format!("DELETE FROM sync_outbox WHERE state IN ('pending', 'blocked') AND row_type = ?2 AND {condition}"),
        params![value, row_type],
    )?;
    Ok(())
}

/// [`take_delivered`] under `reach`: `ThisDevice` deletes the delivered ops here and records no relay row.
pub(crate) fn take_delivered_in(
    conn: &Connection,
    reach: Reach,
    row_type: &str,
    condition: &str,
    value: &str,
    at: i64,
) -> rusqlite::Result<()> {
    match reach {
        Reach::Everywhere => take_delivered(conn, row_type, condition, value, at),
        Reach::ThisDevice { .. } => {
            conn.execute(
                &format!("DELETE FROM sync_outbox WHERE state = 'delivered' AND row_type = ?2 AND {condition}"),
                params![value, row_type],
            )?;
            Ok(())
        }
    }
}

/// [`take_delivered_in`] for the ops named `aggregate_id`.
pub(crate) fn take_delivered_aggregate_in(
    conn: &Connection,
    reach: Reach,
    row_type: &str,
    aggregate_id: &str,
    at: i64,
) -> rusqlite::Result<()> {
    take_delivered_in(conn, reach, row_type, "aggregate_id = ?1", aggregate_id, at)
}

/// Every seed key whose `merged_into_key` chain reaches `key`, `key` first.
pub(crate) fn merge_chain(conn: &Connection, key: &str) -> rusqlite::Result<Vec<String>> {
    conn.prepare(
        "WITH RECURSIVE c(k, d) AS (SELECT ?1, 0 UNION \
         SELECT s.key, c.d + 1 FROM topic_seed s JOIN c ON s.merged_into_key = c.k WHERE c.d < 64) \
         SELECT k FROM c ORDER BY d",
    )?
    .query_map([key], |r| r.get(0))?
    .collect()
}

/// Hard-deletes seed topic `key`, every seed merged into it (directly or down a chain), and their topic
/// content here, keeping nothing that names them; records each key as forgotten and its own relay rows for
/// deletion. Returns the keys purged, `key` first.
pub(crate) fn purge_seed_tx(conn: &Connection, key: &str, at: i64) -> rusqlite::Result<Vec<String>> {
    let keys = merge_chain(conn, key)?;
    // Every name of the chain, read before any of it goes: what a shared retention stub is matched against.
    let mut chain_terms: Vec<String> = Vec::new();
    for k in &keys {
        chain_terms.extend(seed_terms(conn, k)?);
    }
    clear_model_error_notes(conn)?;
    for k in &keys {
        // No trace of the client's names stays in kept word fingerprints (X6 fix round 5, N1).
        let terms = seed_terms(conn, k)?;
        crate::topics::exclusion::purge_input_words_naming_tx(conn, &terms)?;
        // Nor of its calendar ranges kept for exclusion (Task 8's `excluded_event_span`).
        crate::topics::exclusion::purge_event_spans_of_tx(conn, k)?;
        purge_one_seed(conn, k, &chain_terms, &keys)?;
        record_forgotten(conn, FORGOTTEN_SEED_KIND, k, at)?;
        // A client delete's pending record for this key is moot once the seed is gone.
        conn.execute(
            "DELETE FROM client_delete_pending WHERE run = ?1",
            [Store::client_delete_run_id(k)],
        )?;
        // So are an expiry's owed work and its count of items expired here only.
        conn.execute(
            "DELETE FROM client_delete_pending WHERE run = ?1",
            [Store::client_expiry_run_id(k)],
        )?;
        Store::clear_client_expired_count_tx(conn, k)?;
        // Its export records: the folders stay the user's (the delete's receipt listed them).
        crate::client_export::forget_client_exports_tx(conn, k)?;
        // So are its keep-for-N-months decisions (opaque keys, but they would outlive the client).
        conn.execute(
            "DELETE FROM kv_meta WHERE substr(key, 1, length(?1)) = ?1",
            [Store::client_retention_prefix(k)],
        )?;
    }
    crate::topics::aliases::bump_seed_version(conn)?;
    Ok(keys)
}

/// kv: a peer's client delete removed rows here and the full-text index clean-up has not run since.
const CLEANUP_OWED_KEY: &str = "client_peer_cleanup_owed";

/// Records that [`Store::run_owed_client_cleanup`] has work (in the applying transaction).
pub(crate) fn owe_client_cleanup(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO kv_meta (key, value) VALUES (?1, '1') ON CONFLICT(key) DO UPDATE SET value = '1'",
        [CLEANUP_OWED_KEY],
    )?;
    Ok(())
}

/// Drops every model error note (`kv_meta` `last_error:<job type>`, [`Store::record_summarize_error`]): a failed
/// summary's error quotes up to 500 characters of the model's reply, which can carry a client's text (X6 final
/// review, Major 1). All of them, not those naming the client: the text is escaped and cut, so a name match can
/// miss. Diagnostics only; the next failure writes a new one. Run by every client delete and expiry here, and by
/// every device applying a client's purge.
pub(crate) fn clear_model_error_notes(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM kv_meta WHERE substr(key, 1, 11) = 'last_error:'", [])?;
    Ok(())
}

/// Retention stubs (`stub = 1`, `ref_id` NULL: a meeting or episode past retention whose source keeps its title,
/// text, vector and mentions) filed in `topics` (X6 final review, B1). The client inventory cannot list them (no
/// item to show), so they go here, with everything the `topic_source_ad` trigger takes along. A stub also held by
/// another live client (a live seed outside `chain`, directly or through an unseeded topic merged into it) is that
/// client's too: it goes only when its title or text names one of `terms` (whole words), and is otherwise left to
/// the other client. Auto topics and deleted seeds hold nobody's claim (14a review, B1-residual). A `removed`
/// membership is not a filing.
fn purge_stub_members(conn: &Connection, topics: &[i64], terms: &[String], chain: &[String]) -> rusqlite::Result<()> {
    if topics.is_empty() {
        return Ok(());
    }
    let list = topics.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
    // Topic ids are integers: safe to write into the SQL.
    let stubs: Vec<(i64, String, Option<String>)> = conn
        .prepare(&format!(
            "SELECT s.id, s.title, x.text \
             FROM topic_source s LEFT JOIN topic_source_text x ON x.source_id = s.id \
             WHERE s.stub = 1 AND s.ref_id IS NULL AND s.id IN \
               (SELECT source_id FROM topic_member WHERE origin != 'removed' AND topic_id IN ({list}))"
        ))?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (id, title, text) in stubs {
        // Its other filings, each resolved through `merged_into` to the seed topic it was merged into (as
        // `purge_one_seed` builds its own set): shared only when one is another live client's. An auto topic, or a
        // deleted seed's, is no client's: the stub goes from it too.
        let others: Vec<i64> = conn
            .prepare(&format!(
                "SELECT topic_id FROM topic_member WHERE source_id = ?1 AND origin != 'removed' AND topic_id NOT IN ({list})"
            ))?
            .query_map([id], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let mut shared = false;
        for t in others {
            let seed: Option<(String, bool)> = conn
                .query_row(
                    "WITH RECURSIVE up(id, d) AS (SELECT ?1, 0 UNION \
                     SELECT topic.merged_into, up.d + 1 FROM topic JOIN up ON topic.id = up.id \
                     WHERE topic.merged_into IS NOT NULL AND up.d < 64 \
                       AND up.id NOT IN (SELECT topic_id FROM topic_seed)) \
                     SELECT s.key, s.deleted FROM up JOIN topic_seed s ON s.topic_id = up.id ORDER BY up.d LIMIT 1",
                    [t],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if seed.is_some_and(|(key, deleted)| !deleted && !chain.contains(&key)) {
                shared = true;
                break;
            }
        }
        let names = |t: &str| crate::words::names_any(t, terms);
        if !shared || names(&title) || text.as_deref().is_some_and(names) {
            conn.execute("DELETE FROM topic_source WHERE id = ?1", [id])?;
        }
    }
    Ok(())
}

/// Seed `key`'s name and aliases (empty when it is gone). An alias list that does not parse is an
/// error: dropping it would leave its names behind.
fn seed_terms(conn: &Connection, key: &str) -> rusqlite::Result<Vec<String>> {
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT t.name, s.aliases FROM topic_seed s JOIN topic t ON t.id = s.topic_id WHERE s.key = ?1",
            [key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((name, aliases)) = row else { return Ok(Vec::new()) };
    let aliases: Vec<String> = serde_json::from_str(&aliases).map_err(crate::json_to_sql_error)?;
    Ok(std::iter::once(name).chain(aliases).collect())
}

/// One seed record: its topic (and the unseeded topics merged into it), members, stats, steps, step reads,
/// verdicts, role, page snapshot and client controls, plus their unsent sync rows. Members go back to be
/// filed again, except retention stubs ([`purge_stub_members`]). `terms`: the chain's names.
fn purge_one_seed(conn: &Connection, key: &str, terms: &[String], chain: &[String]) -> rusqlite::Result<()> {
    let topic_id: Option<i64> = conn
        .query_row("SELECT topic_id FROM topic_seed WHERE key = ?1", [key], |r| r.get(0))
        .optional()?;
    if let Some(id) = topic_id {
        let topics: Vec<i64> = conn
            .prepare(
                "WITH RECURSIVE t(id, depth) AS (SELECT ?1, 0 UNION \
                 SELECT topic.id, t.depth + 1 FROM topic JOIN t ON topic.merged_into = t.id \
                 WHERE t.depth < 64 AND topic.id NOT IN (SELECT topic_id FROM topic_seed)) \
                 SELECT id FROM t",
            )?
            .query_map([id], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        purge_stub_members(conn, &topics, terms, chain)?;
        for t in topics {
            conn.execute(
                "UPDATE topic_source SET assigned_at_unix_ms = NULL WHERE id IN \
                 (SELECT source_id FROM topic_member WHERE topic_id = ?1)",
                [t],
            )?;
            for sql in [
                "DELETE FROM topic_member WHERE topic_id = ?1",
                "DELETE FROM topic_stats WHERE topic_id = ?1",
                "DELETE FROM topic_idea WHERE topic_id = ?1",
                "DELETE FROM topic_seed WHERE topic_id = ?1",
                "DELETE FROM topic WHERE id = ?1",
            ] {
                conn.execute(sql, [t])?;
            }
        }
    }
    // The relay rows this device sent about the topic: the seed record (its name), role and snapshot, and its
    // steps, step reads and verdicts filed on it. A peer's rows are its own to delete.
    let at = now_unix_ms();
    for row_type in [OUTBOX_ROW_TYPE_TOPIC_SEED, OUTBOX_ROW_TYPE_TOPIC_ROLE, OUTBOX_ROW_TYPE_TOPIC_PAGE_SNAPSHOT] {
        take_delivered_aggregate(conn, row_type, key, at)?;
    }
    for row_type in [OUTBOX_ROW_TYPE_TOPIC_STEP, OUTBOX_ROW_TYPE_TOPIC_EXTRACTION, OUTBOX_ROW_TYPE_TOPIC_VERDICT] {
        take_delivered_on_topic(conn, row_type, key, at)?;
    }
    for sql in [
        "DELETE FROM topic_step WHERE topic_key = ?1",
        "DELETE FROM topic_extraction WHERE topic_key = ?1",
        "DELETE FROM topic_verdict WHERE topic_key = ?1",
        "DELETE FROM topic_role WHERE topic_key = ?1",
        "DELETE FROM topic_page_snapshot WHERE topic_key = ?1",
        "DELETE FROM client_control WHERE topic_key = ?1",
    ] {
        conn.execute(sql, [key])?;
    }
    conn.execute(
        "DELETE FROM sync_outbox WHERE state IN ('pending', 'blocked') AND ( \
         (row_type IN (?2, ?3, ?4) AND aggregate_id = ?1) \
         OR (row_type IN (?5, ?6, ?7) AND json_extract(payload_json, '$.topic_key') = ?1))",
        params![
            key,
            OUTBOX_ROW_TYPE_TOPIC_SEED,
            OUTBOX_ROW_TYPE_TOPIC_ROLE,
            OUTBOX_ROW_TYPE_TOPIC_PAGE_SNAPSHOT,
            OUTBOX_ROW_TYPE_TOPIC_STEP,
            OUTBOX_ROW_TYPE_TOPIC_EXTRACTION,
            OUTBOX_ROW_TYPE_TOPIC_VERDICT,
        ],
    )?;
    Ok(())
}

impl Store {
    /// Queues a tombstone so every other device deletes its copy of this client item, and records the item
    /// as forgotten here, so a copy pulled later is dropped. Call it BEFORE deleting the item (it reads the
    /// item's identity from its row), ideally in the same transaction ([`Self::enqueue_client_item_forgotten_tx`]).
    /// It also drops the item's unsent outbox rows and records this device's relay rows of it for deletion.
    ///
    /// `local_id` is the row id as a decimal string, or the key for a step (`topic_step.key`) or an extraction
    /// (`topic_extraction.source_key`): the `id` the client inventory lists. See [`TombstoneOutcome`]. A missing
    /// row is `Err(QueryReturnedNoRows)`, never a silent no-op.
    pub fn enqueue_client_item_forgotten(
        &self,
        kind: ClientItemKind,
        local_id: &str,
    ) -> rusqlite::Result<TombstoneOutcome> {
        let tx = self.conn.unchecked_transaction()?;
        let outcome = self.enqueue_client_item_forgotten_tx(&tx, kind, local_id)?;
        tx.commit()?;
        Ok(outcome)
    }

    /// [`Self::enqueue_client_item_forgotten`] in the caller's transaction (the delete's own).
    pub(crate) fn enqueue_client_item_forgotten_tx(
        &self,
        tx: &Transaction<'_>,
        kind: ClientItemKind,
        local_id: &str,
    ) -> rusqlite::Result<TombstoneOutcome> {
        if kind == ClientItemKind::McpItem {
            // Local only: the keyed record keeps the next `add_items` of it out; nothing is queued.
            let (source, ext): (String, String) = tx.query_row(
                "SELECT source, external_id FROM items WHERE id = ?1 AND kind = 'mcp_item'",
                [row_id(local_id)?],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            let label = crate::mcp_items::label_of_source(&source).unwrap_or_default();
            let opaque_id = crate::mcp_items::mcp_forget_id(tx, label, &ext)?;
            record_forgotten(tx, kind.as_str(), &opaque_id, now_unix_ms())?;
            return Ok(TombstoneOutcome::LocalOnly);
        }
        let Some(opaque_id) = identity(tx, kind, local_id)? else {
            return Ok(TombstoneOutcome::LocalOnly);
        };
        let now = now_unix_ms();
        record_forgotten(tx, kind.as_str(), &opaque_id, now)?;
        self.prepare_local_delete(tx, kind, local_id, now, Reach::Everywhere)?;
        let payload = ClientItemForgotten {
            kind: kind.as_str().to_string(),
            opaque_id,
            forgotten_at_unix_ms: now,
        };
        let aggregate_id = format!("{}:{}", payload.kind, payload.opaque_id);
        self.enqueue_tombstone_tx(tx, OUTBOX_ROW_TYPE_CLIENT_ITEM_FORGOTTEN, aggregate_id, &payload)
    }

    /// Records `opaque_id` of `kind` as forgotten and queues its tombstone, for an item whose identity is
    /// already known (a legacy `capture_item` row with no `items` twin).
    pub(crate) fn enqueue_opaque_tombstone_tx(
        store: &Store,
        tx: &Transaction<'_>,
        kind: ClientItemKind,
        opaque_id: &str,
    ) -> rusqlite::Result<TombstoneOutcome> {
        let now = now_unix_ms();
        record_forgotten(tx, kind.as_str(), opaque_id, now)?;
        let payload = ClientItemForgotten {
            kind: kind.as_str().to_string(),
            opaque_id: opaque_id.to_string(),
            forgotten_at_unix_ms: now,
        };
        let aggregate_id = format!("{}:{}", payload.kind, payload.opaque_id);
        store.enqueue_tombstone_tx(tx, OUTBOX_ROW_TYPE_CLIENT_ITEM_FORGOTTEN, aggregate_id, &payload)
    }

    /// What the delete engine's own delete doesn't cover, for the item still in place: its unsent outbox rows,
    /// this device's relay rows recorded for deletion (not under `ThisDevice`), and a calendar event's meeting
    /// prep.
    fn prepare_local_delete(
        &self,
        tx: &Transaction<'_>,
        kind: ClientItemKind,
        local_id: &str,
        at: i64,
        reach: Reach,
    ) -> rusqlite::Result<()> {
        let own = self.own_device_id();
        match kind {
            ClientItemKind::CalendarEvent => {
                let (s, x, d, ts): (String, String, String, i64) = tx.query_row(
                    "SELECT source, external_id, device_id, ts_unix_ms FROM items WHERE id = ?1",
                    [row_id(local_id)?],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )?;
                calendar_side_rows_in(tx, reach, &s, &x, ts, own.as_deref() == Some(d.as_str()), at)
            }
            ClientItemKind::Capture => {
                let (device, seq, ts): (String, Option<i64>, i64) = tx.query_row(
                    "SELECT device_id, local_seq, ts_unix_ms FROM items WHERE id = ?1",
                    [row_id(local_id)?],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )?;
                if let (Reach::Everywhere, Some(seq)) = (reach, seq) {
                    if own.as_deref() == Some(device.as_str()) {
                        own_capture_cleanup(tx, seq, ts, at)?;
                    }
                }
                Ok(())
            }
            ClientItemKind::Step => {
                drop_unsent(tx, reach, OUTBOX_ROW_TYPE_TOPIC_STEP, local_id)?;
                take_delivered_aggregate_in(tx, reach, OUTBOX_ROW_TYPE_TOPIC_STEP, local_id, at)
            }
            ClientItemKind::Extraction => {
                drop_unsent(tx, reach, OUTBOX_ROW_TYPE_TOPIC_EXTRACTION, local_id)?;
                take_delivered_aggregate_in(tx, reach, OUTBOX_ROW_TYPE_TOPIC_EXTRACTION, local_id, at)
            }
            _ => Ok(()),
        }
    }

    /// [`Self::enqueue_client_item_forgotten_tx`] under `reach`. `ThisDevice` queues nothing and records
    /// nothing as forgotten: it refuses an own row not pushed yet (with sync on), remembers an email's or
    /// calendar event's id in `expired_client_item` so ingestion does not bring it back here, and prepares
    /// the local delete. A missing row is `Err(QueryReturnedNoRows)`.
    pub(crate) fn forget_client_item_in_tx(
        &self,
        tx: &Transaction<'_>,
        kind: ClientItemKind,
        local_id: &str,
        reach: Reach,
    ) -> rusqlite::Result<TombstoneOutcome> {
        let Reach::ThisDevice { sync_on } = reach else {
            return self.enqueue_client_item_forgotten_tx(tx, kind, local_id);
        };
        let opaque_id = identity(tx, kind, local_id)?;
        // Mail is not row-synced (each computer fetches its own); captures and calendar events are.
        if matches!(kind, ClientItemKind::CalendarEvent | ClientItemKind::Capture) {
            let row: (String, Option<i64>) = tx.query_row(
                "SELECT device_id, local_seq FROM items WHERE id = ?1",
                [row_id(local_id)?],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            if self.expiry_must_check_push(sync_on, &row.0)? {
                let cursor: i64 =
                    tx.query_row("SELECT last_pushed_seq FROM sync_push_cursor WHERE id = 1", [], |r| r.get(0))?;
                if row.1.is_some_and(|seq| seq > cursor) {
                    return Err(unsent_ops());
                }
            }
        }
        let now = now_unix_ms();
        if let (ClientItemKind::Email | ClientItemKind::CalendarEvent, Some(id)) = (kind, &opaque_id) {
            tx.execute(
                "INSERT OR IGNORE INTO expired_client_item (kind, opaque_id, expired_at_unix_ms) VALUES (?1, ?2, ?3)",
                params![kind.as_str(), id, now],
            )?;
        }
        self.prepare_local_delete(tx, kind, local_id, now, reach)?;
        Ok(TombstoneOutcome::LocalOnly)
    }

    /// For a local-only expiry of a synced row from `row_device`: whether it is this device's own (so the push
    /// cursor decides whether it was sent). With sync on and no real own device id (none, or the legacy
    /// placeholder) an own row cannot be told from a peer's: refused as unsent ([`UnsentOps`], "waiting to
    /// sync"), so the only copy is never deleted (X6 final review, Major 21). With sync off: never.
    pub(crate) fn expiry_must_check_push(&self, sync_on: bool, row_device: &str) -> rusqlite::Result<bool> {
        if !sync_on {
            return Ok(false);
        }
        match self.own_device_id().filter(|id| !id.trim().is_empty() && id != crate::LEGACY_SPIKE_DEVICE) {
            Some(own) => Ok(own == row_device),
            None => Err(unsent_ops()),
        }
    }

    /// Queues `row_type`'s tombstone once per `aggregate_id` (a re-run finds it queued).
    fn enqueue_tombstone_tx(
        &self,
        tx: &Transaction<'_>,
        row_type: &str,
        aggregate_id: String,
        payload: &impl Serialize,
    ) -> rusqlite::Result<TombstoneOutcome> {
        let Some(own) = self.own_device_id() else {
            return Ok(TombstoneOutcome::NoDeviceId);
        };
        let queued: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM sync_outbox WHERE row_type = ?1 AND aggregate_id = ?2)",
            params![row_type, aggregate_id],
            |r| r.get(0),
        )?;
        if !queued {
            sync_outbox::enqueue_outbox_tx(
                tx,
                NewOutboxOperation {
                    device_id: own,
                    local_seq: Self::next_local_seq_tx(tx)?,
                    row_type: row_type.to_string(),
                    aggregate_type: row_type.to_string(),
                    aggregate_id,
                    operation_kind: OutboxOperationKind::Tombstone,
                    payload_json: serde_json::to_string(payload).map_err(json_to_sql_error)?,
                    connector: OUTBOX_CONNECTOR.to_string(),
                },
                now_unix_ms(),
            )?;
        }
        Ok(TombstoneOutcome::Queued)
    }

    /// Applies a peer's `client_item_forgotten`: deletes this device's copy (if any) and records the identity,
    /// so a copy arriving later is dropped. Never queues a tombstone. Returns the rows removed; a kind this
    /// build doesn't know is ignored (0).
    pub fn apply_client_item_forgotten(&self, t: &ClientItemForgotten) -> rusqlite::Result<usize> {
        let Some(kind) = ClientItemKind::parse(&t.kind) else {
            return Ok(0);
        };
        // Deleted text is overwritten, as on the deleting device, and the index pages are cleaned later in the
        // round ([`Self::run_owed_client_cleanup`]).
        let secure_before = self.set_secure_delete(true).ok();
        let r = self.apply_client_item_forgotten_inner(kind, t);
        if let Some(before) = secure_before {
            let _ = self.set_secure_delete(before);
        }
        r
    }

    fn apply_client_item_forgotten_inner(&self, kind: ClientItemKind, t: &ClientItemForgotten) -> rusqlite::Result<usize> {
        let tx = self.conn.unchecked_transaction()?;
        // Remembered by the opaque id, also when an older tombstone named a step by its raw key.
        let remembered = match kind {
            ClientItemKind::Step | ClientItemKind::Extraction => topic_opaque(&t.opaque_id),
            _ => t.opaque_id.clone(),
        };
        record_forgotten(&tx, kind.as_str(), &remembered, t.forgotten_at_unix_ms)?;
        let removed = delete_local(&tx, self.own_device_id().as_deref(), kind, &t.opaque_id, t.forgotten_at_unix_ms)?;
        if removed > 0 {
            owe_client_cleanup(&tx)?;
        }
        tx.commit()?;
        Ok(removed)
    }

    /// After a pull round applied a peer's client deletes (or meeting forgets): the clean-up the deleting device
    /// runs at the end of its delete ([`Self::finish_client_delete`]: unused names pruned, full-text indexes merged
    /// so no deleted term stays in their pages, model error notes dropped), with secure delete on and a passive
    /// WAL checkpoint (never waits on readers). Once per round,
    /// only when something was removed; the flag is cleared only after it succeeds, so a failure retries next
    /// round (X6 final review, Major 4). Returns whether it ran. The index merge can take a few seconds on a large
    /// database: the sync engine runs it after the round's pulls, never inside one.
    pub fn run_owed_client_cleanup(&self) -> rusqlite::Result<bool> {
        if self.get_meta(CLEANUP_OWED_KEY)?.is_none() {
            return Ok(false);
        }
        let secure_before = self.set_secure_delete(true).ok();
        // A passive checkpoint: the round holds the store's queue (the Mac's access queue, a Windows store lane),
        // so it never waits on a reader through the busy handler (14a review, Minor 2).
        let r = self.finish_client_delete_with("PASSIVE");
        if let Some(before) = secure_before {
            let _ = self.set_secure_delete(before);
        }
        r?;
        self.delete_meta(CLEANUP_OWED_KEY)?;
        Ok(true)
    }

    /// Whether a client delete (here or on a peer) forgot the item `kind`/`opaque_id` names. `kind` is a
    /// [`ClientItemKind::as_str`] or [`FORGOTTEN_SEED_KIND`].
    pub fn is_client_item_forgotten(&self, kind: &str, opaque_id: &str) -> rusqlite::Result<bool> {
        is_forgotten(&self.conn, kind, opaque_id)
    }

    /// Whether a pulled meeting prep (keyed `prep::prep_key`) is for a calendar event a client delete forgot.
    pub fn is_meeting_prep_forgotten(&self, event_id: &str) -> rusqlite::Result<bool> {
        is_forgotten(&self.conn, FORGOTTEN_PREP_KIND, &hashed(prep_event_id(event_id)))
    }

    /// Whether outbox op `op` is about something a client delete forgot, so it must go unsent: the delete
    /// cancels pending rows, and this closes the window for one leased (in flight) at the time.
    pub fn outbox_op_forgotten(&self, op: &ClaimedOutboxOperation) -> rusqlite::Result<bool> {
        let c = &self.conn;
        let payload = serde_json::from_str::<serde_json::Value>(&op.payload_json).ok();
        let field = |name: &str| payload.as_ref().and_then(|v| v.get(name)?.as_str().map(str::to_string));
        let seed = |k: Option<String>| -> rusqlite::Result<bool> {
            match k {
                Some(k) => is_forgotten(c, FORGOTTEN_SEED_KIND, &k),
                None => Ok(false),
            }
        };
        let email_source = |k: Option<String>| -> rusqlite::Result<bool> {
            match k.as_deref().and_then(|k| k.strip_prefix("email:")) {
                Some(id) => is_forgotten(c, ClientItemKind::Email.as_str(), id),
                None => Ok(false),
            }
        };
        Ok(match op.row_type.as_str() {
            OUTBOX_ROW_TYPE_TOPIC_STEP => {
                is_forgotten(c, ClientItemKind::Step.as_str(), &topic_opaque(&op.aggregate_id))?
                    || seed(field("topic_key"))?
                    || email_source(field("source_key"))?
            }
            OUTBOX_ROW_TYPE_TOPIC_EXTRACTION => {
                is_forgotten(c, ClientItemKind::Extraction.as_str(), &topic_opaque(&op.aggregate_id))?
                    || seed(field("topic_key"))?
                    || email_source(Some(op.aggregate_id.clone()))?
            }
            OUTBOX_ROW_TYPE_TOPIC_VERDICT => {
                seed(field("topic_key"))? || email_source(Some(op.aggregate_id.clone()))?
            }
            OUTBOX_ROW_TYPE_TOPIC_SEED => {
                seed(Some(op.aggregate_id.clone()))? || seed(field("merged_into_key"))?
            }
            OUTBOX_ROW_TYPE_TOPIC_ROLE | OUTBOX_ROW_TYPE_TOPIC_PAGE_SNAPSHOT => {
                seed(Some(op.aggregate_id.clone()))?
            }
            OUTBOX_ROW_TYPE_MEETING_PREP => self.is_meeting_prep_forgotten(&op.aggregate_id)?,
            _ => false,
        })
    }

    /// After op `op` was delivered (its relay row written at `delivered_at`): if a client delete forgot what it
    /// is about meanwhile (it was in flight, past the delete's cancel and the send-time check), its relay row is
    /// recorded for deletion and the op (whose payload holds the text) goes. Returns whether it did.
    pub fn take_delivery_if_forgotten(&self, op: &ClaimedOutboxOperation, delivered_at: i64) -> rusqlite::Result<bool> {
        if !self.outbox_op_forgotten(op)? {
            return Ok(false);
        }
        let tx = self.conn.unchecked_transaction()?;
        record_cleanup(
            &tx,
            &format!("seq:{}:{}", op.row_type, op.local_seq),
            delivered_at - DELIVERED_CLEANUP_MARGIN_MS,
            delivered_at,
        )?;
        tx.execute("DELETE FROM sync_outbox WHERE operation_id = ?1 AND state = 'delivered'", [&op.operation_id])?;
        tx.commit()?;
        Ok(true)
    }

    /// Deletes seed topic `key` (and every seed merged into it) with its topic content everywhere: hard-deletes
    /// them here (see `purge_seed_tx`), records the keys as forgotten and queues a `topic_seed_purged` row (no
    /// name) for each. Safe to call when the seed is already gone here.
    pub fn forget_seed_topic(&self, key: &str) -> rusqlite::Result<TombstoneOutcome> {
        // Purged rows (kept word fingerprints, stamps) are overwritten, not left in free pages
        // (X6 fix round 6, N1d).
        let secure_before = self.set_secure_delete(true).ok();
        let r = self.forget_seed_topic_inner(key);
        if let Some(before) = secure_before {
            let _ = self.set_secure_delete(before);
        }
        r
    }

    fn forget_seed_topic_inner(&self, key: &str) -> rusqlite::Result<TombstoneOutcome> {
        let tx = self.conn.unchecked_transaction()?;
        let now = now_unix_ms();
        let mut outcome = TombstoneOutcome::Queued;
        for k in purge_seed_tx(&tx, key, now)? {
            let payload = TopicSeedPayload::purged(&k, now);
            outcome = self.enqueue_tombstone_tx(&tx, OUTBOX_ROW_TYPE_TOPIC_SEED_PURGED, k, &payload)?;
        }
        tx.commit()?;
        Ok(outcome)
    }

    /// A peer's purge of seed `key`: the same hard delete (merged seeds too), and the keys recorded so an older
    /// copy of a seed (or its steps, verdicts, role, snapshot) arriving later is dropped. Never queues anything.
    pub(crate) fn apply_seed_purged(&self, key: &str, at: i64) -> rusqlite::Result<()> {
        let secure_before = self.set_secure_delete(true).ok();
        let r = (|| {
            let tx = self.conn.unchecked_transaction()?;
            purge_seed_tx(&tx, key, at)?;
            owe_client_cleanup(&tx)?;
            tx.commit()
        })();
        if let Some(before) = secure_before {
            let _ = self.set_secure_delete(before);
        }
        r
    }

    /// Up to `limit` of this device's relay rows of forgotten items still to delete:
    /// `(target, not_before, not_after)`, `target` as in the `client_relay_cleanup` table (a `raw:` target waits
    /// for [`Self::seal_client_relay_cleanup`]).
    pub fn client_relay_cleanup(&self, limit: i64) -> rusqlite::Result<Vec<(String, i64, i64)>> {
        self.conn
            .prepare(
                "SELECT target, not_before_unix_ms, not_after_unix_ms FROM client_relay_cleanup ORDER BY target LIMIT ?1",
            )?
            .query_map([limit], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect()
    }

    /// Rewrites every `raw:<entity identity>` cleanup target as `tag:<seal(identity)>` (the sync engine passes
    /// its entity-tag function), so no raw calendar identity outlives the sync round after its forget. Run at
    /// the start of every round, whether or not tagged rows can be deleted yet.
    pub fn seal_client_relay_cleanup(&self, seal: impl Fn(&str) -> String) -> rusqlite::Result<usize> {
        let tx = self.conn.unchecked_transaction()?;
        let raw: Vec<(String, i64, i64)> = tx
            .prepare(
                "SELECT target, not_before_unix_ms, not_after_unix_ms FROM client_relay_cleanup \
                 WHERE substr(target, 1, 4) = 'raw:'",
            )?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (target, not_before, not_after) in &raw {
            let identity = &target["raw:".len()..];
            tx.execute(
                "INSERT INTO client_relay_cleanup (target, not_before_unix_ms, not_after_unix_ms) VALUES (?1, ?2, ?3) \
                 ON CONFLICT(target) DO UPDATE SET not_after_unix_ms = MAX(not_after_unix_ms, excluded.not_after_unix_ms)",
                params![format!("tag:{}", seal(identity)), not_before, not_after],
            )?;
            tx.execute("DELETE FROM client_relay_cleanup WHERE target = ?1", [target])?;
        }
        tx.commit()?;
        Ok(raw.len())
    }

    /// Marks these cleanup targets done.
    pub fn clear_client_relay_cleanup(&self, targets: &[String]) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        for t in targets {
            tx.execute("DELETE FROM client_relay_cleanup WHERE target = ?1", [t])?;
        }
        tx.commit()
    }

    /// How many devices on the sync relay have not updated to this release, for the receipt's "N devices still
    /// need to update": the devices with rows on the relay that had not announced `client_controls_capability`
    /// at the last outbox pass (a phone that only pulls writes no rows and is not counted). Informational: it
    /// holds nothing back. `None` until a pass has measured it (no sync yet), so the caller never reads "every
    /// device is current" from no data.
    pub fn pending_tombstone_devices(&self) -> rusqlite::Result<Option<usize>> {
        self.get_meta(PENDING_DEVICES_KEY)?
            .map(|v| v.parse().map_err(|_| crate::topics::seeds::invalid("unreadable pending device count")))
            .transpose()
    }

    /// How many devices on the relay hold back this device's seed rows (they have not announced the topics marker,
    /// so `topic_seed` rows wait unsent while any is there), at the last outbox pass. The exclusion setting travels
    /// in the seed row, so while this is more than 0 another device may not learn that a client is excluded (X6
    /// final review, Major 22). A removed device still counts: it releases nothing. `None` until a pass measured it.
    pub fn topic_rows_held_devices(&self) -> rusqlite::Result<Option<usize>> {
        self.get_meta(TOPIC_HELD_DEVICES_KEY)?
            .map(|v| v.parse().map_err(|_| crate::topics::seeds::invalid("unreadable topic-held device count")))
            .transpose()
    }

    /// Stores [`Self::topic_rows_held_devices`] (the sync engine, each outbox pass).
    pub fn set_topic_rows_held_devices(&self, n: usize) -> rusqlite::Result<()> {
        // Every outbox pass calls this, nearly always with the same count. A read takes no write lock; an
        // unchanged count then never waits behind another connection's write (Nov 18 2026: 25 s, which
        // timed the round out).
        let value = n.to_string();
        if self.get_meta(TOPIC_HELD_DEVICES_KEY)?.as_deref() == Some(value.as_str()) {
            return Ok(());
        }
        self.set_meta(TOPIC_HELD_DEVICES_KEY, &value)
    }

    /// The ids (sync device ids, which are public keys) of the devices [`Self::pending_tombstone_devices`]
    /// counts, for naming them. `None` until a pass stored them.
    pub fn pending_tombstone_device_ids(&self) -> rusqlite::Result<Option<Vec<String>>> {
        self.get_meta(PENDING_DEVICE_IDS_KEY)?
            .map(|v| serde_json::from_str(&v).map_err(|_| crate::topics::seeds::invalid("unreadable pending device ids")))
            .transpose()
    }

    /// Stores the devices still to update: their ids and their count (the sync engine, each outbox pass, and a
    /// phone's pull-only round).
    pub fn set_pending_tombstone_device_ids(&self, ids: &[String]) -> rusqlite::Result<()> {
        // Unchanged ids write nothing, so this never waits for the write lock (see `set_topic_rows_held_devices`).
        let json = serde_json::to_string(ids).map_err(|_| crate::topics::seeds::invalid("pending device ids"))?;
        if self.get_meta(PENDING_DEVICE_IDS_KEY)?.as_deref() == Some(json.as_str())
            && self.get_meta(PENDING_DEVICES_KEY)?.as_deref() == Some(ids.len().to_string().as_str())
        {
            return Ok(());
        }
        let tx = self.conn.unchecked_transaction()?;
        self.write_pending_device_ids(ids)?;
        tx.commit()
    }

    fn write_pending_device_ids(&self, ids: &[String]) -> rusqlite::Result<()> {
        let json = serde_json::to_string(ids).map_err(|_| crate::topics::seeds::invalid("pending device ids"))?;
        self.set_meta(PENDING_DEVICE_IDS_KEY, &json)?;
        self.set_meta(PENDING_DEVICES_KEY, &ids.len().to_string())
    }

    /// "Remove this device" from the devices still to update: `device_id` (one the last pass listed, never this
    /// device's own) is taken out of the stored list and count now, and queued for the next desktop sync round,
    /// which publishes the removal on the relay so every device stops counting it. It releases nothing: client
    /// deletes are never held for a device, and the topic gates still count it. `false`, with nothing changed,
    /// when the id is this device's or not in the list.
    pub fn queue_retired_device(&self, device_id: &str) -> rusqlite::Result<bool> {
        let id = device_id.trim();
        if id.is_empty() || self.own_device_id().as_deref() == Some(id) {
            return Ok(false);
        }
        let Some(mut pending) = self.pending_tombstone_device_ids()? else { return Ok(false) };
        if !pending.iter().any(|p| p == id) {
            return Ok(false);
        }
        pending.retain(|p| p != id);
        let mut queue = self.retired_device_queue()?;
        if !queue.iter().any(|q| q == id) {
            queue.push(id.to_string());
        }
        let queued = serde_json::to_string(&queue).map_err(|_| crate::topics::seeds::invalid("retire queue"))?;
        let tx = self.conn.unchecked_transaction()?;
        self.write_pending_device_ids(&pending)?;
        self.set_meta(RETIRE_QUEUE_KEY, &queued)?;
        tx.commit()?;
        Ok(true)
    }

    /// Devices removed here whose removal is not on the relay yet.
    pub fn retired_device_queue(&self) -> rusqlite::Result<Vec<String>> {
        Ok(match self.get_meta(RETIRE_QUEUE_KEY)? {
            Some(v) => serde_json::from_str(&v).map_err(|_| crate::topics::seeds::invalid("unreadable retire queue"))?,
            None => Vec::new(),
        })
    }

    /// Takes `done` off the queue (published, or no longer needed).
    pub fn clear_retired_devices(&self, done: &[String]) -> rusqlite::Result<()> {
        let mut queue = self.retired_device_queue()?;
        queue.retain(|q| !done.contains(q));
        let json = serde_json::to_string(&queue).map_err(|_| crate::topics::seeds::invalid("retire queue"))?;
        self.set_meta(RETIRE_QUEUE_KEY, &json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NewCalendarEventItem, NewCaptureItem, NewEmailItem};

    const NAME: &str = "Zorblax";

    fn store(device: &str) -> Store {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id(device);
        s
    }

    /// The outbox stores these counts every pass. An unchanged value writes nothing, so it never waits for
    /// another connection's write lock (Nov 18 2026: 25 s behind one, which timed the sync round out).
    #[test]
    fn unchanged_sync_counts_take_no_write_lock() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.sqlite");
        let s = Store::open(&path, dir.path()).unwrap();
        s.set_topic_rows_held_devices(2).unwrap();
        s.set_pending_tombstone_device_ids(&["dev-b".to_string()]).unwrap();

        let other = rusqlite::Connection::open(&path).unwrap();
        other.execute_batch("BEGIN IMMEDIATE;").unwrap();
        s.conn.busy_timeout(std::time::Duration::from_millis(50)).unwrap();
        s.set_topic_rows_held_devices(2).unwrap();
        s.set_pending_tombstone_device_ids(&["dev-b".to_string()]).unwrap();
        assert!(s.set_topic_rows_held_devices(3).is_err(), "a changed count still writes (and waits)");
        other.execute_batch("ROLLBACK;").unwrap();

        s.set_topic_rows_held_devices(3).unwrap();
        assert_eq!(s.topic_rows_held_devices().unwrap(), Some(3));
        assert_eq!(s.pending_tombstone_devices().unwrap(), Some(1));
    }

    fn capture(s: &Store, device: &str) -> i64 {
        s.insert_capture_item(&NewCaptureItem {
            device_id: device.into(),
            ts_unix_ms: 1_000,
            app_bundle_id: "com.apple.Safari".into(),
            window_title: format!("{NAME} proposal"),
            url: None,
            text: format!("Draft for {NAME}"),
            hash: 7,
            source: "ax".into(),
        })
        .unwrap()
        .unwrap();
        // The client item is the `items` row every capture has.
        s.conn
            .query_row(
                "SELECT i.id FROM items i JOIN capture_item c ON i.source_table = 'capture_item' AND i.source_id = c.id \
                 WHERE i.kind = 'capture' ORDER BY i.id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn calendar(s: &Store, ext: &str) -> i64 {
        s.upsert_calendar_event_items(&[NewCalendarEventItem {
            device_id: "dev-a".into(),
            source: "google_calendar".into(),
            external_id: ext.into(),
            ts_unix_ms: 2_000,
            title: format!("{NAME} kickoff"),
            actors: vec![],
        }])
        .unwrap();
        s.conn
            .query_row("SELECT id FROM items WHERE kind = 'calendar_event' AND external_id = ?1", [ext], |r| r.get(0))
            .unwrap()
    }

    fn email(s: &Store, ext: &str) -> i64 {
        s.upsert_email_items(&[NewEmailItem {
            device_id: "dev-a".into(),
            external_id: ext.into(),
            ts_unix_ms: 3_000,
            from: "pat@x.com".into(),
            to: "me@x.com".into(),
            subject: format!("{NAME} SOW"),
            body_text: format!("{NAME} wants the SOW"),
            gmail_thread_id: "t1".into(),
            account_email: "me@x.com".into(),
            gmail_labels: vec![],
            list_unsubscribe: false,
        }])
        .unwrap();
        s.conn
            .query_row("SELECT id FROM items WHERE kind = 'email' AND external_id = ?1", [ext], |r| r.get(0))
            .unwrap()
    }

    fn tombstones(s: &Store) -> Vec<ClientItemForgotten> {
        s.list_outbox_operations()
            .unwrap()
            .into_iter()
            .filter(|o| o.1 == OUTBOX_ROW_TYPE_CLIENT_ITEM_FORGOTTEN)
            .map(|o| serde_json::from_str(&o.3).unwrap())
            .collect()
    }

    #[test]
    fn each_kind_names_its_cross_device_identity() {
        let s = store("dev-a");
        let c = capture(&s, "dev-a");
        let seq: i64 = s.conn.query_row("SELECT local_seq FROM items WHERE id = ?1", [c], |r| r.get(0)).unwrap();
        let cal = calendar(&s, "ev-1");
        let em = email(&s, "msg-1");
        assert_eq!(s.enqueue_client_item_forgotten(ClientItemKind::Capture, &c.to_string()).unwrap(), TombstoneOutcome::Queued);
        assert_eq!(s.enqueue_client_item_forgotten(ClientItemKind::CalendarEvent, &cal.to_string()).unwrap(), TombstoneOutcome::Queued);
        assert_eq!(s.enqueue_client_item_forgotten(ClientItemKind::Email, &em.to_string()).unwrap(), TombstoneOutcome::Queued);
        let ids: Vec<(String, String)> = tombstones(&s).into_iter().map(|t| (t.kind, t.opaque_id)).collect();
        assert_eq!(
            ids,
            vec![
                ("capture".into(), format!("dev-a:{seq}")),
                ("calendar_event".into(), hashed("google_calendar:ev-1")),
                ("email".into(), "msg-1".into()),
            ]
        );
        assert!(s.is_client_item_forgotten("email", "msg-1").unwrap());
        // A re-run (the delete engine continuing after a failure) queues nothing new.
        assert_eq!(s.enqueue_client_item_forgotten(ClientItemKind::Email, &em.to_string()).unwrap(), TombstoneOutcome::Queued);
        assert_eq!(tombstones(&s).len(), 3);
    }

    #[test]
    fn never_synced_items_queue_nothing_and_a_missing_row_is_an_error() {
        let s = store("dev-a");
        let c = capture(&s, "dev-a");
        s.conn.execute("UPDATE items SET local_seq = NULL WHERE id = ?1", [c]).unwrap();
        assert_eq!(s.enqueue_client_item_forgotten(ClientItemKind::Capture, &c.to_string()).unwrap(), TombstoneOutcome::LocalOnly);
        s.conn
            .execute(
                "INSERT INTO conversation (title, created_at_unix_ms, updated_at_unix_ms) VALUES ('q', 1, 1)",
                [],
            )
            .unwrap();
        let conv = s.conn.last_insert_rowid();
        assert_eq!(s.enqueue_client_item_forgotten(ClientItemKind::Conversation, &conv.to_string()).unwrap(), TombstoneOutcome::LocalOnly);
        assert!(tombstones(&s).is_empty());
        assert!(matches!(
            s.enqueue_client_item_forgotten(ClientItemKind::Email, "999"),
            Err(rusqlite::Error::QueryReturnedNoRows)
        ));
        assert!(s.enqueue_client_item_forgotten(ClientItemKind::Email, "not-a-number").is_err());
    }

    #[test]
    fn client_deleting_a_conversation_drops_its_topic_link() {
        let s = store("dev-a");
        let c = s.insert_conversation("q", 1).unwrap();
        s.set_conversation_topic(c, Some(1), true).unwrap();
        s.delete_client_item(ClientItemKind::Conversation, &c.to_string()).unwrap();
        let n: i64 = s
            .conn
            .query_row("SELECT COUNT(*) FROM topic_conversation WHERE conversation_id = ?1", [c], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn applying_deletes_the_copy_and_remembers_it() {
        let s = store("dev-b");
        let em = email(&s, "msg-1");
        let cal = calendar(&s, "ev-1");
        let at = |kind: &str, id: &str| ClientItemForgotten { kind: kind.into(), opaque_id: id.into(), forgotten_at_unix_ms: 5 };
        assert_eq!(s.apply_client_item_forgotten(&at("email", "msg-1")).unwrap(), 1);
        assert_eq!(s.apply_client_item_forgotten(&at("calendar_event", &hashed("google_calendar:ev-1"))).unwrap(), 1);
        for id in [em, cal] {
            let n: i64 = s.conn.query_row("SELECT COUNT(*) FROM items WHERE id = ?1", [id], |r| r.get(0)).unwrap();
            assert_eq!(n, 0);
        }
        assert!(s.is_client_item_forgotten("calendar_event", &hashed("google_calendar:ev-1")).unwrap());
        assert!(tombstones(&s).is_empty(), "applying never queues a tombstone");
        assert_eq!(s.apply_client_item_forgotten(&at("some_future_kind", "x")).unwrap(), 0);
    }

    /// X6 final review, Major 3: a meeting step's key ends in a hash of its text (`{source}#h{fnv64}`); its
    /// tombstone and every device's forgotten record carry an opaque id of the whole key, never the key itself.
    /// A tombstone already published in the old (raw key) form still applies and is remembered opaque, and an
    /// old raw record here is rewritten.
    #[test]
    fn a_text_hashed_step_key_never_leaves_in_the_clear() {
        const KEY: &str = "meeting:42#h0123456789abcdef";
        let step = |s: &Store| {
            s.conn
                .execute(
                    "INSERT INTO topic_step (key, topic_key, source_key, source_kind, source_title, text, direction, \
                     asked_at_unix_ms, updated_at_unix_ms) VALUES (?1, 'tk', 'meeting:42', 'meeting', 't', 'x', 'mine', 1, 1)",
                    [KEY],
                )
                .unwrap();
        };
        let raw_anywhere = |s: &Store| {
            count(s, "SELECT COUNT(*) FROM forgotten_client_item WHERE instr(opaque_id, '0123456789abcdef') > 0")
                + count(s, "SELECT COUNT(*) FROM sync_outbox WHERE instr(payload_json || aggregate_id, '0123456789abcdef') > 0")
        };
        let a = store("dev-a");
        step(&a);
        assert_eq!(a.enqueue_client_item_forgotten(ClientItemKind::Step, KEY).unwrap(), TombstoneOutcome::Queued);
        let sent = tombstones(&a);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].opaque_id, hashed(KEY));
        assert_eq!(raw_anywhere(&a), 0, "the key's text hash stays here");
        assert_eq!(topic_key_opaque_id(KEY), hashed(KEY));

        // A peer applies it: the step goes, and a late copy is dropped by the opaque id.
        let b = store("dev-b");
        step(&b);
        assert_eq!(b.apply_client_item_forgotten(&sent[0]).unwrap(), 1);
        assert!(b.is_client_item_forgotten("step", &topic_key_opaque_id(KEY)).unwrap());
        assert_eq!(raw_anywhere(&b), 0);

        // An old-format tombstone (the raw key) still applies, and is remembered opaque.
        let c = store("dev-c");
        step(&c);
        let old = ClientItemForgotten { kind: "step".into(), opaque_id: KEY.into(), forgotten_at_unix_ms: 5 };
        assert_eq!(c.apply_client_item_forgotten(&old).unwrap(), 1);
        assert!(c.is_client_item_forgotten("step", &topic_key_opaque_id(KEY)).unwrap());
        assert_eq!(raw_anywhere(&c), 0);

        // A raw record from before is rewritten when the tables are opened; an index key is left as it is.
        let d = store("dev-d");
        d.conn
            .execute_batch(&format!(
                "INSERT INTO forgotten_client_item VALUES ('step', '{KEY}', 1); \
                 INSERT INTO forgotten_client_item VALUES ('step', 'email:m1#0', 1);"
            ))
            .unwrap();
        // And tombstones queued in the old form: one delivered, one leased at a crash, one not sent yet.
        const KEY2: &str = "meeting:43#h0123456789abcdef";
        const KEY3: &str = "meeting:44#h0123456789abcdef";
        for (k, state) in [(KEY, "delivered"), (KEY2, "leased"), (KEY3, "pending")] {
            let tx = d.conn.unchecked_transaction().unwrap();
            Store::enqueue_opaque_tombstone_tx(&d, &tx, ClientItemKind::Step, k).unwrap();
            tx.commit().unwrap();
            d.conn.execute("UPDATE sync_outbox SET state = ?1 WHERE aggregate_id = ?2", [state, &format!("step:{k}")]).unwrap();
        }
        assert!(raw_anywhere(&d) > 0);
        ensure_client_forget_tables(&d.conn).unwrap();
        let ids: Vec<String> = tombstones(&d).into_iter().map(|t| t.opaque_id).collect();
        assert_eq!(ids, vec![hashed(KEY), hashed(KEY2), hashed(KEY3)]);
        assert_eq!(raw_anywhere(&d), 0);
        assert!(d.is_client_item_forgotten("step", &topic_key_opaque_id(KEY)).unwrap());
        assert!(d.is_client_item_forgotten("step", "email:m1#0").unwrap());
    }

    /// X6 final review, Major 4: a peer's applied delete leaves the term in the full-text index pages until the
    /// owed clean-up merges them, once; nothing removed owes nothing.
    #[test]
    fn an_applied_delete_owes_one_index_clean_up() {
        let s = store("dev-b");
        email(&s, "msg-1");
        let at = |id: &str| ClientItemForgotten { kind: "email".into(), opaque_id: id.into(), forgotten_at_unix_ms: 5 };
        assert_eq!(s.apply_client_item_forgotten(&at("nothing-here")).unwrap(), 0);
        assert!(!s.run_owed_client_cleanup().unwrap(), "nothing removed, nothing owed");
        assert_eq!(s.apply_client_item_forgotten(&at("msg-1")).unwrap(), 1);
        let term = NAME.to_lowercase();
        assert!(s.fts_pages_naming(&term).unwrap() > 0, "a plain delete leaves the term in its segment");
        assert!(s.run_owed_client_cleanup().unwrap());
        assert_eq!(s.fts_pages_naming(&term).unwrap(), 0);
        assert!(!s.run_owed_client_cleanup().unwrap(), "run once");
    }

    /// A thread, an episode, a step and a step read, as both devices hold them.
    fn derived_rows(s: &Store) {
        s.conn
            .execute_batch(&format!(
                "INSERT INTO threads (source, external_thread_id, account_email, subject, message_count, \
                   last_activity_ts_unix_ms, last_message_from, status, created_at_unix_ms, updated_at_unix_ms) \
                 VALUES ('gmail', 't1', 'me@x.com', '{NAME} SOW', 1, 1, 'pat@x.com', 'open', 1, 1);
                 INSERT INTO episodes (device_id, app_bundle_id, window_title, start_ts_unix_ms, end_ts_unix_ms, \
                   start_item_id, end_item_id, item_count, summary, created_at_unix_ms, updated_at_unix_ms) \
                 VALUES ('dev-a', 'com.apple.Safari', '{NAME}', 500, 900, 1, 2, 2, 'Read the {NAME} draft', 1, 1);
                 INSERT INTO topic_step (key, topic_key, source_key, source_kind, source_title, text, direction, \
                   asked_at_unix_ms, updated_at_unix_ms) \
                 VALUES ('email:m1#0', 'tk', 'email:m1', 'email', '{NAME} SOW', 'Send {NAME} the SOW', 'mine', 1, 1);
                 INSERT INTO topic_extraction (source_key, topic_key, dates, model, extracted_at_unix_ms, updated_at_unix_ms) \
                 VALUES ('email:m1', 'tk', '[]', 'm', 1, 1);"
            ))
            .unwrap();
    }

    fn count(s: &Store, sql: &str) -> i64 {
        s.conn.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn thread_episode_step_and_extraction_reach_a_peer() {
        let (a, b) = (store("dev-a"), store("dev-b"));
        derived_rows(&a);
        derived_rows(&b);
        let thread: i64 = a.conn.query_row("SELECT id FROM threads", [], |r| r.get(0)).unwrap();
        let episode: i64 = a.conn.query_row("SELECT id FROM episodes", [], |r| r.get(0)).unwrap();
        for (kind, id) in [
            (ClientItemKind::Thread, thread.to_string()),
            (ClientItemKind::Episode, episode.to_string()),
            (ClientItemKind::Step, "email:m1#0".to_string()),
            (ClientItemKind::Extraction, "email:m1".to_string()),
        ] {
            assert_eq!(a.enqueue_client_item_forgotten(kind, &id).unwrap(), TombstoneOutcome::Queued, "{kind:?}");
        }
        let sent = tombstones(&a);
        assert_eq!(sent.len(), 4);
        for t in &sent {
            assert!(!t.opaque_id.contains(NAME));
            assert_eq!(b.apply_client_item_forgotten(t).unwrap(), 1, "{}", t.kind);
        }
        for table in ["threads", "episodes", "topic_step", "topic_extraction"] {
            assert_eq!(count(&b, &format!("SELECT COUNT(*) FROM {table}")), 0, "{table} on B");
        }
    }

    #[test]
    fn a_purged_seed_never_comes_back_from_an_older_or_newer_record() {
        let s = store("dev-b");
        let named = |ts: i64| TopicSeedPayload {
            name: NAME.into(),
            aliases: vec![NAME.into()],
            deleted: false,
            purged: false,
            ..TopicSeedPayload::purged("k1", ts)
        };
        s.apply_synced_topic_seed(&named(10)).unwrap();
        assert_eq!(s.list_seed_topics().unwrap().len(), 1);
        s.apply_synced_topic_seed(&TopicSeedPayload::purged("k1", 5)).unwrap();
        // An older device's copy, then one stamped later than the purge: neither brings the name back.
        s.apply_synced_topic_seed(&named(10)).unwrap();
        s.apply_synced_topic_seed(&named(i64::MAX / 2)).unwrap();
        assert!(s.list_seed_topics().unwrap().is_empty());
        let named_topics = s
            .conn
            .query_row("SELECT COUNT(*) FROM topic WHERE name LIKE ?1", [format!("%{NAME}%")], |r| r.get::<_, i64>(0))
            .unwrap();
        assert_eq!(named_topics, 0);
        assert_eq!(count(&s, "SELECT COUNT(*) FROM topic_seed"), 0);
    }

    /// X6 final review, B1: a peer applying the purge deletes the client's retention stubs (title, text,
    /// mentions); a stub another client shares stays unless it names the client.
    #[test]
    fn a_peer_purge_deletes_the_clients_retention_stubs() {
        let s = store("dev-b");
        let a = s.create_seed_topic(NAME, &[], None).unwrap();
        let b = s.create_seed_topic("Globex", &[], None).unwrap();
        let stub = |title: &str, topics: &[i64]| -> i64 {
            s.conn
                .execute(
                    "INSERT INTO topic_source (kind, ref_id, ts_unix_ms, title, stub, indexed_at_unix_ms) VALUES ('meeting', NULL, 1, ?1, 1, 1)",
                    [title],
                )
                .unwrap();
            let id = s.conn.last_insert_rowid();
            s.conn.execute("INSERT INTO topic_source_text (source_id, text) VALUES (?1, 'notes')", [id]).unwrap();
            s.conn.execute("INSERT INTO entity_mention (entity_id, source_id) VALUES (1, ?1)", [id]).unwrap();
            for t in topics {
                s.conn.execute("INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1, ?2, 'seed')", [*t, id]).unwrap();
            }
            id
        };
        // Topics that are no live client's: an auto topic, and a soft-deleted seed's. And an unseeded topic merged
        // into B's seed topic, which is B's.
        let topic = |merged_into: Option<i64>| -> i64 {
            s.conn
                .execute(
                    "INSERT INTO topic (name, merged_into, created_at_unix_ms, updated_at_unix_ms) VALUES ('Rollout', ?1, 1, 1)",
                    [merged_into],
                )
                .unwrap();
            s.conn.last_insert_rowid()
        };
        let auto = topic(None);
        let into_b = topic(Some(b.topic_id));
        let gone = s.create_seed_topic("Initech", &[], None).unwrap();
        s.conn.execute("UPDATE topic_seed SET deleted = 1 WHERE key = ?1", [&gone.key]).unwrap();
        let own = stub("Planning call", &[a.topic_id]);
        let shared_named = stub(&format!("{NAME} and Globex"), &[a.topic_id, b.topic_id]);
        let shared_plain = stub("Partners call", &[a.topic_id, b.topic_id]);
        // (a) Filed under A (by Jev: no name in it) and also in an auto topic and a deleted seed's: A's, it goes.
        let with_auto = stub("Rollout call", &[a.topic_id, auto, gone.topic_id]);
        // (b) Also held by B through B's merged topic, not naming A: B's too, it stays.
        let via_merge = stub("Weekly sync", &[a.topic_id, into_b]);
        s.apply_synced_topic_seed(&TopicSeedPayload::purged(&a.key, 5)).unwrap();
        assert_eq!(count(&s, &format!("SELECT COUNT(*) FROM topic_member WHERE source_id = {via_merge} AND topic_id = {into_b}")), 1);
        for id in [own, shared_named, with_auto] {
            assert_eq!(count(&s, &format!("SELECT COUNT(*) FROM topic_source WHERE id = {id}")), 0);
            assert_eq!(count(&s, &format!("SELECT COUNT(*) FROM topic_source_text WHERE source_id = {id}")), 0);
            assert_eq!(count(&s, &format!("SELECT COUNT(*) FROM entity_mention WHERE source_id = {id}")), 0);
        }
        assert_eq!(count(&s, &format!("SELECT COUNT(*) FROM topic_member WHERE source_id = {shared_plain}")), 1);
    }

    /// X6 final review, Major 1: a peer's own model error notes can quote its summary of the client's
    /// material; the purge clears them.
    #[test]
    fn a_peer_purge_clears_model_error_notes() {
        let s = store("dev-b");
        let a = s.create_seed_topic(NAME, &[], None).unwrap();
        s.record_summarize_error("thread_summarize", &format!("no JSON object: \"{NAME} wants changes\""), 1);
        s.apply_synced_topic_seed(&TopicSeedPayload::purged(&a.key, 5)).unwrap();
        assert_eq!(count(&s, "SELECT COUNT(*) FROM kv_meta WHERE key LIKE 'last_error:%'"), 0);
    }

    #[test]
    fn without_a_device_id_the_tombstone_is_reported_not_lost() {
        let s = Store::open_in_memory().unwrap();
        let em = email(&s, "msg-1");
        assert_eq!(
            s.enqueue_client_item_forgotten(ClientItemKind::Email, &em.to_string()).unwrap(),
            TombstoneOutcome::NoDeviceId
        );
        assert!(s.is_client_item_forgotten("email", "msg-1").unwrap(), "still forgotten here");
        let seed = s.create_seed_topic(NAME, &[], None).unwrap();
        assert_eq!(s.forget_seed_topic(&seed.key).unwrap(), TombstoneOutcome::NoDeviceId);
    }

    #[test]
    fn a_calendar_uid_with_a_domain_travels_hashed_and_takes_its_prep() {
        let (a, b) = (store("dev-a"), store("dev-b"));
        // As in production: the item is one occurrence, `{event id}#{start secs}`; the brief is keyed
        // `{event id}@{start ms}`.
        let uid = format!("kickoff-1@{}.com", NAME.to_lowercase());
        let occurrence = format!("{uid}#2");
        let brief = |key: &str, title: &str| crate::prep::MeetingPrepRow {
            event_id: key.into(),
            title: title.into(),
            start_unix_ms: 2000,
            generated_at_unix_ms: 1,
            brief_json: format!(r#"{{"about":"{NAME}"}}"#),
            provider: "p".into(),
            model: "m".into(),
        };
        let cal = calendar(&a, &occurrence);
        calendar(&b, &occurrence);
        // A wrote the brief (queued for sync, not sent yet); B pulled it and copied it under its own id.
        a.save_meeting_prep(&brief(&format!("{uid}@2000"), "Kickoff")).unwrap();
        b.apply_synced_meeting_prep(&brief(&format!("{uid}@2000"), "Kickoff")).unwrap();
        b.apply_synced_meeting_prep(&brief("b-local-id@2000", " kickoff ")).unwrap();
        assert_eq!(count(&a, "SELECT COUNT(*) FROM sync_outbox WHERE row_type = 'meeting_prep' AND state = 'pending'"), 1);

        a.enqueue_client_item_forgotten(ClientItemKind::CalendarEvent, &cal.to_string()).unwrap();
        let t = tombstones(&a).remove(0);
        assert!(!t.opaque_id.to_lowercase().contains(&NAME.to_lowercase()));
        assert_eq!(t.opaque_id.len(), 2 + 64);
        assert_eq!(count(&a, "SELECT COUNT(*) FROM meeting_prep"), 0, "the deleting device's brief goes too");
        assert_eq!(count(&a, "SELECT COUNT(*) FROM sync_outbox WHERE row_type = 'meeting_prep'"), 0, "never sent");
        assert!(a.is_meeting_prep_forgotten(&format!("{uid}@2000")).unwrap());

        assert_eq!(b.apply_client_item_forgotten(&t).unwrap(), 1);
        assert_eq!(count(&b, "SELECT COUNT(*) FROM items WHERE kind = 'calendar_event'"), 0);
        assert_eq!(count(&b, "SELECT COUNT(*) FROM meeting_prep"), 0, "the brief and B's copy of it");
        assert!(b.is_meeting_prep_forgotten(&format!("{uid}@3000")).unwrap(), "any occurrence of the series");
        assert!(b.is_meeting_prep_forgotten(&uid).unwrap(), "the bare id of an older brief");
        assert!(!b.is_meeting_prep_forgotten("other@2000").unwrap());

        // No raw calendar id is kept: B did not push the event, so B records nothing; A records it raw only
        // until its next sync round seals it.
        assert!(b.client_relay_cleanup(10).unwrap().is_empty());
        let prep_op = ClaimedOutboxOperation {
            operation_id: "o".into(),
            device_id: "dev-b".into(),
            local_seq: 1,
            row_type: OUTBOX_ROW_TYPE_MEETING_PREP.into(),
            aggregate_type: OUTBOX_ROW_TYPE_MEETING_PREP.into(),
            aggregate_id: format!("{uid}@9000"),
            operation_kind: OutboxOperationKind::Upsert,
            payload_json: "{}".into(),
            connector: OUTBOX_CONNECTOR.into(),
            attempts: 1,
        };
        assert!(b.outbox_op_forgotten(&prep_op).unwrap(), "a brief leased before the delete is never sent");
        a.seal_client_relay_cleanup(|identity| hashed(identity)).unwrap();
        let mut stmt = a.conn.prepare("SELECT target FROM client_relay_cleanup").unwrap();
        for target in stmt.query_map([], |r| r.get::<_, String>(0)).unwrap() {
            let target = target.unwrap();
            assert!(!target.to_lowercase().contains(&NAME.to_lowercase()), "{target}");
        }
    }

    #[test]
    fn a_capture_takes_the_episode_built_over_it() {
        let (a, b) = (store("dev-a"), store("dev-b"));
        let c = capture(&a, "dev-a");
        let (seq, ts): (i64, i64) =
            a.conn.query_row("SELECT local_seq, ts_unix_ms FROM items WHERE id = ?1", [c], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        // B holds A's capture (pulled) and its own episode over it.
        b.conn
            .execute(
                "INSERT INTO items (device_id, local_seq, kind, ts_unix_ms, app_bundle_id, body_text) \
                 VALUES ('dev-a', ?1, 'capture', ?2, 'com.apple.Safari', 'x')",
                params![seq, ts],
            )
            .unwrap();
        b.conn
            .execute(
                "INSERT INTO episodes (device_id, app_bundle_id, window_title, start_ts_unix_ms, end_ts_unix_ms, \
                   start_item_id, end_item_id, item_count, summary, created_at_unix_ms, updated_at_unix_ms) \
                 VALUES ('dev-a', 'com.apple.Safari', 'w', ?1, ?2, 1, 1, 1, 's', 1, 1)",
                params![ts - 10, ts + 10],
            )
            .unwrap();
        a.enqueue_client_item_forgotten(ClientItemKind::Capture, &c.to_string()).unwrap();
        b.apply_client_item_forgotten(&tombstones(&a)[0]).unwrap();
        assert_eq!(count(&b, "SELECT COUNT(*) FROM episodes"), 0);
        assert_eq!(count(&b, "SELECT COUNT(*) FROM items WHERE kind = 'capture'"), 0);
        let targets: Vec<(String, i64)> = a.client_relay_cleanup(10).unwrap().into_iter().map(|t| (t.0, t.1)).collect();
        assert_eq!(
            targets,
            vec![
                (format!("seq:capture_item:{seq}"), ts - CAPTURE_CLEANUP_MARGIN_MS),
                (format!("seq:items:{seq}"), ts - CAPTURE_CLEANUP_MARGIN_MS),
            ]
        );
        assert!(b.client_relay_cleanup(10).unwrap().is_empty(), "B pushed none of A's capture");
    }

    #[test]
    fn purging_a_seed_takes_the_seeds_merged_into_it() {
        let s = store("dev-b");
        let named = |key: &str, merged: Option<&str>, ts: i64| TopicSeedPayload {
            name: format!("{NAME} {key}"),
            aliases: vec![],
            deleted: false,
            purged: false,
            merged_into_key: merged.map(str::to_string),
            ..TopicSeedPayload::purged(key, ts)
        };
        s.apply_synced_topic_seed(&named("k1", None, 10)).unwrap();
        s.apply_synced_topic_seed(&named("k2", Some("k1"), 10)).unwrap();
        s.apply_synced_topic_seed(&named("k3", Some("k2"), 10)).unwrap();
        s.apply_synced_topic_seed(&TopicSeedPayload::purged("k1", 20)).unwrap();
        assert_eq!(count(&s, "SELECT COUNT(*) FROM topic_seed"), 0);
        for k in ["k1", "k2", "k3"] {
            assert!(s.is_client_item_forgotten(FORGOTTEN_SEED_KIND, k).unwrap(), "{k}");
        }
        // A seed merged into a purged one, arriving later (k4 into k3), is purged on arrival.
        s.apply_synced_topic_seed(&named("k4", Some("k3"), 30)).unwrap();
        assert_eq!(count(&s, "SELECT COUNT(*) FROM topic_seed"), 0);
        assert!(s.is_client_item_forgotten(FORGOTTEN_SEED_KIND, "k4").unwrap());
        let named_topics = s
            .conn
            .query_row("SELECT COUNT(*) FROM topic WHERE name LIKE ?1", [format!("%{NAME}%")], |r| r.get::<_, i64>(0))
            .unwrap();
        assert_eq!(named_topics, 0);
    }

    #[test]
    fn an_op_about_a_forgotten_item_is_flagged_at_send_time() {
        let s = store("dev-a");
        let seed = s.create_seed_topic(NAME, &[], None).unwrap();
        let op = |row_type: &str, aggregate_id: &str, payload: &str| ClaimedOutboxOperation {
            operation_id: "o".into(),
            device_id: "dev-a".into(),
            local_seq: 1,
            row_type: row_type.into(),
            aggregate_type: row_type.into(),
            aggregate_id: aggregate_id.into(),
            operation_kind: OutboxOperationKind::Upsert,
            payload_json: payload.into(),
            connector: OUTBOX_CONNECTOR.into(),
            attempts: 1,
        };
        let seed_op = op(OUTBOX_ROW_TYPE_TOPIC_SEED, &seed.key, "{}");
        let step_op = op(OUTBOX_ROW_TYPE_TOPIC_STEP, "calendar:g:x#0", &format!(r#"{{"topic_key":"{}"}}"#, seed.key));
        assert!(!s.outbox_op_forgotten(&seed_op).unwrap());
        assert!(!s.outbox_op_forgotten(&step_op).unwrap());
        s.forget_seed_topic(&seed.key).unwrap();
        assert!(s.outbox_op_forgotten(&seed_op).unwrap());
        assert!(s.outbox_op_forgotten(&step_op).unwrap());
        let other = op(OUTBOX_ROW_TYPE_TOPIC_STEP, "calendar:g:y#0", r#"{"topic_key":"other"}"#);
        assert!(!s.outbox_op_forgotten(&other).unwrap());
        s.apply_client_item_forgotten(&ClientItemForgotten {
            kind: "step".into(),
            opaque_id: hashed("calendar:g:y#0"),
            forgotten_at_unix_ms: 1,
        })
        .unwrap();
        assert!(s.outbox_op_forgotten(&other).unwrap(), "a hashed step key matches");
    }

    #[test]
    fn pending_devices_is_unknown_until_measured() {
        let s = store("dev-a");
        assert_eq!(s.pending_tombstone_devices().unwrap(), None);
        assert_eq!(s.pending_tombstone_device_ids().unwrap(), None);
        s.set_pending_tombstone_device_ids(&["old-1".into(), "old-2".into()]).unwrap();
        assert_eq!(s.pending_tombstone_devices().unwrap(), Some(2), "the count and the ids are written together");
        assert_eq!(s.pending_tombstone_device_ids().unwrap(), Some(vec!["old-1".to_string(), "old-2".to_string()]));
    }

    /// "Remove this device" never takes this device itself, even if a stale list names it.
    #[test]
    fn a_device_cannot_remove_itself_from_the_count() {
        let s = store("dev-a");
        s.set_pending_tombstone_device_ids(&["dev-a".into(), "old".into()]).unwrap();
        assert!(!s.queue_retired_device("dev-a").unwrap());
        assert!(!s.queue_retired_device("  dev-a  ").unwrap(), "trimmed before the check");
        assert!(!s.queue_retired_device("").unwrap());
        assert_eq!(s.pending_tombstone_devices().unwrap(), Some(2), "nothing changed");
        assert!(s.retired_device_queue().unwrap().is_empty());
        assert!(s.queue_retired_device("old").unwrap());
        assert_eq!(s.pending_tombstone_device_ids().unwrap(), Some(vec!["dev-a".to_string()]));
    }

    #[test]
    fn forgetting_a_seed_hard_deletes_it_and_queues_a_nameless_purge() {
        let s = store("dev-a");
        let seed = s.create_seed_topic(NAME, &[format!("{NAME} Ltd")], None).unwrap();
        assert_eq!(s.forget_seed_topic(&seed.key).unwrap(), TombstoneOutcome::Queued);
        let n: i64 = s
            .conn
            .query_row("SELECT COUNT(*) FROM topic WHERE name LIKE ?1", [format!("%{NAME}%")], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
        let n: i64 = s.conn.query_row("SELECT COUNT(*) FROM topic_seed", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0);
        for (_, row_type, state, payload) in s.list_outbox_operations().unwrap() {
            assert!(!payload.contains(NAME), "{row_type} ({state}) carries the name");
        }
        let purge: Vec<TopicSeedPayload> = s
            .list_outbox_operations()
            .unwrap()
            .into_iter()
            .filter(|o| o.1 == OUTBOX_ROW_TYPE_TOPIC_SEED_PURGED)
            .map(|o| serde_json::from_str(&o.3).unwrap())
            .collect();
        assert_eq!(purge.len(), 1);
        assert!(purge[0].purged && purge[0].name.is_empty() && purge[0].aliases.is_empty());
        assert!(s.is_client_item_forgotten(FORGOTTEN_SEED_KIND, &seed.key).unwrap());
    }

    /// Rows a local-only expiry must never write: tombstones, relay clean-up, forgotten records.
    fn outbound(s: &Store) -> (i64, i64, i64, i64) {
        (
            count(s, "SELECT COUNT(*) FROM sync_outbox WHERE operation_kind = 'tombstone'"),
            count(s, "SELECT COUNT(*) FROM client_relay_cleanup"),
            count(s, "SELECT COUNT(*) FROM forgotten_client_item"),
            count(s, "SELECT COUNT(*) FROM forgotten_meeting"),
        )
    }

    fn deliver_all(s: &Store) {
        s.conn
            .execute(
                "UPDATE sync_outbox SET state = 'delivered', delivered_at_unix_ms = 10, \
                 remote_receipt = 'sync_row:' || local_seq",
                [],
            )
            .unwrap();
    }

    /// X6 final review, Major 21: with sync on, an expiry on a store that does not know its own device id
    /// (or knows only the legacy placeholder) cannot tell an unsent own row from a peer's: it deletes nothing
    /// and reports the item as waiting to sync. With sync off it goes as before.
    #[test]
    fn an_expiry_with_sync_on_and_no_device_id_deletes_nothing() {
        for own in [None, Some(crate::LEGACY_SPIKE_DEVICE)] {
            let s = Store::open_in_memory().unwrap();
            if let Some(id) = own {
                s.set_own_device_id(id);
            }
            let cal = calendar(&s, "kickoff#1");
            let cap = capture(&s, "dev-a");
            let raw_cap: i64 = s.conn.query_row("SELECT id FROM capture_item", [], |r| r.get(0)).unwrap();
            let m = s
                .insert_meeting(&crate::NewMeeting {
                    device_id: "dev-a".into(),
                    start_unix_ms: 1,
                    end_unix_ms: Some(2),
                    title: "Call".into(),
                    attendees: vec![],
                    trigger: crate::MeetingTrigger::Manual,
                    audio_path: None,
                    status: crate::MeetingStatus::Completed,
                })
                .unwrap();
            for (kind, id) in [(ClientItemKind::CalendarEvent, cal), (ClientItemKind::Capture, cap)] {
                let e = s.expire_client_item(kind, &id.to_string(), true).unwrap_err();
                assert!(is_unsent_ops(&e), "{kind:?} {own:?}: {e}");
            }
            assert!(is_unsent_ops(&s.expire_meeting_here(m, true).unwrap_err()));
            assert_eq!(count(&s, "SELECT COUNT(*) FROM items"), 2, "{own:?}");
            assert_eq!(count(&s, "SELECT COUNT(*) FROM meeting"), 1);
            // A legacy capture with no `items` twin.
            s.conn.execute("DELETE FROM items WHERE id = ?1", [cap]).unwrap();
            assert!(is_unsent_ops(&s.delete_client_capture_in(raw_cap, Reach::ThisDevice { sync_on: true }).unwrap_err()));
            assert_eq!(count(&s, "SELECT COUNT(*) FROM capture_item"), 1);
            // Sync off: nothing can be lost by deleting here.
            assert!(s.expire_client_item(ClientItemKind::CalendarEvent, &cal.to_string(), false).unwrap().is_some());
            assert!(s.expire_meeting_here(m, false).unwrap().is_some());
        }
    }

    #[test]
    fn an_expiry_sends_nothing_records_nothing_and_takes_only_its_occurrences_brief() {
        let s = store("dev-a");
        let uid = format!("weekly@{}.com", NAME.to_lowercase());
        let cal = calendar(&s, &format!("{uid}#2"));
        let brief = |start: i64| crate::prep::MeetingPrepRow {
            event_id: crate::prep::prep_key(&uid, start),
            title: "Weekly".into(),
            start_unix_ms: start,
            generated_at_unix_ms: 1,
            brief_json: format!(r#"{{"about":"{NAME}"}}"#),
            provider: "p".into(),
            model: "m".into(),
        };
        s.save_meeting_prep(&brief(2000)).unwrap();
        s.save_meeting_prep(&brief(9000)).unwrap();
        deliver_all(&s);
        s.set_sync_push_cursor(i64::MAX).unwrap();
        let before = outbound(&s);

        let out = s.delete_client_item_in(ClientItemKind::CalendarEvent, &cal.to_string(), Reach::ThisDevice { sync_on: true });
        assert_eq!(out.unwrap(), Some(TombstoneOutcome::LocalOnly));
        assert_eq!(outbound(&s), before, "no tombstone, relay clean-up or forgotten record");
        assert_eq!(count(&s, "SELECT COUNT(*) FROM items WHERE kind = 'calendar_event'"), 0);
        let left: Vec<String> = s
            .conn
            .prepare("SELECT event_id FROM meeting_prep")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(left, vec![crate::prep::prep_key(&uid, 9000)], "the later occurrence keeps its brief");
        assert_eq!(
            count(&s, "SELECT COUNT(*) FROM sync_outbox WHERE row_type = 'meeting_prep'"),
            1,
            "the expired brief's delivered op goes here; the later one's stays"
        );
        assert!(!s.is_meeting_prep_forgotten(&crate::prep::prep_key(&uid, 9000)).unwrap());
        // A calendar poll does not bring the expired occurrence back here.
        calendar_skipped(&s, &format!("{uid}#2"));
        // A re-run finds it gone.
        assert_eq!(
            s.delete_client_item_in(ClientItemKind::CalendarEvent, &cal.to_string(), Reach::ThisDevice { sync_on: true }).unwrap(),
            None
        );
    }

    fn calendar_skipped(s: &Store, ext: &str) {
        s.upsert_calendar_event_items(&[NewCalendarEventItem {
            device_id: "dev-a".into(),
            source: "google_calendar".into(),
            external_id: ext.into(),
            ts_unix_ms: 2_000,
            title: "again".into(),
            actors: vec![],
        }])
        .unwrap();
        assert_eq!(count(s, "SELECT COUNT(*) FROM items WHERE kind = 'calendar_event'"), 0);
    }

    #[test]
    fn an_expiry_waits_for_an_item_not_sent_yet_unless_sync_is_off() {
        let s = store("dev-a");
        let c = capture(&s, "dev-a");
        let this = |sync_on| Reach::ThisDevice { sync_on };
        // The push cursor has not passed the capture: the other devices have not got it.
        let err = s.delete_client_item_in(ClientItemKind::Capture, &c.to_string(), this(true)).unwrap_err();
        assert!(is_unsent_ops(&err));
        assert_eq!(count(&s, "SELECT COUNT(*) FROM items WHERE kind = 'capture'"), 1);
        s.set_sync_push_cursor(i64::MAX).unwrap();
        assert!(s.delete_client_item_in(ClientItemKind::Capture, &c.to_string(), this(true)).unwrap().is_some());
        assert_eq!(count(&s, "SELECT COUNT(*) FROM capture_item"), 0);

        // A step whose op is still queued: refused with sync on, nothing changed; dropped with sync off.
        derived_rows(&s);
        s.conn
            .execute(
                "INSERT INTO sync_outbox (operation_id, device_id, local_seq, row_type, aggregate_type, aggregate_id, \
                 operation_kind, payload_json, connector, state, next_attempt_at_unix_ms, created_at_unix_ms, updated_at_unix_ms) \
                 VALUES ('op1', 'dev-a', 99, 'topic_step', 'topic_step', 'email:m1#0', 'upsert', '{}', 'c', 'pending', 0, 0, 0)",
                [],
            )
            .unwrap();
        let err = s.delete_client_item_in(ClientItemKind::Step, "email:m1#0", this(true)).unwrap_err();
        assert!(is_unsent_ops(&err));
        assert_eq!(count(&s, "SELECT COUNT(*) FROM topic_step"), 1);
        assert_eq!(count(&s, "SELECT COUNT(*) FROM sync_outbox"), 1);
        assert!(s.delete_client_item_in(ClientItemKind::Step, "email:m1#0", this(false)).unwrap().is_some());
        assert_eq!(count(&s, "SELECT COUNT(*) FROM topic_step"), 0);
        assert_eq!(count(&s, "SELECT COUNT(*) FROM sync_outbox"), 0);
        assert_eq!(outbound(&s), (0, 0, 0, 0));
    }

    #[test]
    fn an_expired_email_keeps_its_thread_while_newer_mail_is_in_it() {
        let s = store("dev-a");
        let old = email(&s, "msg-1");
        let new = email(&s, "msg-2");
        derived_rows(&s);
        let this = Reach::ThisDevice { sync_on: false };
        s.delete_client_item_in(ClientItemKind::Email, &old.to_string(), this).unwrap();
        assert_eq!(count(&s, "SELECT COUNT(*) FROM threads"), 1, "msg-2 is still in the thread");
        s.delete_client_item_in(ClientItemKind::Email, &new.to_string(), this).unwrap();
        assert_eq!(count(&s, "SELECT COUNT(*) FROM threads"), 0);
        assert_eq!(outbound(&s), (0, 0, 0, 0));
        // A mail resync does not bring them back here.
        s.upsert_email_items(&[NewEmailItem {
            device_id: "dev-a".into(),
            external_id: "msg-1".into(),
            ts_unix_ms: 3_000,
            from: "pat@x.com".into(),
            to: "me@x.com".into(),
            subject: "again".into(),
            body_text: "again".into(),
            gmail_thread_id: "t1".into(),
            account_email: "me@x.com".into(),
            gmail_labels: vec![],
            list_unsubscribe: false,
        }])
        .unwrap();
        assert_eq!(count(&s, "SELECT COUNT(*) FROM items WHERE kind = 'email'"), 0);
    }
}
