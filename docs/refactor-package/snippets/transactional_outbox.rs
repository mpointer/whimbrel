//! Draft target: core/memory/src/sync_outbox.rs
//!
//! Integration intent
//! ------------------
//! Kestrel currently derives a push from the mutable domain tables by comparing a single
//! `sync_push_cursor` with `local_seq`. This module makes delivery a first-class durable
//! concern. The caller must perform the domain mutation and `enqueue_outbox_tx` in the
//! SAME SQLite transaction. A crash therefore yields either neither change, or a local
//! change with a replay-safe outgoing operation.
//!
//! This is an initial refactoring slice, not a drop-in replacement. It assumes the
//! existing `Store` single-owner rule remains in force while the wider CoreEngine actor
//! is introduced. `unchecked_transaction()` is safe only under that rule.
//!
//! Cargo additions for core/memory/Cargo.toml:
//!   uuid = { version = "1", features = ["v4"] }
//!
//! Migration placement:
//!   call `ensure_sync_outbox_schema(&store.conn)` from Store::init() after the current
//!   sync cursor/map schema. Keep existing `local_seq`, `sync_row_map`,
//!   `sync_meeting_map`, and `sync_pending_dependency` during the relay migration.

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const OUTBOX_PENDING: &str = "pending";
const OUTBOX_LEASED: &str = "leased";
const OUTBOX_DELIVERED: &str = "delivered";
const OUTBOX_BLOCKED: &str = "blocked";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboxOperationKind {
    Upsert,
    Tombstone,
}

impl OutboxOperationKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Upsert => "upsert",
            Self::Tombstone => "tombstone",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewOutboxOperation {
    /// Existing Kestrel logical source identity. Retain it through the migration so the
    /// relay can continue using `content_row_aad(device_id, local_seq, row_type)`.
    pub device_id: String,
    pub local_seq: i64,
    pub row_type: String,
    pub aggregate_type: String,
    pub aggregate_id: String,
    pub operation_kind: OutboxOperationKind,
    /// Canonical JSON snapshot/event. Encrypt only at the transport boundary exactly as
    /// `sync-engine::content::encode_row` does today; never log this plaintext.
    pub payload_json: String,
    pub connector: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedOutboxOperation {
    pub operation_id: String,
    pub device_id: String,
    pub local_seq: i64,
    pub row_type: String,
    pub aggregate_type: String,
    pub aggregate_id: String,
    pub operation_kind: OutboxOperationKind,
    pub payload_json: String,
    pub connector: String,
    pub attempts: i64,
}

pub fn ensure_sync_outbox_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "BEGIN;
         CREATE TABLE IF NOT EXISTS sync_outbox (
            operation_id TEXT PRIMARY KEY NOT NULL,
            -- One logical durable source operation. This unique key preserves the
            -- existing relay identity during the transition from cursor scanning.
            device_id TEXT NOT NULL,
            local_seq INTEGER NOT NULL,
            row_type TEXT NOT NULL,
            aggregate_type TEXT NOT NULL,
            aggregate_id TEXT NOT NULL,
            operation_kind TEXT NOT NULL CHECK(operation_kind IN ('upsert','tombstone')),
            payload_json TEXT NOT NULL,
            connector TEXT NOT NULL,
            state TEXT NOT NULL CHECK(state IN ('pending','leased','delivered','blocked')),
            attempts INTEGER NOT NULL DEFAULT 0,
            next_attempt_at_unix_ms INTEGER NOT NULL,
            lease_owner TEXT,
            lease_expires_at_unix_ms INTEGER,
            delivered_at_unix_ms INTEGER,
            remote_receipt TEXT,
            last_error_code TEXT,
            last_error_message TEXT,
            created_at_unix_ms INTEGER NOT NULL,
            updated_at_unix_ms INTEGER NOT NULL,
            UNIQUE(device_id, local_seq, row_type)
         );
         CREATE INDEX IF NOT EXISTS idx_sync_outbox_dispatch
           ON sync_outbox(connector, state, next_attempt_at_unix_ms, created_at_unix_ms);
         CREATE INDEX IF NOT EXISTS idx_sync_outbox_lease
           ON sync_outbox(state, lease_expires_at_unix_ms);
         COMMIT;",
    )
}

/// Allocates the next Kestrel-wide local sequence *inside* the caller's mutation
/// transaction. Do not call Store::next_local_seq here: the current method executes
/// directly on `Connection`, which would escape the transaction boundary.
fn next_local_seq_tx(tx: &Transaction<'_>) -> rusqlite::Result<i64> {
    tx.query_row(
        "UPDATE sync_local_seq_counter
         SET next_seq = next_seq + 1
         WHERE id = 1
         RETURNING next_seq - 1",
        [],
        |row| row.get(0),
    )
}

/// Example shape for existing Store write methods. The actual `insert_meeting`,
/// `complete_meeting`, `set_meeting_transcript`, `insert_transcript_segments`, and
/// user-note update methods should each get a `*_tx` implementation that accepts the
/// same transaction. Their public methods become thin transaction wrappers.
///
/// The important invariant is one COMMIT contains:
///   1. canonical domain mutation;
///   2. `local_seq` allocation;
///   3. exact sync payload construction; and
///   4. the outbox row.
pub fn complete_meeting_and_enqueue_tx(
    tx: &Transaction<'_>,
    device_id: &str,
    meeting_id: i64,
    end_unix_ms: i64,
    audio_path: Option<&str>,
    now_unix_ms: i64,
) -> rusqlite::Result<String> {
    // Replace this with the existing complete_meeting SQL moved into a transaction-aware
    // helper. `updated_meeting_json` must be a stable serialization of the post-write
    // meeting snapshot, including the stable source meeting id.
    tx.execute(
        "UPDATE meeting
         SET end_unix_ms = ?1, audio_path = ?2, status = 'completed'
         WHERE id = ?3",
        params![end_unix_ms, audio_path, meeting_id],
    )?;

    let local_seq = next_local_seq_tx(tx)?;
    let updated_meeting_json: String = tx.query_row(
        "SELECT json_object(
            'id', id,
            'start_unix_ms', start_unix_ms,
            'end_unix_ms', end_unix_ms,
            'title', title,
            'status', status
         ) FROM meeting WHERE id = ?1",
        params![meeting_id],
        |row| row.get(0),
    )?;

    enqueue_outbox_tx(
        tx,
        NewOutboxOperation {
            device_id: device_id.to_owned(),
            local_seq,
            row_type: "meeting".to_owned(),
            aggregate_type: "meeting".to_owned(),
            aggregate_id: meeting_id.to_string(),
            operation_kind: OutboxOperationKind::Upsert,
            payload_json: updated_meeting_json,
            connector: "kestrel_data_plane".to_owned(),
        },
        now_unix_ms,
    )
}

pub fn enqueue_outbox_tx(
    tx: &Transaction<'_>,
    operation: NewOutboxOperation,
    now_unix_ms: i64,
) -> rusqlite::Result<String> {
    let operation_id = Uuid::new_v4().to_string();
    tx.execute(
        "INSERT INTO sync_outbox (
             operation_id, device_id, local_seq, row_type, aggregate_type, aggregate_id,
             operation_kind, payload_json, connector, state, attempts, next_attempt_at_unix_ms,
             created_at_unix_ms, updated_at_unix_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0, ?11, ?11, ?11)",
        params![
            operation_id,
            operation.device_id,
            operation.local_seq,
            operation.row_type,
            operation.aggregate_type,
            operation.aggregate_id,
            operation.operation_kind.as_str(),
            operation.payload_json,
            operation.connector,
            OUTBOX_PENDING,
            now_unix_ms,
        ],
    )?;
    Ok(operation_id)
}

/// Claims a bounded ordered batch. `UPDATE ... RETURNING` is atomic at SQLite level;
/// only one dispatcher can obtain a live lease for each operation. Expired leases become
/// eligible for recovery, but remote delivery stays idempotent because the transport
/// uses `operation_id` as the idempotency key.
pub fn claim_due_outbox_batch(
    conn: &Connection,
    connector: &str,
    worker_id: &str,
    now_unix_ms: i64,
    lease_until_unix_ms: i64,
    limit: i64,
) -> rusqlite::Result<Vec<ClaimedOutboxOperation>> {
    let mut statement = conn.prepare(
        "WITH due AS (
            SELECT operation_id
            FROM sync_outbox
            WHERE connector = ?1
              AND state IN ('pending', 'leased')
              AND next_attempt_at_unix_ms <= ?2
              AND (state = 'pending' OR lease_expires_at_unix_ms <= ?2)
            ORDER BY created_at_unix_ms, local_seq
            LIMIT ?3
         )
         UPDATE sync_outbox
         SET state = ?4,
             attempts = attempts + 1,
             lease_owner = ?5,
             lease_expires_at_unix_ms = ?6,
             updated_at_unix_ms = ?2
         WHERE operation_id IN due
         RETURNING operation_id, device_id, local_seq, row_type, aggregate_type,
                   aggregate_id, operation_kind, payload_json, connector, attempts",
    )?;

    statement
        .query_map(
            params![
                connector,
                now_unix_ms,
                limit,
                OUTBOX_LEASED,
                worker_id,
                lease_until_unix_ms,
            ],
            |row| {
                let operation_kind = match row.get::<_, String>(6)?.as_str() {
                    "upsert" => OutboxOperationKind::Upsert,
                    "tombstone" => OutboxOperationKind::Tombstone,
                    other => return Err(rusqlite::Error::InvalidParameterName(other.to_owned())),
                };
                Ok(ClaimedOutboxOperation {
                    operation_id: row.get(0)?,
                    device_id: row.get(1)?,
                    local_seq: row.get(2)?,
                    row_type: row.get(3)?,
                    aggregate_type: row.get(4)?,
                    aggregate_id: row.get(5)?,
                    operation_kind,
                    payload_json: row.get(7)?,
                    connector: row.get(8)?,
                    attempts: row.get(9)?,
                })
            },
        )?
        .collect()
}

/// Acknowledge only the exact worker lease. If the process crashed after the remote
/// accepted the operation but before this local update, a later worker will resend the
/// same `operation_id`; the relay must return the original receipt instead of applying
/// the domain effect twice.
pub fn acknowledge_outbox_operation(
    conn: &Connection,
    operation_id: &str,
    worker_id: &str,
    remote_receipt: &str,
    now_unix_ms: i64,
) -> rusqlite::Result<bool> {
    let affected = conn.execute(
        "UPDATE sync_outbox
         SET state = ?1,
             delivered_at_unix_ms = ?2,
             remote_receipt = ?3,
             lease_owner = NULL,
             lease_expires_at_unix_ms = NULL,
             last_error_code = NULL,
             last_error_message = NULL,
             updated_at_unix_ms = ?2
         WHERE operation_id = ?4 AND state = ?5 AND lease_owner = ?6",
        params![
            OUTBOX_DELIVERED,
            now_unix_ms,
            remote_receipt,
            operation_id,
            OUTBOX_LEASED,
            worker_id,
        ],
    )?;
    Ok(affected == 1)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryFailureClass {
    Retryable,
    NeedsAuthentication,
    PermanentValidation,
}

/// Do not delete on error. Retryable errors release a delayed operation; authentication
/// and policy/validation errors become explicit blocked work for source-specific recovery.
pub fn release_outbox_after_failure(
    conn: &Connection,
    operation_id: &str,
    worker_id: &str,
    class: DeliveryFailureClass,
    error_code: &str,
    redacted_message: &str,
    retry_at_unix_ms: Option<i64>,
    now_unix_ms: i64,
) -> rusqlite::Result<bool> {
    let (new_state, next_attempt_at) = match class {
        DeliveryFailureClass::Retryable => (OUTBOX_PENDING, retry_at_unix_ms.unwrap_or(now_unix_ms)),
        DeliveryFailureClass::NeedsAuthentication | DeliveryFailureClass::PermanentValidation => {
            (OUTBOX_BLOCKED, i64::MAX)
        }
    };
    let affected = conn.execute(
        "UPDATE sync_outbox
         SET state = ?1,
             next_attempt_at_unix_ms = ?2,
             lease_owner = NULL,
             lease_expires_at_unix_ms = NULL,
             last_error_code = ?3,
             last_error_message = ?4,
             updated_at_unix_ms = ?5
         WHERE operation_id = ?6 AND state = ?7 AND lease_owner = ?8",
        params![
            new_state,
            next_attempt_at,
            error_code,
            redacted_message,
            now_unix_ms,
            operation_id,
            OUTBOX_LEASED,
            worker_id,
        ],
    )?;
    Ok(affected == 1)
}

/// Intended Store-level wrapper pattern. Each public mutation uses this shape; do not
/// perform an auto-committed domain write and then invoke `enqueue_outbox_tx` separately.
pub fn mutate_then_enqueue<T>(
    conn: &mut Connection,
    now_unix_ms: i64,
    mutation: impl FnOnce(&Transaction<'_>) -> rusqlite::Result<(T, NewOutboxOperation)>,
) -> rusqlite::Result<(T, String)> {
    let tx = conn.transaction()?;
    let (result, operation) = mutation(&tx)?;
    let operation_id = enqueue_outbox_tx(&tx, operation, now_unix_ms)?;
    tx.commit()?;
    Ok((result, operation_id))
}

// Implementation note:
// The owned operation deliberately pays a small allocation cost at the transaction
// boundary. This makes it impossible for a JSON payload borrowed from a mutation helper
// to outlive that helper and keeps the atomicity contract straightforward. Optimize only
// after profiling a representative first-sync backlog.
