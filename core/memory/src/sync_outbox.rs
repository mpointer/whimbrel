//! Transactional outbox — shadow mode (docs/adr/0001-core-execution-boundary-and-transactional-outbox.md).
//!
//! Adapted from docs/refactor-package/snippets/transactional_outbox.rs. The caller
//! performs the domain mutation and `enqueue_outbox_tx` in the SAME SQLite transaction
//! (`Connection::unchecked_transaction`, matching every other multi-statement write in
//! this crate — see `run_episode_segment_job` for precedent). A crash therefore yields
//! either neither change, or a local change with a replay-safe outgoing operation.
//!
//! This module does not send anything over the network and nothing currently reads
//! `sync_outbox` outside its own tests — see the ADR for why this is deliberate.

use rusqlite::{params, Connection, OptionalExtension, Transaction};

const OUTBOX_PENDING: &str = "pending";
const OUTBOX_LEASED: &str = "leased";
const OUTBOX_DELIVERED: &str = "delivered";
const OUTBOX_BLOCKED: &str = "blocked";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// Existing Whimbrel logical source identity, so a future dispatcher can still use
    /// `content_row_aad(device_id, local_seq, row_type)` exactly as `sync-engine` does
    /// today for the legacy path.
    pub device_id: String,
    pub local_seq: i64,
    pub row_type: String,
    pub aggregate_type: String,
    pub aggregate_id: String,
    pub operation_kind: OutboxOperationKind,
    /// Canonical JSON snapshot. Encrypt only at the transport boundary, exactly as
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryFailureClass {
    Retryable,
    NeedsAuthentication,
    PermanentValidation,
}

/// A random 128-bit id, hex-encoded. Deliberately not the `uuid` crate: this crate
/// already depends on `rand`/`hex` (see Cargo.toml's MCP-token-generation comment), and
/// a fresh dependency isn't warranted for "unique opaque string."
fn new_operation_id() -> String {
    let bytes: [u8; 16] = rand::random();
    hex::encode(bytes)
}

pub(crate) fn ensure_sync_outbox_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS sync_outbox (
            operation_id TEXT PRIMARY KEY NOT NULL,
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
           ON sync_outbox(state, lease_expires_at_unix_ms);",
    )
}

/// Must be called inside the caller's mutation transaction — never a standalone commit.
pub(crate) fn enqueue_outbox_tx(
    tx: &Transaction<'_>,
    operation: NewOutboxOperation,
    now_unix_ms: i64,
) -> rusqlite::Result<String> {
    let operation_id = new_operation_id();
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

/// Claims a bounded ordered batch. `UPDATE ... RETURNING` is atomic at the SQLite level;
/// only one caller can obtain a live lease for each operation at a time. Expired leases
/// become eligible for recovery, but remote delivery stays idempotent because the
/// transport is expected to use `operation_id` as its idempotency key (SYNC-107).
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

    let rows = statement
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
                    other => {
                        return Err(rusqlite::Error::InvalidColumnType(
                            6,
                            other.to_owned(),
                            rusqlite::types::Type::Text,
                        ))
                    }
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
        .collect();
    rows
}

/// Acknowledge only the exact worker lease. If the process crashed after the remote
/// accepted the operation but before this local update, a later worker resends the same
/// `operation_id`; the transport must return the original receipt instead of applying the
/// domain effect twice (SYNC-101).
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

/// Do not delete on error. A retryable error releases the row back to pending with a
/// delay; authentication and validation errors become explicit blocked work for
/// source-specific recovery (SYNC-104, SYNC-105).
#[allow(clippy::too_many_arguments)]
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
        DeliveryFailureClass::Retryable => {
            (OUTBOX_PENDING, retry_at_unix_ms.unwrap_or(now_unix_ms))
        }
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

/// Test/diagnostic seam: read back one row's durable state without exposing the whole
/// row shape as a public API surface yet (no real caller needs more than this today).
pub fn outbox_operation_state(
    conn: &Connection,
    operation_id: &str,
) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT state FROM sync_outbox WHERE operation_id = ?1",
        params![operation_id],
        |row| row.get(0),
    )
    .optional()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        ensure_sync_outbox_schema(&conn).unwrap();
        conn
    }

    fn sample_op(local_seq: i64) -> NewOutboxOperation {
        NewOutboxOperation {
            device_id: "device-a".to_string(),
            local_seq,
            row_type: "meeting".to_string(),
            aggregate_type: "meeting".to_string(),
            aggregate_id: "42".to_string(),
            operation_kind: OutboxOperationKind::Upsert,
            payload_json: r#"{"id":42,"status":"completed"}"#.to_string(),
            connector: "kestrel_data_plane".to_string(),
        }
    }

    /// SYNC-001/002 shape: failure before commit leaves nothing durable.
    #[test]
    fn enqueue_rolls_back_completely_if_the_transaction_never_commits() {
        let mut conn = setup();
        {
            let tx = conn.transaction().unwrap();
            let id = enqueue_outbox_tx(&tx, sample_op(1), 1000).unwrap();
            assert!(outbox_operation_state(&tx, &id).unwrap().is_some());
            // Dropped without commit == a crash before commit.
        }
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM sync_outbox", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            count, 0,
            "an uncommitted transaction must leave no outbox row"
        );
    }

    /// SYNC-003: commit succeeds, so the operation is durably pending for a later
    /// dispatcher to reclaim (simulated here by directly reading it back).
    #[test]
    fn enqueue_is_durable_once_committed() {
        let mut conn = setup();
        let tx = conn.transaction().unwrap();
        let id = enqueue_outbox_tx(&tx, sample_op(1), 1000).unwrap();
        tx.commit().unwrap();
        assert_eq!(
            outbox_operation_state(&conn, &id).unwrap().as_deref(),
            Some("pending")
        );
    }

    #[test]
    fn claim_leases_only_one_worker_at_a_time() {
        let mut conn = setup();
        let tx = conn.transaction().unwrap();
        let id = enqueue_outbox_tx(&tx, sample_op(1), 1000).unwrap();
        tx.commit().unwrap();

        let claimed =
            claim_due_outbox_batch(&conn, "kestrel_data_plane", "worker-1", 2000, 32000, 10)
                .unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].operation_id, id);
        assert_eq!(claimed[0].attempts, 1);

        // SYNC-107: a second worker racing for the same due row gets nothing while the
        // lease is live.
        let second =
            claim_due_outbox_batch(&conn, "kestrel_data_plane", "worker-2", 2001, 32001, 10)
                .unwrap();
        assert!(second.is_empty());
    }

    /// SYNC-107 recovery half: an expired lease becomes claimable again by another
    /// worker, and the attempt count increments (evidence a retry actually happened).
    #[test]
    fn expired_lease_is_reclaimed_by_a_different_worker() {
        let mut conn = setup();
        let tx = conn.transaction().unwrap();
        enqueue_outbox_tx(&tx, sample_op(1), 1000).unwrap();
        tx.commit().unwrap();

        let first = claim_due_outbox_batch(&conn, "kestrel_data_plane", "worker-1", 2000, 2500, 10)
            .unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].attempts, 1);

        // Lease expires at 2500; a claim attempt at 3000 should reclaim it.
        let recovered =
            claim_due_outbox_batch(&conn, "kestrel_data_plane", "worker-2", 3000, 3500, 10)
                .unwrap();
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].operation_id, first[0].operation_id);
        assert_eq!(
            recovered[0].attempts, 2,
            "attempts must increment on reclaim"
        );
    }

    /// SYNC-101: remote accepted, crash before local ack — same operation_id resends,
    /// and only the owning lease's ack call actually marks it delivered (an ack from the
    /// wrong/expired worker id must not silently succeed).
    #[test]
    fn acknowledge_requires_the_exact_current_lease_owner() {
        let mut conn = setup();
        let tx = conn.transaction().unwrap();
        enqueue_outbox_tx(&tx, sample_op(1), 1000).unwrap();
        tx.commit().unwrap();

        let claimed =
            claim_due_outbox_batch(&conn, "kestrel_data_plane", "worker-1", 2000, 32000, 10)
                .unwrap();
        let op_id = &claimed[0].operation_id;

        // A stale/foreign worker id must not be able to acknowledge.
        let ok = acknowledge_outbox_operation(&conn, op_id, "worker-imposter", "receipt-1", 3000)
            .unwrap();
        assert!(!ok);
        assert_eq!(
            outbox_operation_state(&conn, op_id).unwrap().as_deref(),
            Some("leased")
        );

        // The real lease owner acknowledges successfully, exactly once.
        let ok = acknowledge_outbox_operation(&conn, op_id, "worker-1", "receipt-1", 3001).unwrap();
        assert!(ok);
        assert_eq!(
            outbox_operation_state(&conn, op_id).unwrap().as_deref(),
            Some("delivered")
        );

        // A resend attempt (simulating "crashed before ack, retried") finds the
        // operation already delivered and out of lease — a second ack call is a no-op,
        // proving there is exactly one durable delivered state, not a second effect.
        let ok = acknowledge_outbox_operation(&conn, op_id, "worker-1", "receipt-1", 3002).unwrap();
        assert!(
            !ok,
            "acknowledging an already-delivered operation must not re-fire"
        );
    }

    /// SYNC-104: a retryable failure returns the row to pending with backoff; it must
    /// become claimable again after its new next_attempt_at, and never silently vanish.
    #[test]
    fn retryable_failure_returns_to_pending_with_backoff() {
        let mut conn = setup();
        let tx = conn.transaction().unwrap();
        enqueue_outbox_tx(&tx, sample_op(1), 1000).unwrap();
        tx.commit().unwrap();

        let claimed =
            claim_due_outbox_batch(&conn, "kestrel_data_plane", "worker-1", 2000, 32000, 10)
                .unwrap();
        let op_id = &claimed[0].operation_id;

        let ok = release_outbox_after_failure(
            &conn,
            op_id,
            "worker-1",
            DeliveryFailureClass::Retryable,
            "timeout",
            "request timed out",
            Some(5000),
            2100,
        )
        .unwrap();
        assert!(ok);
        assert_eq!(
            outbox_operation_state(&conn, op_id).unwrap().as_deref(),
            Some("pending")
        );

        // Not yet due.
        let too_early =
            claim_due_outbox_batch(&conn, "kestrel_data_plane", "worker-2", 4000, 34000, 10)
                .unwrap();
        assert!(too_early.is_empty());

        // Due now.
        let retried =
            claim_due_outbox_batch(&conn, "kestrel_data_plane", "worker-2", 5000, 35000, 10)
                .unwrap();
        assert_eq!(retried.len(), 1);
        assert_eq!(retried[0].attempts, 2);
    }

    /// SYNC-105: an authentication failure blocks the operation (no hot retry loop) and
    /// it never comes due again on its own.
    #[test]
    fn auth_failure_blocks_without_hot_retry() {
        let mut conn = setup();
        let tx = conn.transaction().unwrap();
        enqueue_outbox_tx(&tx, sample_op(1), 1000).unwrap();
        tx.commit().unwrap();

        let claimed =
            claim_due_outbox_batch(&conn, "kestrel_data_plane", "worker-1", 2000, 32000, 10)
                .unwrap();
        let op_id = &claimed[0].operation_id;

        release_outbox_after_failure(
            &conn,
            op_id,
            "worker-1",
            DeliveryFailureClass::NeedsAuthentication,
            "auth_revoked",
            "credential revoked",
            None,
            2100,
        )
        .unwrap();
        assert_eq!(
            outbox_operation_state(&conn, op_id).unwrap().as_deref(),
            Some("blocked")
        );

        let never_claimed = claim_due_outbox_batch(
            &conn,
            "kestrel_data_plane",
            "worker-2",
            999_999_999,
            1_000_000_000,
            10,
        )
        .unwrap();
        assert!(
            never_claimed.is_empty(),
            "a blocked operation must not be claimable again"
        );
    }

    /// SYNC-106: the same operation delivered/returned many times still yields exactly
    /// one durable delivered state — the unique (device_id, local_seq, row_type)
    /// constraint plus idempotent ack (proven above) are what a real dispatcher relies
    /// on; this test proves the constraint side directly.
    #[test]
    fn duplicate_enqueue_for_the_same_local_seq_is_rejected_by_the_unique_constraint() {
        let mut conn = setup();
        let tx = conn.transaction().unwrap();
        enqueue_outbox_tx(&tx, sample_op(7), 1000).unwrap();
        // Same device_id/local_seq/row_type as above -> violates UNIQUE.
        let result = enqueue_outbox_tx(&tx, sample_op(7), 1001);
        assert!(result.is_err());
    }
}
