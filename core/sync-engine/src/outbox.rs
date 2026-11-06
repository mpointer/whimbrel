//! Outbox dispatcher (docs/adr/0001, slice 2): drains `memory`'s `sync_outbox` into the
//! relay's `sync_row` table. Runs right after the legacy `push_content` scan inside the
//! same `sync_content` round. Both paths insert the same plaintext under the same
//! `(device_id, local_seq, row_type)` key with `ON CONFLICT DO NOTHING`, so they coexist
//! until the legacy scan is retired; a row either path already delivered is a no-op for
//! the other. Delivery is leased per operation and acknowledged only by the lease owner,
//! so a crash between the relay write and the local ack simply resends the same key.

use std::collections::HashMap;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use libsql::{params, Connection};
use memory::{ClaimedOutboxOperation, DeliveryFailureClass, MeetingStatus, Store};
use sync::GroupKey;

use crate::content::{
    all_peers_items_capable, content_row_aad, is_missing_entity_tag_column, outbox_entity_identity, tag_for,
    CompactionBudget, ENTITY_TAG_COLUMN_SEEN, MAX_SKIN_CHUNK_BYTES_PER_PASS,
};
use crate::SyncEngineError;

const CLAIM_BATCH: i64 = 100;
/// ponytail: `sync_content` runs under the 30s `SYNC_TIMEOUT` on Swift's serial
/// `accessQueue`, and every delivered row is one Turso round trip, so one pass is capped
/// well inside that budget and any remainder drains on the next sync tick. Upgrade path:
/// a dedicated background dispatcher off `accessQueue` if a backlog is ever seen to lag.
const MAX_ROWS_PER_PASS: usize = 100;
const LEASE_MS: i64 = 60_000;
const BASE_BACKOFF_MS: i64 = 30_000;
const MAX_BACKOFF_MS: i64 = 60 * 60 * 1000;
/// A row held back by a gate (parent meeting still recording, or a peer not yet
/// `items`-capable) is not a failure; it is simply re-checked on a later round.
const DEFERRED_RECHECK_MS: i64 = 60_000;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct OutboxDispatchSummary {
    pub delivered: usize,
    pub deferred: usize,
    pub failed: usize,
    /// `items` rows acknowledged without a relay write because a peer is still
    /// pre-cutover: the legacy scan pushes those in the old `transcript_segment` format,
    /// and delivering them again as `items` after the cutover would give the peer the
    /// same segment under two row types.
    pub legacy_owned: usize,
    /// `items` rows of a meeting that no longer exists (forgotten, or removed by retention, merge or
    /// split): acknowledged unsent with receipt `dropped:parent_meeting_gone`.
    pub dropped: usize,
}

/// Row types the legacy `push_content` scan also sends, under the outbox op's own `local_seq`.
fn legacy_twin(row_type: &str) -> bool {
    matches!(row_type, memory::OUTBOX_ROW_TYPE_MEETING | memory::OUTBOX_ROW_TYPE_ITEMS)
}

pub async fn dispatch_outbox(
    conn: &Connection,
    group_key: &GroupKey,
    store: &Store,
    device_id: &str,
) -> Result<OutboxDispatchSummary, SyncEngineError> {
    dispatch_outbox_at(conn, group_key, store, device_id, crate::now_unix_ms()).await
}

/// Clock-injected body of `dispatch_outbox`, so tests can advance past lease/backoff
/// windows without sleeping.
pub(crate) async fn dispatch_outbox_at(
    conn: &Connection,
    group_key: &GroupKey,
    store: &Store,
    device_id: &str,
    now_unix_ms: i64,
) -> Result<OutboxDispatchSummary, SyncEngineError> {
    let worker_id = format!("{device_id}:{}", std::process::id());
    let items_capable = all_peers_items_capable(conn, device_id).await?;
    let mut summary = OutboxDispatchSummary::default();
    let mut parent_terminal: HashMap<i64, Option<bool>> = HashMap::new();
    let mut processed = 0;
    let mut chunk_bytes = 0usize;
    // As `push_content`: untagged while the column is known missing or the epoch is unreadable.
    let mut epoch = if ENTITY_TAG_COLUMN_SEEN.get() {
        store
            .sync_entity_epoch()
            .inspect_err(|e| eprintln!("[sync] dispatching untagged, no entity epoch: {e}"))
            .ok()
    } else {
        None
    };
    let mut compaction = CompactionBudget::default();
    // ponytail: per-pass batching can leak one stale row per entity when SYNC_TIMEOUT hits mid-pass; upgrade path: delete before acknowledging
    let mut tags: Vec<String> = Vec::new();

    while processed < MAX_ROWS_PER_PASS {
        let batch = store
            .claim_due_outbox_batch(
                memory::OUTBOX_CONNECTOR,
                &worker_id,
                now_unix_ms,
                now_unix_ms + LEASE_MS,
                CLAIM_BATCH,
            )
            .map_err(query_error)?;
        if batch.is_empty() {
            break;
        }
        for mut op in batch {
            // The image budget: chunks past it wait for the next round (not a failure, no backoff), and
            // don't count toward this pass's rows, so rows enqueued after a large image still go now.
            if op.row_type == memory::OUTBOX_ROW_TYPE_SKIN_IMAGE_CHUNK {
                if chunk_bytes >= MAX_SKIN_CHUNK_BYTES_PER_PASS {
                    store
                        .release_outbox_after_failure(
                            &op.operation_id,
                            &worker_id,
                            DeliveryFailureClass::Retryable,
                            "skin_image_budget",
                            "held for the next round",
                            Some(now_unix_ms + 1),
                            now_unix_ms,
                        )
                        .map_err(query_error)?;
                    summary.deferred += 1;
                    continue;
                }
                // About what the sealed, base64-encoded row will be.
                chunk_bytes += op.payload_json.len() * 4 / 3;
            }
            processed += 1;
            if op.row_type == memory::OUTBOX_ROW_TYPE_ITEMS && !items_capable {
                store
                    .acknowledge_outbox_operation(
                        &op.operation_id,
                        &worker_id,
                        "legacy_owned:peer_not_items_capable",
                        now_unix_ms,
                    )
                    .map_err(query_error)?;
                summary.legacy_owned += 1;
                continue;
            }
            if let Some(code) = deferral_reason(&op, store, &mut parent_terminal) {
                if code == PARENT_MEETING_GONE {
                    // A line of a deleted meeting (forgotten, or by retention): nothing to send, ever.
                    store
                        .acknowledge_outbox_operation(&op.operation_id, &worker_id, "dropped:parent_meeting_gone", now_unix_ms)
                        .map_err(query_error)?;
                    summary.dropped += 1;
                    continue;
                }
                store
                    .release_outbox_after_failure(
                        &op.operation_id,
                        &worker_id,
                        DeliveryFailureClass::Retryable,
                        code,
                        "held until the gate clears",
                        Some(now_unix_ms + DEFERRED_RECHECK_MS),
                        now_unix_ms,
                    )
                    .map_err(query_error)?;
                summary.deferred += 1;
                continue;
            }
            // #600: rows only the outbox sends get a fresh number on every send, above any puller's cursor.
            // Meeting and transcript rows keep theirs: the legacy push sent the same row under it.
            if !legacy_twin(&op.row_type) {
                op.local_seq = store.restamp_outbox_operation(&op.operation_id).map_err(query_error)?;
            }
            let tag = epoch
                .as_deref()
                .zip(outbox_entity_identity(&op))
                .map(|(epoch, id)| tag_for(group_key, &op.device_id, epoch, &id));
            match write_sync_row(conn, group_key, &op, tag.as_deref(), now_unix_ms).await {
                Ok((receipt, tagged)) => {
                    // Acknowledged before the batched DELETE (spec D8.2).
                    store
                        .acknowledge_outbox_operation(&op.operation_id, &worker_id, &receipt, now_unix_ms)
                        .map_err(query_error)?;
                    summary.delivered += 1;
                    // So forgetting the meeting deletes this row, even when the forget came while it was
                    // in flight (the recorder's cleanup finds it next round).
                    if let Some(meeting_id) = op_meeting_id(&op) {
                        store
                            .record_pushed_meeting_row(meeting_id, op.local_seq, &op.row_type)
                            .map_err(query_error)?;
                    }
                    match tag {
                        Some(t) if tagged => {
                            if !tags.contains(&t) {
                                tags.push(t);
                            }
                            if tags.len() >= 50 {
                                compaction.compact(conn, device_id, &tags).await;
                                tags.clear();
                            }
                        }
                        // The relay has no `entity_tag`: the rest of the pass goes untagged.
                        Some(_) => epoch = None,
                        None => {}
                    }
                }
                Err(error) => {
                    // ponytail: every relay error is retried with backoff — `SyncEngineError`
                    // has no auth variant, so an expired Turso token backs off (max 1h)
                    // instead of parking the row as `blocked`. Upgrade: map libsql's
                    // 401/403 to `NeedsAuthentication` once its error type exposes status.
                    let message = error.to_string();
                    store
                        .release_outbox_after_failure(
                            &op.operation_id,
                            &worker_id,
                            DeliveryFailureClass::Retryable,
                            "relay_write",
                            &message[..message.len().min(200)],
                            Some(now_unix_ms + backoff_ms(op.attempts)),
                            now_unix_ms,
                        )
                        .map_err(query_error)?;
                    summary.failed += 1;
                }
            }
        }
    }
    compaction.compact(conn, device_id, &tags).await;
    Ok(summary)
}

/// `deferral_reason` for an `items` row whose meeting no longer exists: acknowledged unsent.
const PARENT_MEETING_GONE: &str = "parent_meeting_gone";

/// The own meeting a `meeting`, transcript `items`, `live_alert` or `meeting_speaker` op belongs to. A speaker
/// row about another device's meeting (`origin_device_id` set) names no meeting of this device.
fn op_meeting_id(op: &ClaimedOutboxOperation) -> Option<i64> {
    let source_meeting_id = || {
        serde_json::from_str::<serde_json::Value>(&op.payload_json).ok()?.get("source_meeting_id")?.as_i64()
    };
    match op.row_type.as_str() {
        memory::OUTBOX_ROW_TYPE_MEETING => op.aggregate_id.parse().ok(),
        memory::OUTBOX_ROW_TYPE_ITEMS | memory::OUTBOX_ROW_TYPE_LIVE_ALERT => source_meeting_id(),
        // Named `<origin>:<source id>:<speaker>`, with an empty origin for this device's own meeting.
        memory::OUTBOX_ROW_TYPE_MEETING_SPEAKER if op.aggregate_id.starts_with(':') => source_meeting_id(),
        _ => None,
    }
}

/// `items`-format rows carry the transcript of a meeting that may still be recording;
/// the legacy `unpushed_item_rows` scan applies the same gate at query time. A meeting
/// that is gone (forgotten while the row was in flight) gives `PARENT_MEETING_GONE`.
fn deferral_reason(
    op: &ClaimedOutboxOperation,
    store: &Store,
    parent_terminal: &mut HashMap<i64, Option<bool>>,
) -> Option<&'static str> {
    if op.row_type != memory::OUTBOX_ROW_TYPE_ITEMS {
        return None;
    }
    let meeting_id = op_meeting_id(op)?;
    let terminal = *parent_terminal.entry(meeting_id).or_insert_with(|| {
        match store.get_meeting(meeting_id) {
            Ok(Some(m)) => Some(matches!(m.status, MeetingStatus::Completed | MeetingStatus::Cancelled)),
            Ok(None) => None,
            // An unreadable meeting is held, never dropped.
            Err(_) => Some(false),
        }
    });
    match terminal {
        None => Some(PARENT_MEETING_GONE),
        Some(false) => Some("parent_meeting_not_terminal"),
        Some(true) => None,
    }
}

/// Writes one op's row. The `bool` is true when it went in with `tag`; a relay that
/// predates `entity_tag` gets the untagged INSERT instead.
async fn write_sync_row(
    conn: &Connection,
    group_key: &GroupKey,
    op: &ClaimedOutboxOperation,
    tag: Option<&str>,
    now_unix_ms: i64,
) -> Result<(String, bool), SyncEngineError> {
    let aad = content_row_aad(&op.device_id, op.local_seq, &op.row_type);
    let ciphertext = STANDARD.encode(sync::encrypt_payload(group_key, op.payload_json.as_bytes(), &aad));
    let untagged = || {
        conn.execute(
            "INSERT INTO sync_row (device_id, local_seq, row_type, ciphertext, created_at_unix_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT (device_id, local_seq, row_type) DO NOTHING",
            params![op.device_id.as_str(), op.local_seq, op.row_type.as_str(), ciphertext.as_str(), now_unix_ms],
        )
    };
    let tagged = match tag {
        None => untagged().await.map(|_| false),
        Some(tag) => match conn
            .execute(
                "INSERT INTO sync_row (device_id, local_seq, row_type, ciphertext, created_at_unix_ms, entity_tag) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
                 ON CONFLICT (device_id, local_seq, row_type) DO NOTHING",
                params![op.device_id.as_str(), op.local_seq, op.row_type.as_str(), ciphertext.as_str(), now_unix_ms, tag],
            )
            .await
        {
            Err(e) if is_missing_entity_tag_column(&e.to_string()) => untagged().await.map(|_| false),
            other => other.map(|_| true),
        },
    }
    .map_err(|e| SyncEngineError::Query(e.to_string()))?;
    Ok((format!("sync_row:{}:{}:{}", op.device_id, op.local_seq, op.row_type), tagged))
}

fn backoff_ms(attempts: i64) -> i64 {
    let exponent = attempts.saturating_sub(1).clamp(0, 16) as u32;
    (BASE_BACKOFF_MS << exponent).min(MAX_BACKOFF_MS)
}

fn query_error(e: impl std::fmt::Display) -> SyncEngineError {
    SyncEngineError::Query(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{ensure_content_schema, pull_content, ROW_TYPE_ITEMS, ROW_TYPE_MEETING};
    use memory::{MeetingTrigger, NewMeeting, NewTranscriptSegment};

    /// Outbox rows are stamped with the real clock at enqueue, so the injected clock
    /// starts from it and only ever moves forward.
    fn t0() -> i64 {
        crate::now_unix_ms() + 1
    }

    async fn local_connection(path: &std::path::Path) -> Connection {
        let db = libsql::Builder::new_local(path).build().await.unwrap();
        db.connect().unwrap()
    }

    fn own_store(device_id: &str) -> (Store, i64) {
        let store = Store::open_in_memory().unwrap();
        store.set_own_device_id(device_id);
        let meeting_id = store
            .insert_meeting(&NewMeeting {
                device_id: device_id.to_string(),
                start_unix_ms: 1000,
                end_unix_ms: None,
                title: "Outbox Standup".to_string(),
                attendees: vec![],
                trigger: MeetingTrigger::Calendar,
                audio_path: None,
                status: memory::MeetingStatus::Recording,
            })
            .unwrap();
        (store, meeting_id)
    }

    async fn relay_rows(conn: &Connection, device_id: &str, row_type: &str) -> i64 {
        let mut rows = conn
            .query(
                "SELECT COUNT(*) FROM sync_row WHERE device_id = ?1 AND row_type = ?2",
                params![device_id, row_type],
            )
            .await
            .unwrap();
        rows.next().await.unwrap().unwrap().get(0).unwrap()
    }

    fn states(store: &Store) -> Vec<String> {
        store.list_outbox_operations().unwrap().into_iter().map(|r| r.2).collect()
    }

    /// The outbox path alone (no legacy `push_content` in this test) delivers a row a
    /// peer decodes and applies exactly as it would a legacy-pushed one.
    #[tokio::test]
    async fn dispatch_delivers_an_own_meeting_row_that_a_peer_can_pull() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("relay.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        let group_key = GroupKey::generate();
        let (store_a, meeting_id) = own_store("device-a");
        store_a.complete_meeting(meeting_id, 2000, None).unwrap();
        let t0 = t0();

        let summary = dispatch_outbox_at(&conn, &group_key, &store_a, "device-a", t0).await.unwrap();
        assert_eq!(summary, OutboxDispatchSummary { delivered: 1, ..Default::default() });
        assert_eq!(states(&store_a), vec!["delivered"]);
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_MEETING).await, 1);

        let store_b = Store::open_in_memory().unwrap();
        pull_content(&conn, &group_key, &store_b, "device-b").await.unwrap();
        let local_seq = store_a.list_outbox_operations().unwrap()[0].0;
        let applied_id = store_b.resolve_sync_row_map("device-a", local_seq, ROW_TYPE_MEETING).unwrap().unwrap();
        let applied = store_b.get_meeting(applied_id).unwrap().unwrap();
        assert_eq!(applied.title, "Outbox Standup");
        assert_eq!(applied.status, memory::MeetingStatus::Completed);

        // A second round has nothing left to claim.
        let again = dispatch_outbox_at(&conn, &group_key, &store_a, "device-a", t0 + 1).await.unwrap();
        assert_eq!(again, OutboxDispatchSummary::default());
    }

    /// The invariant the dual-write depends on: the outbox snapshot and the legacy
    /// `encode_row` produce the same JSON for the same stamp, so whichever path wins
    /// the `ON CONFLICT DO NOTHING` race, the peer decodes the same meeting.
    #[tokio::test]
    async fn outbox_payload_equals_the_legacy_push_encoding() {
        // libsql's local backend must run `sqlite3_config` before any rusqlite
        // connection initialises SQLite in this process — every test here opens the
        // relay first for that reason, even one that never writes to it.
        let dir = tempfile::tempdir().unwrap();
        let _conn = local_connection(&dir.path().join("relay.db")).await;
        let (store, meeting_id) = own_store("device-a");
        store.complete_meeting(meeting_id, 2000, None).unwrap();
        store
            .update_meeting_summary(meeting_id, "tldr", &["d".into()], &["a".into()], &["q".into()], "ollama", "m")
            .unwrap();

        let legacy = store.unpushed_meeting_rows("device-a", 0).unwrap();
        assert_eq!(legacy.len(), 1);
        let (legacy_seq, legacy_type, legacy_json) = crate::content::encode_row(&legacy[0]).unwrap();
        let (outbox_seq, outbox_type, _, outbox_json) =
            store.list_outbox_operations().unwrap().pop().unwrap();
        assert_eq!((legacy_seq, legacy_type), (outbox_seq, outbox_type.as_str()));
        let legacy_value: serde_json::Value = serde_json::from_str(&legacy_json).unwrap();
        let outbox_value: serde_json::Value = serde_json::from_str(&outbox_json).unwrap();
        assert_eq!(outbox_value, legacy_value);
    }

    #[tokio::test]
    async fn both_push_paths_name_a_meeting_with_the_same_identity() {
        let dir = tempfile::tempdir().unwrap();
        let _conn = local_connection(&dir.path().join("relay.db")).await;
        let (store, meeting_id) = own_store("device-a");
        store.complete_meeting(meeting_id, 2000, None).unwrap();
        let legacy = store.unpushed_meeting_rows("device-a", 0).unwrap();
        let op = store
            .claim_due_outbox_batch(memory::OUTBOX_CONNECTOR, "w", t0(), t0() + LEASE_MS, 10)
            .unwrap()
            .pop()
            .unwrap();
        let expected = Some(format!("meeting:{meeting_id}"));
        assert_eq!(crate::content::entity_identity(&legacy[0]), expected);
        assert_eq!(crate::content::outbox_entity_identity(&op), expected);
    }

    #[tokio::test]
    async fn transcript_items_are_held_until_the_parent_meeting_is_terminal() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("relay.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        let group_key = GroupKey::generate();
        let (store_a, meeting_id) = own_store("device-a");
        let segment = |start_cs, text: &str| NewTranscriptSegment {
            meeting_id,
            start_cs,
            speaker_label: "Me".into(),
            text: text.into(),
        };
        store_a.insert_transcript_segments(&[segment(0, "first"), segment(100, "second")]).unwrap();
        let t0 = t0();

        let held = dispatch_outbox_at(&conn, &group_key, &store_a, "device-a", t0).await.unwrap();
        assert_eq!(held, OutboxDispatchSummary { deferred: 2, ..Default::default() });
        assert_eq!(states(&store_a), vec!["pending", "pending"]);
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_ITEMS).await, 0);

        store_a.complete_meeting(meeting_id, 2000, None).unwrap();
        let released = dispatch_outbox_at(&conn, &group_key, &store_a, "device-a", t0 + DEFERRED_RECHECK_MS)
            .await
            .unwrap();
        assert_eq!(released, OutboxDispatchSummary { delivered: 3, ..Default::default() });
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_ITEMS).await, 2);
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_MEETING).await, 1);

        let store_b = Store::open_in_memory().unwrap();
        pull_content(&conn, &group_key, &store_b, "device-b").await.unwrap();
        let hits = store_b.search_transcript_keyword("second", 10).unwrap();
        assert_eq!(hits.len(), 1, "peer applied the transcript items under the pulled meeting");
    }

    /// A pre-cutover peer gets transcripts from the legacy scan in the old format; the
    /// outbox must not later deliver the same rows as `items` (two row types, one
    /// segment), so they are acknowledged as legacy-owned rather than deferred.
    #[tokio::test]
    async fn transcript_items_are_left_to_the_legacy_push_while_a_peer_is_not_items_capable() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("relay.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        let group_key = GroupKey::generate();
        // A peer with content on the relay but no items_capability marker.
        conn.execute(
            "INSERT INTO sync_row (device_id, local_seq, row_type, ciphertext, created_at_unix_ms) VALUES ('device-old', 1, 'meeting', 'x', 0)",
            (),
        )
        .await
        .unwrap();
        let (store_a, meeting_id) = own_store("device-a");
        store_a
            .insert_transcript_segments(&[NewTranscriptSegment {
                meeting_id,
                start_cs: 0,
                speaker_label: "Me".into(),
                text: "hello".into(),
            }])
            .unwrap();
        store_a.complete_meeting(meeting_id, 2000, None).unwrap();

        let summary = dispatch_outbox_at(&conn, &group_key, &store_a, "device-a", t0()).await.unwrap();
        assert_eq!(summary, OutboxDispatchSummary { delivered: 1, legacy_owned: 1, ..Default::default() });
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_MEETING).await, 1);
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_ITEMS).await, 0);
        assert_eq!(states(&store_a), vec!["delivered", "delivered"], "the items row is closed out, never re-dispatched as items");
    }

    /// SYNC-104 end to end: a relay write failure returns the row to pending with
    /// backoff, it is not reclaimed inside the backoff window, and it delivers once the
    /// window passes and the relay is healthy again.
    #[tokio::test]
    async fn a_failed_relay_write_backs_off_and_then_delivers() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("relay.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        let group_key = GroupKey::generate();
        let (store_a, meeting_id) = own_store("device-a");
        store_a.complete_meeting(meeting_id, 2000, None).unwrap();
        let t0 = t0();

        // Relay rejects every write: the INSERT fails, the local claim/release side is
        // unaffected.
        conn.execute(
            "CREATE TRIGGER relay_down BEFORE INSERT ON sync_row BEGIN SELECT RAISE(ABORT, 'relay down'); END",
            (),
        )
        .await
        .unwrap();
        let failed = dispatch_outbox_at(&conn, &group_key, &store_a, "device-a", t0).await.unwrap();
        assert_eq!(failed, OutboxDispatchSummary { failed: 1, ..Default::default() });
        assert_eq!(states(&store_a), vec!["pending"]);

        conn.execute("DROP TRIGGER relay_down", ()).await.unwrap();
        let too_early = dispatch_outbox_at(&conn, &group_key, &store_a, "device-a", t0 + BASE_BACKOFF_MS - 1)
            .await
            .unwrap();
        assert_eq!(too_early, OutboxDispatchSummary::default());

        let retried = dispatch_outbox_at(&conn, &group_key, &store_a, "device-a", t0 + BASE_BACKOFF_MS)
            .await
            .unwrap();
        assert_eq!(retried, OutboxDispatchSummary { delivered: 1, ..Default::default() });
        assert_eq!(states(&store_a), vec!["delivered"]);
    }

    #[test]
    fn backoff_doubles_from_thirty_seconds_and_caps_at_an_hour() {
        assert_eq!(backoff_ms(1), 30_000);
        assert_eq!(backoff_ms(2), 60_000);
        assert_eq!(backoff_ms(3), 120_000);
        assert_eq!(backoff_ms(40), MAX_BACKOFF_MS);
    }

    /// A finished routine run reaches a peer through the real push path; a pending approval never
    /// leaves the device.
    #[tokio::test]
    async fn dispatch_delivers_a_finished_routine_run_and_never_a_pending_one() {
        use crate::content::ROW_TYPE_ROUTINE_RUN;
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("relay.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        let group_key = GroupKey::generate();
        let (store_a, _) = own_store("device-a");
        store_a.seed_builtin_routines().unwrap();
        let routines = store_a.list_routines().unwrap();
        let brief = routines.iter().find(|r| r.template == memory::RoutineTemplate::DailyBrief).unwrap();
        let focus = routines.iter().find(|r| r.template == memory::RoutineTemplate::FocusTime).unwrap();
        store_a
            .insert_routine_run(&memory::NewRoutineRun {
                routine_id: brief.id,
                ran_at_unix_ms: 7_000,
                status: memory::RunStatus::Success,
                output_summary: "Daily brief from A".into(),
                output_json: "{}".into(),
            })
            .unwrap();
        store_a
            .insert_pending_routine_run(&memory::NewPendingRoutineRun {
                routine_id: focus.id,
                ran_at_unix_ms: 7_100,
                output_summary: "draft".into(),
                pending_action_json: "{}".into(),
            })
            .unwrap();

        dispatch_outbox_at(&conn, &group_key, &store_a, "device-a", t0()).await.unwrap();
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_ROUTINE_RUN).await, 1, "only the finished run is delivered");

        let store_b = Store::open_in_memory().unwrap();
        pull_content(&conn, &group_key, &store_b, "device-b").await.unwrap();
        let runs = store_b.recent_routine_runs_across_all(10).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].output_summary, "Daily brief from A");
        assert_eq!(runs[0].origin_device_id.as_deref(), Some("device-a"));
        assert!(store_b.pending_routine_runs().unwrap().is_empty());
    }

    #[tokio::test]
    async fn dispatch_delivers_a_saved_meeting_prep_and_a_peer_applies_it() {
        use crate::content::ROW_TYPE_MEETING_PREP;
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("relay.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        let group_key = GroupKey::generate();
        let (store_a, _) = own_store("device-a");
        store_a
            .save_meeting_prep(&memory::prep::MeetingPrepRow {
                event_id: "ev1".into(), title: "Sync".into(), start_unix_ms: 2_000_000, generated_at_unix_ms: 9,
                brief_json: "{}".into(), provider: "p".into(), model: "m".into(),
            })
            .unwrap();
        dispatch_outbox_at(&conn, &group_key, &store_a, "device-a", t0()).await.unwrap();
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_MEETING_PREP).await, 1);

        let store_b = Store::open_in_memory().unwrap();
        pull_content(&conn, &group_key, &store_b, "device-b").await.unwrap();
        assert_eq!(store_b.get_meeting_prep("ev1").unwrap().unwrap().generated_at_unix_ms, 9);
    }

    #[tokio::test]
    async fn dispatch_delivers_a_profile_suggestion_and_a_peer_applies_it() {
        use crate::content::ROW_TYPE_PROFILE_SUGGESTION;
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("relay.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        let group_key = GroupKey::generate();
        let (store_a, _) = own_store("device-a");
        store_a.add_profile_suggestion("Prefers mornings", None, 9).unwrap();
        dispatch_outbox_at(&conn, &group_key, &store_a, "device-a", t0()).await.unwrap();
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_PROFILE_SUGGESTION).await, 1);

        let store_b = Store::open_in_memory().unwrap();
        pull_content(&conn, &group_key, &store_b, "device-b").await.unwrap();
        assert_eq!(store_b.list_profile_suggestions("pending").unwrap().len(), 1);

        // Dismissed on A: B's copy is dismissed at its next pull.
        let id = store_a.list_profile_suggestions("pending").unwrap()[0].id;
        store_a.set_profile_suggestion_status(id, "dismissed").unwrap();
        dispatch_outbox_at(&conn, &group_key, &store_a, "device-a", t0()).await.unwrap();
        pull_content(&conn, &group_key, &store_b, "device-b").await.unwrap();
        assert!(store_b.list_profile_suggestions("pending").unwrap().is_empty());
        assert_eq!(store_b.list_profile_suggestions("dismissed").unwrap().len(), 1);
    }

    #[tokio::test]
    async fn live_alert_round_trips_between_two_stores() {
        use crate::content::ROW_TYPE_LIVE_ALERT;
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("relay.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        let group_key = GroupKey::generate();
        let (store_a, meeting_id) = own_store("device-a");
        let now = crate::now_unix_ms();
        let id = store_a
            .insert_live_alert(&memory::live_alert::NewLiveAlert {
                meeting_id,
                meeting_title: "Outbox Standup".into(),
                quote: "Mike, can you send the numbers?".into(),
                restatement: "Send the pricing numbers".into(),
                asker: Some("Dana".into()),
                at_cs: 4_200,
                created_at_unix_ms: now,
            })
            .unwrap();
        dispatch_outbox_at(&conn, &group_key, &store_a, "device-a", t0()).await.unwrap();
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_LIVE_ALERT).await, 1);

        let store_b = Store::open_in_memory().unwrap();
        pull_content(&conn, &group_key, &store_b, "device-b").await.unwrap();
        let taken = store_b.take_unnotified_peer_alerts(now, 1_800_000).unwrap();
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].meeting_title, "Outbox Standup");
        assert_eq!(taken[0].meeting_id, None, "the meeting has not synced yet");
        assert_eq!((taken[0].source_id, taken[0].source_meeting_id), (id, meeting_id));

        store_a.set_live_alert_draft(id, r#"{"body":"Numbers attached"}"#).unwrap();
        dispatch_outbox_at(&conn, &group_key, &store_a, "device-a", t0()).await.unwrap();
        pull_content(&conn, &group_key, &store_b, "device-b").await.unwrap();
        let row = store_b.get_live_alert(taken[0].id).unwrap().unwrap();
        assert_eq!(row.draft_json.as_deref(), Some(r#"{"body":"Numbers attached"}"#));
        assert!(store_b.take_unnotified_peer_alerts(now, 1_800_000).unwrap().is_empty(), "an update does not re-notify");
    }

    fn has_line(store: &Store, word: &str) -> bool {
        !store.search_transcript_keyword(word, 10).unwrap().is_empty()
    }

    /// Meeting 1_000-121_000 with lines at 0 s ("opening") and 90 s ("tangent"), all pushed.
    async fn pushed_meeting_with_two_lines(conn: &Connection, group_key: &GroupKey) -> (Store, i64) {
        let (store_a, meeting_id) = own_store("device-a");
        let line = |start_cs, text: &str| NewTranscriptSegment {
            meeting_id,
            start_cs,
            speaker_label: "Me".into(),
            text: text.into(),
        };
        store_a.insert_transcript_segments(&[line(0, "opening"), line(9_000, "tangent")]).unwrap();
        store_a.complete_meeting(meeting_id, 121_000, None).unwrap();
        dispatch_outbox_at(conn, group_key, &store_a, "device-a", t0()).await.unwrap();
        (store_a, meeting_id)
    }

    #[tokio::test]
    async fn peer_drops_segments_past_trim_both_orders() {
        let (_dir, conn, group_key) = relay_and_store().await;
        let (store_a, meeting_id) = pushed_meeting_with_two_lines(&conn, &group_key).await;

        // B pulls the lines before the trim; C only after it.
        let store_b = Store::open_in_memory().unwrap();
        pull_content(&conn, &group_key, &store_b, "device-b").await.unwrap();
        assert!(has_line(&store_b, "opening") && has_line(&store_b, "tangent"));

        store_a.trim_meeting(meeting_id, 60_000).unwrap().unwrap();
        dispatch_outbox_at(&conn, &group_key, &store_a, "device-a", t0()).await.unwrap();
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_ITEMS).await, 2, "no tombstone: the relay keeps the late line");

        pull_content(&conn, &group_key, &store_b, "device-b").await.unwrap();
        let store_c = Store::open_in_memory().unwrap();
        pull_content(&conn, &group_key, &store_c, "device-c").await.unwrap();
        for (name, peer) in [("B: lines then trim", &store_b), ("C: trim then lines", &store_c)] {
            assert!(has_line(peer, "opening"), "{name}");
            assert!(!has_line(peer, "tangent"), "{name}");
            let local = peer.resolve_sync_meeting_map("device-a", meeting_id).unwrap().unwrap();
            assert_eq!(peer.get_meeting(local).unwrap().unwrap().end_unix_ms, Some(61_000), "{name}");
        }
    }

    #[tokio::test]
    async fn untrimmed_meeting_keeps_late_segments() {
        let (_dir, conn, group_key) = relay_and_store().await;
        let (store_a, meeting_id) = own_store("device-a");
        store_a.complete_meeting(meeting_id, 2_000, None).unwrap();
        // Transcription finishes after the stop, with a line past the recorded end.
        store_a
            .insert_transcript_segments(&[NewTranscriptSegment {
                meeting_id,
                start_cs: 9_000,
                speaker_label: "Me".into(),
                text: "afterwards".into(),
            }])
            .unwrap();
        dispatch_outbox_at(&conn, &group_key, &store_a, "device-a", t0()).await.unwrap();
        let store_b = Store::open_in_memory().unwrap();
        pull_content(&conn, &group_key, &store_b, "device-b").await.unwrap();
        assert!(has_line(&store_b, "afterwards"));
    }

    /// A's completed meeting synced to B; returns B's store and B's id for it.
    async fn meeting_on_b(conn: &Connection, group_key: &GroupKey, store_a: &Store, a_meeting: i64) -> (Store, i64) {
        store_a.complete_meeting(a_meeting, 2000, None).unwrap();
        dispatch_outbox_at(conn, group_key, store_a, "device-a", t0()).await.unwrap();
        let store_b = Store::open_in_memory().unwrap();
        store_b.set_own_device_id("device-b");
        pull_content(conn, group_key, &store_b, "device-b").await.unwrap();
        let b_meeting = store_b.resolve_sync_meeting_map("device-a", a_meeting).unwrap().unwrap();
        (store_b, b_meeting)
    }

    /// Two devices' edits a clock tick apart, so the later one is strictly newer.
    fn tick() {
        std::thread::sleep(std::time::Duration::from_millis(2));
    }

    async fn relay_and_store() -> (tempfile::TempDir, Connection, GroupKey) {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("relay.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        (dir, conn, GroupKey::generate())
    }

    #[tokio::test]
    async fn person_round_trips_between_two_stores() {
        use crate::content::ROW_TYPE_PERSON;
        let (_dir, conn, group_key) = relay_and_store().await;
        let (store_a, a_meeting) = own_store("device-a");
        store_a.upsert_person("sam@x.com", "Sam", Some("sam@x.com")).unwrap();
        store_a.set_meeting_speaker(a_meeting, 0, Some("sam@x.com"), Some(memory::speaker_names::NameStatus::Confirmed)).unwrap();
        let (store_b, b_meeting) = meeting_on_b(&conn, &group_key, &store_a, a_meeting).await;
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_PERSON).await, 1);
        assert_eq!(store_b.meeting_speakers(b_meeting).unwrap()[0].name.as_deref(), Some("Sam"));

        tick();
        store_a.upsert_person("sam@x.com", "Sam Lee", Some("sam@x.com")).unwrap();
        dispatch_outbox_at(&conn, &group_key, &store_a, "device-a", t0()).await.unwrap();
        pull_content(&conn, &group_key, &store_b, "device-b").await.unwrap();
        assert_eq!(store_b.meeting_speakers(b_meeting).unwrap()[0].name.as_deref(), Some("Sam Lee"));
        assert!(
            store_b.list_outbox_operations().unwrap().iter().all(|o| o.1 != ROW_TYPE_PERSON),
            "an applied row is never re-enqueued"
        );
    }

    #[tokio::test]
    async fn provider_spend_round_trips_between_two_stores() {
        use crate::content::ROW_TYPE_PROVIDER_SPEND;
        let (_dir, conn, group_key) = relay_and_store().await;
        let (store_a, _) = own_store("device-a");
        let (store_b, _) = own_store("device-b");
        let log = |s: &Store, cents: i64| {
            s.log_usage(&memory::NewUsageLogEntry {
                user_id: None,
                provider: "anthropic".into(),
                model: "m".into(),
                route: None,
                input_tokens: None,
                output_tokens: None,
                estimated_cost_cents: cents,
                cache_hit: false,
                trace_id: "t".into(),
                ts_unix_ms: 1_794_744_000_000, // 2026-11-15
            })
            .unwrap();
        };
        log(&store_a, 300);
        log(&store_b, 50);
        dispatch_outbox_at(&conn, &group_key, &store_a, "device-a", t0()).await.unwrap();
        pull_content(&conn, &group_key, &store_b, "device-b").await.unwrap();
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_PROVIDER_SPEND).await, 1);
        assert_eq!(store_b.month_spend_by_provider("2026-11").unwrap(), vec![("anthropic".into(), 350)]);

        tick();
        log(&store_a, 100);
        dispatch_outbox_at(&conn, &group_key, &store_a, "device-a", t0()).await.unwrap();
        pull_content(&conn, &group_key, &store_b, "device-b").await.unwrap();
        assert_eq!(store_b.month_spend_by_provider("2026-11").unwrap(), vec![("anthropic".into(), 450)]);
    }

    /// A reinstalled device (new database, same Keychain device id) never lowers what its
    /// peers count for it, and gets its own month back from the relay.
    #[tokio::test]
    async fn reinstall_never_lowers_provider_spend() {
        let (_dir, conn, group_key) = relay_and_store().await;
        let (store_a, _) = own_store("device-a");
        let (store_b, _) = own_store("device-b");
        let log = |s: &Store, cents: i64| {
            s.log_usage(&memory::NewUsageLogEntry {
                user_id: None,
                provider: "anthropic".into(),
                model: "m".into(),
                route: None,
                input_tokens: None,
                output_tokens: None,
                estimated_cost_cents: cents,
                cache_hit: false,
                trace_id: "t".into(),
                ts_unix_ms: 1_794_744_000_000, // 2026-11-15
            })
            .unwrap();
        };
        let spend = |s: &Store| s.month_spend_by_provider("2026-11").unwrap();
        log(&store_a, 2000);
        dispatch_outbox_at(&conn, &group_key, &store_a, "device-a", t0()).await.unwrap();
        pull_content(&conn, &group_key, &store_b, "device-b").await.unwrap();
        assert_eq!(spend(&store_b), vec![("anthropic".into(), 2000)]);

        drop(store_a);
        tick();
        let (store_a, _) = own_store("device-a");
        log(&store_a, 50);
        dispatch_outbox_at(&conn, &group_key, &store_a, "device-a", t0()).await.unwrap();
        pull_content(&conn, &group_key, &store_a, "device-a").await.unwrap();
        pull_content(&conn, &group_key, &store_b, "device-b").await.unwrap();
        assert!(spend(&store_b)[0].1 >= 2000, "B: {:?}", spend(&store_b));
        assert!(spend(&store_a)[0].1 >= 2050, "A: {:?}", spend(&store_a));
        // Pulling again doesn't count the restored month twice.
        pull_content(&conn, &group_key, &store_a, "device-a").await.unwrap();
        assert_eq!(spend(&store_a), vec![("anthropic".into(), 2050)]);
    }

    #[tokio::test]
    async fn meeting_speaker_round_trips_between_two_stores() {
        use crate::content::ROW_TYPE_MEETING_SPEAKER;
        use memory::speaker_names::NameStatus;
        let (_dir, conn, group_key) = relay_and_store().await;
        let (store_a, a_meeting) = own_store("device-a");
        store_a.upsert_person("sam@x.com", "Sam", Some("sam@x.com")).unwrap();
        store_a.save_meeting_voices(a_meeting, &[(0, vec![1.0, 0.0]), (1, vec![0.0, 1.0])], true).unwrap();
        store_a.set_meeting_speaker(a_meeting, 0, Some("sam@x.com"), Some(NameStatus::Confirmed)).unwrap();
        // The speaker rows reach B before their meeting does: they wait, then attach.
        dispatch_outbox_at(&conn, &group_key, &store_a, "device-a", t0()).await.unwrap();
        let early = Store::open_in_memory().unwrap();
        pull_content(&conn, &group_key, &early, "device-b").await.unwrap();
        assert!(early.resolve_sync_meeting_map("device-a", a_meeting).unwrap().is_none(), "the meeting is still recording");
        let (store_b, b_meeting) = meeting_on_b(&conn, &group_key, &store_a, a_meeting).await;
        pull_content(&conn, &group_key, &early, "device-b").await.unwrap();
        assert!(relay_rows(&conn, "device-a", ROW_TYPE_MEETING_SPEAKER).await >= 1);
        for store in [&early, &store_b] {
            let b_id = store.resolve_sync_meeting_map("device-a", a_meeting).unwrap().unwrap();
            let speakers = store.meeting_speakers(b_id).unwrap();
            assert_eq!(speakers.len(), 2);
            assert_eq!((speakers[0].name.as_deref(), speakers[0].status), (Some("Sam"), Some(NameStatus::Confirmed)));
            assert_eq!(speakers[1].person_key, None);
            assert_eq!(store.voice_print("sam@x.com").unwrap(), Some(vec![1.0, 0.0]), "the embedding came along");
        }
        assert_eq!(b_meeting, store_b.resolve_sync_meeting_map("device-a", a_meeting).unwrap().unwrap());
    }

    #[tokio::test]
    async fn meeting_speaker_rename_converges() {
        use memory::speaker_names::NameStatus;
        let (_dir, conn, group_key) = relay_and_store().await;
        let (store_a, a_meeting) = own_store("device-a");
        let (store_b, b_meeting) = meeting_on_b(&conn, &group_key, &store_a, a_meeting).await;
        store_a.upsert_person("ana", "Ana", None).unwrap();
        store_a.set_meeting_speaker(a_meeting, 0, Some("ana"), Some(NameStatus::Guess)).unwrap();
        tick();
        store_b.upsert_person("bo", "Bo", None).unwrap();
        store_b.set_meeting_speaker(b_meeting, 0, Some("bo"), Some(NameStatus::Confirmed)).unwrap();

        dispatch_outbox_at(&conn, &group_key, &store_a, "device-a", t0()).await.unwrap();
        dispatch_outbox_at(&conn, &group_key, &store_b, "device-b", t0()).await.unwrap();
        pull_content(&conn, &group_key, &store_a, "device-a").await.unwrap();
        pull_content(&conn, &group_key, &store_b, "device-b").await.unwrap();
        for (store, id) in [(&store_a, a_meeting), (&store_b, b_meeting)] {
            let s = &store.meeting_speakers(id).unwrap()[0];
            assert_eq!((s.name.as_deref(), s.status), (Some("Bo"), Some(NameStatus::Confirmed)), "B's later edit wins");
        }
    }

    #[tokio::test]
    async fn meeting_speaker_edit_on_non_recording_device_reaches_origin() {
        use crate::content::ROW_TYPE_MEETING_SPEAKER;
        use memory::speaker_names::{AppliedMeetingSpeaker, NameStatus};
        let (_dir, conn, group_key) = relay_and_store().await;
        let (store_a, a_meeting) = own_store("device-a");
        store_a.save_meeting_voices(a_meeting, &[(0, vec![1.0, 0.0])], true).unwrap();
        let (store_b, b_meeting) = meeting_on_b(&conn, &group_key, &store_a, a_meeting).await;
        tick();
        store_b.upsert_person("sam@x.com", "Sam", Some("sam@x.com")).unwrap();
        store_b.set_meeting_speaker(b_meeting, 0, Some("sam@x.com"), Some(NameStatus::Confirmed)).unwrap();
        let ops = store_b.list_outbox_operations().unwrap();
        let sent: AppliedMeetingSpeaker =
            serde_json::from_str(&ops.iter().rev().find(|o| o.1 == ROW_TYPE_MEETING_SPEAKER).unwrap().3).unwrap();
        assert_eq!((sent.origin_device_id.as_deref(), sent.source_meeting_id), (Some("device-a"), a_meeting));

        dispatch_outbox_at(&conn, &group_key, &store_b, "device-b", t0()).await.unwrap();
        pull_content(&conn, &group_key, &store_a, "device-a").await.unwrap();
        let s = &store_a.meeting_speakers(a_meeting).unwrap()[0];
        assert_eq!((s.name.as_deref(), s.status), (Some("Sam"), Some(NameStatus::Confirmed)));
        assert_eq!(store_a.voice_print("sam@x.com").unwrap(), Some(vec![1.0, 0.0]), "A's embedding is B's row's too");
    }

    #[tokio::test]
    async fn forget_voice_syncs() {
        use memory::speaker_names::NameStatus;
        let (_dir, conn, group_key) = relay_and_store().await;
        let (store_a, a_meeting) = own_store("device-a");
        store_a.upsert_person("sam@x.com", "Sam", Some("sam@x.com")).unwrap();
        store_a.save_meeting_voices(a_meeting, &[(0, vec![1.0, 0.0])], true).unwrap();
        store_a.set_meeting_speaker(a_meeting, 0, Some("sam@x.com"), Some(NameStatus::Confirmed)).unwrap();
        let (store_b, b_meeting) = meeting_on_b(&conn, &group_key, &store_a, a_meeting).await;
        assert!(store_b.voice_print("sam@x.com").unwrap().is_some());

        tick();
        store_a.forget_voice("sam@x.com").unwrap();
        dispatch_outbox_at(&conn, &group_key, &store_a, "device-a", t0()).await.unwrap();
        pull_content(&conn, &group_key, &store_b, "device-b").await.unwrap();
        assert_eq!(store_b.voice_print("sam@x.com").unwrap(), None, "B's embedding is gone");
        assert_eq!(store_b.meeting_speakers(b_meeting).unwrap()[0].name.as_deref(), Some("Sam"), "the name stays");
    }

    fn save_prep(store: &Store, event_id: &str, generated_at_unix_ms: i64) {
        store
            .save_meeting_prep(&memory::prep::MeetingPrepRow {
                event_id: event_id.into(), title: "Sync".into(), start_unix_ms: 2_000_000, generated_at_unix_ms,
                brief_json: "{}".into(), provider: "p".into(), model: "m".into(),
            })
            .unwrap();
    }

    async fn relay_seqs(conn: &Connection, row_type: &str) -> Vec<i64> {
        let mut rows = conn
            .query("SELECT local_seq FROM sync_row WHERE row_type = ?1 ORDER BY local_seq", params![row_type])
            .await
            .unwrap();
        let mut seqs = Vec::new();
        while let Some(row) = rows.next().await.unwrap() {
            seqs.push(row.get::<i64>(0).unwrap());
        }
        seqs
    }

    fn newest_op_seq(store: &Store, row_type: &str) -> i64 {
        store.list_outbox_operations().unwrap().into_iter().filter(|r| r.1 == row_type).map(|r| r.0).max().unwrap()
    }

    /// The legacy scan sends the newest meeting version first; the outbox delivers the
    /// older one late. Its own pass deletes it, because a higher `local_seq` is there.
    #[tokio::test]
    async fn a_late_older_outbox_version_is_deleted_and_the_newest_survives() {
        let dir = tempfile::tempdir().unwrap();
        let conn = crate::content::tests::compacting_relay(&dir).await;
        let group_key = GroupKey::generate();
        let (store_a, meeting_id) = own_store("device-a");
        store_a.complete_meeting(meeting_id, 2000, None).unwrap();
        store_a.set_meeting_title(meeting_id, "Renamed").unwrap();

        crate::content::push_content(&conn, &group_key, &store_a, "device-a").await.unwrap();
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_MEETING).await, 1, "the legacy scan pushes only the newest");
        dispatch_outbox_at(&conn, &group_key, &store_a, "device-a", t0()).await.unwrap();

        assert_eq!(relay_seqs(&conn, ROW_TYPE_MEETING).await, vec![newest_op_seq(&store_a, ROW_TYPE_MEETING)]);
    }

    #[tokio::test]
    async fn meeting_prep_resaves_compact_to_one_row() {
        use crate::content::ROW_TYPE_MEETING_PREP;
        let dir = tempfile::tempdir().unwrap();
        let conn = crate::content::tests::compacting_relay(&dir).await;
        let group_key = GroupKey::generate();
        let (store_a, _) = own_store("device-a");
        save_prep(&store_a, "ev1", 9);
        save_prep(&store_a, "ev1", 10);

        dispatch_outbox_at(&conn, &group_key, &store_a, "device-a", t0()).await.unwrap();
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_MEETING_PREP).await, 1);

        let store_b = Store::open_in_memory().unwrap();
        pull_content(&conn, &group_key, &store_b, "device-b").await.unwrap();
        assert_eq!(store_b.get_meeting_prep("ev1").unwrap().unwrap().generated_at_unix_ms, 10);
    }

    #[tokio::test]
    async fn outbox_deletes_run_once_per_fifty_tags() {
        let dir = tempfile::tempdir().unwrap();
        let conn = crate::content::tests::compacting_relay(&dir).await;
        let (store_a, _) = own_store("device-a");
        for i in 0..60 {
            save_prep(&store_a, &format!("ev{i}"), 9);
        }
        crate::content::take_compaction_deletes();

        let summary = dispatch_outbox_at(&conn, &GroupKey::generate(), &store_a, "device-a", t0()).await.unwrap();

        assert_eq!(summary.delivered, 60);
        assert_eq!(crate::content::take_compaction_deletes(), 2, "one at 50 tags, one at the end of the pass");
    }

    /// The outbox shares one 5 s compaction budget across its pass (the legacy push has its
    /// own, so up to 10 s per round): after the first
    /// DELETE times out, the second batch of tags is not compacted, and every op is
    /// still delivered. The clock is paused, so the injected 60 s delay costs no real time.
    #[tokio::test(start_paused = true)]
    async fn a_slow_compaction_spends_one_budget_per_outbox_pass() {
        let dir = tempfile::tempdir().unwrap();
        let conn = crate::content::tests::compacting_relay(&dir).await;
        let (store_a, _) = own_store("device-a");
        for i in 0..60 {
            save_prep(&store_a, &format!("ev{i}"), 9);
        }
        crate::content::take_compaction_deletes();
        crate::content::INJECTED_DELETE_DELAY.with(|d| d.set(Some(std::time::Duration::from_secs(60))));

        let started = tokio::time::Instant::now();
        let summary = dispatch_outbox_at(&conn, &GroupKey::generate(), &store_a, "device-a", t0()).await.unwrap();
        let spent = started.elapsed();

        assert!(spent <= std::time::Duration::from_secs(6), "{spent:?}");
        assert_eq!(crate::content::take_compaction_deletes(), 1, "after the timeout, the pass stops compacting");
        assert_eq!(summary.delivered, 60);
        assert_eq!(states(&store_a), vec!["delivered"; 60]);
    }

    #[tokio::test]
    async fn an_old_schema_relay_still_takes_outbox_deliveries() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("relay.db")).await;
        conn.execute(crate::content::tests::OLD_SYNC_ROW_DDL, ()).await.unwrap();
        // The account-switch case: this process saw the column on another relay, so it
        // still tries the tagged INSERT and must fall back.
        crate::content::ENTITY_TAG_COLUMN_SEEN.set();
        let (store_a, meeting_id) = own_store("device-a");
        store_a.complete_meeting(meeting_id, 2000, None).unwrap();
        store_a.set_meeting_title(meeting_id, "Renamed").unwrap();
        crate::content::take_compaction_deletes();

        let summary = dispatch_outbox_at(&conn, &GroupKey::generate(), &store_a, "device-a", t0()).await.unwrap();

        assert_eq!(summary, OutboxDispatchSummary { delivered: 2, ..Default::default() });
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_MEETING).await, 2);
        assert_eq!(crate::content::take_compaction_deletes(), 0);
    }

    #[tokio::test]
    async fn a_failing_compaction_delete_never_fails_the_dispatch() {
        let dir = tempfile::tempdir().unwrap();
        let conn = crate::content::tests::compacting_relay(&dir).await;
        conn.execute("CREATE TRIGGER no_delete BEFORE DELETE ON sync_row BEGIN SELECT RAISE(ABORT, 'nope'); END", ())
            .await
            .unwrap();
        let (store_a, meeting_id) = own_store("device-a");
        store_a.complete_meeting(meeting_id, 2000, None).unwrap();
        store_a.set_meeting_title(meeting_id, "Renamed").unwrap();
        crate::content::take_compaction_deletes();

        let summary = dispatch_outbox_at(&conn, &GroupKey::generate(), &store_a, "device-a", t0()).await.unwrap();

        assert_eq!(summary, OutboxDispatchSummary { delivered: 2, ..Default::default() });
        assert_eq!(states(&store_a), vec!["delivered", "delivered"]);
        assert_eq!(crate::content::take_compaction_deletes(), 1, "the DELETE was tried and failed");
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_MEETING).await, 2);
    }

    /// Rows are tagged without the ops index; only the DELETE waits for it.
    #[tokio::test]
    async fn no_outbox_compaction_without_the_index() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("relay.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        crate::content::COMPACTION_INDEX_SEEN.reset();
        let (store_a, meeting_id) = own_store("device-a");
        store_a.complete_meeting(meeting_id, 2000, None).unwrap();
        store_a.set_meeting_title(meeting_id, "Renamed").unwrap();
        crate::content::take_compaction_deletes();

        dispatch_outbox_at(&conn, &GroupKey::generate(), &store_a, "device-a", t0()).await.unwrap();

        assert_eq!(crate::content::take_compaction_deletes(), 0);
        let mut rows = conn
            .query("SELECT COUNT(*) FROM sync_row WHERE row_type = 'meeting' AND entity_tag IS NOT NULL", ())
            .await
            .unwrap();
        assert_eq!(rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(), 2, "both versions stay, tagged");
    }

    // ---- Deletion sync (docs/superpowers/specs/2026-11-05-deletion-sync-design.md) ----

    use crate::content::{delete_forgotten_meeting_rows, pull_content_without_skins, push_content, PullOptions};

    const ROW_TYPE_MEETING_FORGOTTEN: &str = memory::OUTBOX_ROW_TYPE_MEETING_FORGOTTEN;

    /// A's completed meeting with two transcript lines, a live alert and a speaker, all delivered.
    async fn delivered_meeting(conn: &Connection, group_key: &GroupKey) -> (Store, i64) {
        let (store_a, meeting_id) = own_store("device-a");
        let line = |start_cs, text: &str| NewTranscriptSegment { meeting_id, start_cs, speaker_label: "Me".into(), text: text.into() };
        store_a.insert_transcript_segments(&[line(0, "pelican"), line(500, "heron")]).unwrap();
        store_a
            .insert_live_alert(&memory::live_alert::NewLiveAlert {
                meeting_id,
                meeting_title: "Outbox Standup".into(),
                quote: "Can you send the deck?".into(),
                restatement: "Send the deck".into(),
                asker: None,
                at_cs: 100,
                created_at_unix_ms: crate::now_unix_ms(),
            })
            .unwrap();
        store_a.save_meeting_voices(meeting_id, &[(0, vec![1.0, 0.0])], true).unwrap();
        store_a.complete_meeting(meeting_id, 2000, None).unwrap();
        dispatch_outbox_at(conn, group_key, &store_a, "device-a", t0()).await.unwrap();
        (store_a, meeting_id)
    }

    fn peer(device_id: &str) -> Store {
        let store = Store::open_in_memory().unwrap();
        store.set_own_device_id(device_id);
        store
    }

    /// Whether `store` holds any trace of A's meeting `a_meeting`.
    fn has_a_meeting(store: &Store, a_meeting: i64) -> bool {
        store.resolve_sync_meeting_map("device-a", a_meeting).unwrap().is_some() || has_line(store, "pelican") || has_line(store, "heron")
    }

    /// #600: an outbox-only row that waits while the legacy push sends newer rows still reaches a puller whose
    /// cursor already passed the number it was enqueued with.
    #[tokio::test]
    async fn a_late_outbox_row_lands_above_a_pullers_cursor() {
        let (_dir, conn, group_key) = relay_and_store().await;
        let (store_a, meeting_id) = own_store("device-a");
        store_a
            .insert_live_alert(&memory::live_alert::NewLiveAlert {
                meeting_id,
                meeting_title: "Outbox Standup".into(),
                quote: "Can you send the deck?".into(),
                restatement: "Send the deck".into(),
                asker: None,
                at_cs: 100,
                created_at_unix_ms: crate::now_unix_ms(),
            })
            .unwrap();
        store_a.complete_meeting(meeting_id, 2000, None).unwrap();
        // The legacy scan sends the meeting (numbered after the alert) before the outbox runs.
        push_content(&conn, &group_key, &store_a, "device-a").await.unwrap();
        let store_c = peer("device-c");
        pull_content(&conn, &group_key, &store_c, "device-c").await.unwrap();
        assert!(store_c.resolve_sync_meeting_map("device-a", meeting_id).unwrap().is_some());

        dispatch_outbox_at(&conn, &group_key, &store_a, "device-a", t0()).await.unwrap();
        pull_content(&conn, &group_key, &store_c, "device-c").await.unwrap();
        assert_eq!(store_c.take_unnotified_peer_alerts(crate::now_unix_ms(), 1_800_000).unwrap().len(), 1, "the alert arrives");
    }

    #[tokio::test]
    async fn forget_on_recorder_removes_meeting_everywhere() {
        let (_dir, conn, group_key) = relay_and_store().await;
        let (store_a, a_meeting) = delivered_meeting(&conn, &group_key).await;
        let store_b = peer("device-b");
        pull_content(&conn, &group_key, &store_b, "device-b").await.unwrap();
        assert!(has_a_meeting(&store_b, a_meeting));
        assert!(relay_rows(&conn, "device-a", ROW_TYPE_ITEMS).await >= 2);
        assert_eq!(relay_rows(&conn, "device-a", crate::content::ROW_TYPE_LIVE_ALERT).await, 1);
        assert!(relay_rows(&conn, "device-a", crate::content::ROW_TYPE_MEETING_SPEAKER).await >= 1);

        store_a.forget_meeting(a_meeting).unwrap().unwrap();
        dispatch_outbox_at(&conn, &group_key, &store_a, "device-a", t0()).await.unwrap();
        delete_forgotten_meeting_rows(&conn, &group_key, &store_a, "device-a").await;

        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_MEETING).await, 0);
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_ITEMS).await, 0);
        assert_eq!(relay_rows(&conn, "device-a", crate::content::ROW_TYPE_LIVE_ALERT).await, 0, "the alert quotes the meeting");
        assert_eq!(relay_rows(&conn, "device-a", crate::content::ROW_TYPE_MEETING_SPEAKER).await, 0);
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_MEETING_FORGOTTEN).await, 1, "the tombstone stays");
        assert!(store_a.forgotten_pushed_meeting_rows(100).unwrap().is_empty(), "records cleared once deleted");

        crate::content::take_forgotten_meeting_files();
        pull_content(&conn, &group_key, &store_b, "device-b").await.unwrap();
        assert!(!has_a_meeting(&store_b, a_meeting), "B applied the tombstone");
        assert_eq!(crate::content::take_forgotten_meeting_files().len(), 1, "B's files go to the app to delete");
        assert!(store_b.take_unnotified_peer_alerts(crate::now_unix_ms(), 1_800_000).unwrap().is_empty());

        // A phone joining later (pull-only path) never sees it.
        let phone = Store::open_in_memory().unwrap();
        pull_content_without_skins(&conn, &group_key, &phone, "phone", &PullOptions::default()).await.unwrap();
        assert!(!has_a_meeting(&phone, a_meeting));
        assert!(phone.is_meeting_forgotten("device-a", None, a_meeting).unwrap());
    }

    #[tokio::test]
    async fn forget_on_non_recorder_reaches_recorder() {
        let (_dir, conn, group_key) = relay_and_store().await;
        let (store_a, a_meeting) = delivered_meeting(&conn, &group_key).await;
        let store_b = peer("device-b");
        pull_content(&conn, &group_key, &store_b, "device-b").await.unwrap();
        let b_meeting = store_b.resolve_sync_meeting_map("device-a", a_meeting).unwrap().unwrap();

        store_b.forget_meeting(b_meeting).unwrap().unwrap();
        dispatch_outbox_at(&conn, &group_key, &store_b, "device-b", t0()).await.unwrap();
        delete_forgotten_meeting_rows(&conn, &group_key, &store_b, "device-b").await;
        assert!(relay_rows(&conn, "device-a", ROW_TYPE_ITEMS).await >= 2, "B never deletes A's rows");

        pull_content(&conn, &group_key, &store_a, "device-a").await.unwrap();
        assert!(store_a.get_meeting(a_meeting).unwrap().is_none(), "A applied B's tombstone");
        delete_forgotten_meeting_rows(&conn, &group_key, &store_a, "device-a").await;
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_MEETING).await, 0);
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_ITEMS).await, 0);
        assert_eq!(relay_rows(&conn, "device-b", ROW_TYPE_MEETING_FORGOTTEN).await, 1);

        let store_c = peer("device-c");
        pull_content(&conn, &group_key, &store_c, "device-c").await.unwrap();
        assert!(!has_a_meeting(&store_c, a_meeting));
    }

    /// Final pass 1: B holds A's meeting from before epochs were sent (map epoch ''), so B's tombstone has no
    /// epoch. A takes it by id, device and start, and its server rows go; a copy with another start doesn't.
    #[tokio::test]
    async fn forgetting_a_pre_release_meeting_reaches_the_recorder() {
        for (start_shift, reaches) in [(0, true), (1, false)] {
            let (_dir, conn, group_key) = relay_and_store().await;
            let (store_a, a_meeting) = delivered_meeting(&conn, &group_key).await;
            let a_start = store_a.get_meeting(a_meeting).unwrap().unwrap().start_unix_ms;
            let store_b = peer("device-b");
            let copy = store_b
                .insert_meeting(&NewMeeting {
                    device_id: "device-a".into(),
                    start_unix_ms: a_start + start_shift,
                    end_unix_ms: None,
                    title: "Old".into(),
                    attendees: vec![],
                    trigger: MeetingTrigger::Manual,
                    audio_path: None,
                    status: memory::MeetingStatus::Completed,
                })
                .unwrap();
            store_b.record_sync_meeting_map("device-a", a_meeting, copy).unwrap();

            store_b.forget_meeting(copy).unwrap().unwrap();
            dispatch_outbox_at(&conn, &group_key, &store_b, "device-b", t0()).await.unwrap();
            pull_content(&conn, &group_key, &store_a, "device-a").await.unwrap();
            delete_forgotten_meeting_rows(&conn, &group_key, &store_a, "device-a").await;

            assert_eq!(store_a.get_meeting(a_meeting).unwrap().is_none(), reaches, "shift {start_shift}");
            assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_MEETING).await == 0, reaches, "shift {start_shift}");
            assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_ITEMS).await == 0, reaches, "shift {start_shift}");
        }
    }

    #[tokio::test]
    async fn late_rows_for_forgotten_meeting_dropped() {
        let (_dir, conn, group_key) = relay_and_store().await;
        let (store_a, a_meeting) = delivered_meeting(&conn, &group_key).await;
        let forgotten = memory::MeetingForgotten {
            origin_device_id: "device-a".into(),
            origin_epoch: Some(store_a.sync_entity_epoch().unwrap()),
            source_meeting_id: a_meeting,
            forgotten_at_unix_ms: 1,
            source_start_unix_ms: None,
        };

        // The tombstone first, then every row of the meeting: all dropped, nothing left waiting.
        let store_c = peer("device-c");
        store_c.apply_meeting_forgotten(&forgotten).unwrap();
        pull_content(&conn, &group_key, &store_c, "device-c").await.unwrap();
        assert!(!has_a_meeting(&store_c, a_meeting));
        assert!(store_c.list_sync_pending_dependencies("device-a").unwrap().is_empty());
        assert!(store_c.take_unnotified_peer_alerts(crate::now_unix_ms(), 1_800_000).unwrap().is_empty(), "the alert is dropped");

        // Lines that arrived before their meeting wait for it; the tombstone ends the wait, both while the
        // lines are still on the relay and after the recorder deleted them.
        conn.execute("DELETE FROM sync_row WHERE device_id = 'device-a' AND row_type = 'meeting'", ()).await.unwrap();
        for clean_up_first in [false, true] {
            let store_d = peer("device-d");
            pull_content(&conn, &group_key, &store_d, "device-d").await.unwrap();
            assert_eq!(store_d.list_sync_pending_dependencies("device-a").unwrap().len(), 2, "lines wait for the meeting");
            let waiting_alert = store_d.take_unnotified_peer_alerts(crate::now_unix_ms(), 1_800_000).unwrap();
            assert_eq!(waiting_alert.len(), 1, "the alert waits for its meeting");
            store_d.apply_meeting_forgotten(&forgotten).unwrap();
            assert!(store_d.get_live_alert(waiting_alert[0].id).unwrap().is_none(), "the tombstone removes it");
            if clean_up_first {
                store_a.forget_meeting(a_meeting).unwrap();
                dispatch_outbox_at(&conn, &group_key, &store_a, "device-a", t0()).await.unwrap();
                delete_forgotten_meeting_rows(&conn, &group_key, &store_a, "device-a").await;
                assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_ITEMS).await, 0);
            }
            pull_content(&conn, &group_key, &store_d, "device-d").await.unwrap();
            assert!(store_d.list_sync_pending_dependencies("device-a").unwrap().is_empty(), "clean_up_first: {clean_up_first}");
            assert!(!has_a_meeting(&store_d, a_meeting));
        }
    }

    #[tokio::test]
    async fn an_earlier_databases_forget_never_drops_a_reused_ids_rows() {
        let (_dir, conn, group_key) = relay_and_store().await;
        let (_store_a, a_meeting) = delivered_meeting(&conn, &group_key).await;
        // A forget of the same id from a database device-a had before (another epoch).
        let old = memory::MeetingForgotten {
            origin_device_id: "device-a".into(),
            origin_epoch: Some("an-earlier-database".into()),
            source_meeting_id: a_meeting,
            forgotten_at_unix_ms: 1,
            source_start_unix_ms: None,
        };
        // The lines and alert arrive before their meeting: they carry their epoch, so they wait, not drop.
        conn.execute("DELETE FROM sync_row WHERE device_id = 'device-a' AND row_type = 'meeting'", ()).await.unwrap();
        let store_c = peer("device-c");
        store_c.apply_meeting_forgotten(&old).unwrap();
        pull_content(&conn, &group_key, &store_c, "device-c").await.unwrap();
        assert_eq!(store_c.list_sync_pending_dependencies("device-a").unwrap().len(), 2, "lines wait for the meeting");
        assert_eq!(store_c.take_unnotified_peer_alerts(crate::now_unix_ms(), 1_800_000).unwrap().len(), 1, "the alert waits");
    }

    #[tokio::test]
    async fn recorder_deletes_its_server_rows_by_key() {
        let (_dir, conn, group_key) = relay_and_store().await;
        // B has rows of its own, and A a second meeting: neither is touched.
        let (store_b, b_meeting) = own_store("device-b");
        store_b.complete_meeting(b_meeting, 2000, None).unwrap();
        push_content(&conn, &group_key, &store_b, "device-b").await.unwrap();
        let (store_a, kept) = own_store("device-a");
        let line = |meeting_id, text: &str| NewTranscriptSegment { meeting_id, start_cs: 0, speaker_label: "Me".into(), text: text.into() };
        store_a.insert_transcript_segments(&[line(kept, "kept")]).unwrap();
        store_a.complete_meeting(kept, 2000, None).unwrap();
        let gone = store_a
            .insert_meeting(&NewMeeting {
                device_id: "device-a".into(),
                start_unix_ms: 5000,
                end_unix_ms: None,
                title: "Gone".into(),
                attendees: vec![],
                trigger: MeetingTrigger::Manual,
                audio_path: None,
                status: memory::MeetingStatus::Recording,
            })
            .unwrap();
        store_a.insert_transcript_segments(&[line(gone, "gone")]).unwrap();
        store_a.complete_meeting(gone, 6000, None).unwrap();
        // The legacy scan pushes A's rows, so only what it recorded names them for the cleanup.
        push_content(&conn, &group_key, &store_a, "device-a").await.unwrap();
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_ITEMS).await, 2);

        store_a.forget_meeting(gone).unwrap().unwrap();
        delete_forgotten_meeting_rows(&conn, &group_key, &store_a, "device-a").await;

        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_MEETING).await, 1, "only the kept meeting's row");
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_ITEMS).await, 1);
        assert_eq!(relay_rows(&conn, "device-b", ROW_TYPE_MEETING).await, 1, "another device's rows stay");
        let store_c = peer("device-c");
        pull_content(&conn, &group_key, &store_c, "device-c").await.unwrap();
        assert!(has_line(&store_c, "kept") && !has_line(&store_c, "gone"));

        // A delivery in flight at the forget (leased, acknowledged after it) is recorded and cleaned up too.
        let (store_e, late) = own_store("device-e");
        store_e.insert_transcript_segments(&[line(late, "late")]).unwrap();
        store_e.complete_meeting(late, 2000, None).unwrap();
        let now = t0();
        store_e.claim_due_outbox_batch(memory::OUTBOX_CONNECTOR, "crashed", now, now + LEASE_MS, 100).unwrap();
        store_e.forget_meeting(late).unwrap().unwrap();
        let summary = dispatch_outbox_at(&conn, &group_key, &store_e, "device-e", now + LEASE_MS + 1).await.unwrap();
        assert_eq!(summary.dropped, 1, "its line has no meeting to go with: acknowledged unsent");
        assert_eq!(relay_rows(&conn, "device-e", ROW_TYPE_ITEMS).await, 0);
        assert_eq!(relay_rows(&conn, "device-e", ROW_TYPE_MEETING).await, 1, "the leased row still went out");
        delete_forgotten_meeting_rows(&conn, &group_key, &store_e, "device-e").await;
        assert_eq!(relay_rows(&conn, "device-e", ROW_TYPE_MEETING).await, 0);
        assert!(states(&store_e).iter().all(|s| s == "delivered"), "{:?}", states(&store_e));
    }

    /// A row leased at the forget and acknowledged only after a cleanup round already finished (tag deleted,
    /// nothing recorded yet), with other meeting deletes in between: the next cleanup still deletes it.
    #[tokio::test]
    async fn a_late_acknowledged_row_is_cleaned_up_after_the_cleanup_finished() {
        let (_dir, conn, group_key) = relay_and_store().await;
        let (store_e, late) = own_store("device-e");
        let line = |meeting_id, text: &str| NewTranscriptSegment { meeting_id, start_cs: 0, speaker_label: "Me".into(), text: text.into() };
        store_e.insert_transcript_segments(&[line(late, "late")]).unwrap();
        store_e.complete_meeting(late, 2000, None).unwrap();
        let other = store_e
            .insert_meeting(&NewMeeting {
                device_id: "device-e".into(),
                start_unix_ms: 3000,
                end_unix_ms: None,
                title: "Other".into(),
                attendees: vec![],
                trigger: MeetingTrigger::Manual,
                audio_path: None,
                status: memory::MeetingStatus::Completed,
            })
            .unwrap();
        let now = t0();
        store_e.claim_due_outbox_batch(memory::OUTBOX_CONNECTOR, "crashed", now, now + LEASE_MS, 100).unwrap();
        store_e.forget_meeting(late).unwrap().unwrap();
        delete_forgotten_meeting_rows(&conn, &group_key, &store_e, "device-e").await;
        store_e.mark_meeting_tag_deleted(late).unwrap(); // as on a relay with entity tags
        assert!(store_e.forgotten_pushed_meeting_rows(10).unwrap().is_empty(), "this round's work is done");
        store_e.delete_meeting(other).unwrap(); // a delete path that prunes its own records

        dispatch_outbox_at(&conn, &group_key, &store_e, "device-e", now + LEASE_MS + 1).await.unwrap();
        assert_eq!(relay_rows(&conn, "device-e", ROW_TYPE_MEETING).await, 1, "the leased row went out late");
        delete_forgotten_meeting_rows(&conn, &group_key, &store_e, "device-e").await;
        assert_eq!(relay_rows(&conn, "device-e", ROW_TYPE_MEETING).await, 0);
    }

    /// A re-created database (new epoch, same device id, ids restarting) gets an old tombstone that doesn't
    /// know its epoch for a reused meeting id: the new meeting and its server rows stay.
    #[tokio::test]
    async fn an_unknown_epoch_tombstone_never_reaches_the_recorders_new_meeting() {
        let (_dir, conn, group_key) = relay_and_store().await;
        let (store_a, a_meeting) = delivered_meeting(&conn, &group_key).await;
        let old = memory::MeetingForgotten {
            origin_device_id: "device-a".into(),
            origin_epoch: None,
            source_meeting_id: a_meeting,
            forgotten_at_unix_ms: 1,
            source_start_unix_ms: None,
        };
        assert!(store_a.apply_meeting_forgotten(&old).unwrap().is_none());
        delete_forgotten_meeting_rows(&conn, &group_key, &store_a, "device-a").await;
        assert!(store_a.get_meeting(a_meeting).unwrap().is_some());
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_MEETING).await, 1);
        assert!(relay_rows(&conn, "device-a", ROW_TYPE_ITEMS).await >= 2);
    }

    /// Review fix A: a `meeting` row pushed before `pushed_meeting_row` existed goes by its entity tag, and an
    /// earlier database's row with the same meeting id (another epoch, so another tag) stays.
    #[tokio::test]
    async fn recorder_deletes_a_legacy_meeting_row_by_its_tag() {
        let dir = tempfile::tempdir().unwrap();
        let conn = crate::content::tests::compacting_relay(&dir).await;
        let group_key = GroupKey::generate();
        let (store_a, id) = own_store("device-a");
        let epoch = store_a.sync_entity_epoch().unwrap();
        for (seq, epoch) in [(500, epoch.as_str()), (501, "an-earlier-database")] {
            let tag = crate::content::tag_for(&group_key, "device-a", epoch, &format!("meeting:{id}"));
            conn.execute(
                "INSERT INTO sync_row (device_id, local_seq, row_type, ciphertext, created_at_unix_ms, entity_tag) \
                 VALUES ('device-a', ?1, 'meeting', 'x', 0, ?2)",
                params![seq, tag],
            )
            .await
            .unwrap();
        }
        store_a.forget_meeting(id).unwrap().unwrap();
        delete_forgotten_meeting_rows(&conn, &group_key, &store_a, "device-a").await;
        let mut rows = conn.query("SELECT local_seq FROM sync_row WHERE row_type = 'meeting'", ()).await.unwrap();
        assert_eq!(rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(), 501);
        assert!(rows.next().await.unwrap().is_none());
        assert!(store_a.meetings_awaiting_tag_delete().unwrap().is_empty(), "done once");
    }

    /// Review fix B: a re-created database reuses `local_seq`s, and the relay keeps the earlier database's row
    /// at that key (the new INSERT was a no-op), yet the seq is recorded. That row predates the new meeting by
    /// days, so the cleanup leaves it.
    #[tokio::test]
    async fn an_earlier_databases_row_at_a_reused_seq_survives() {
        let (_dir, conn, group_key) = relay_and_store().await;
        let (store_a, _) = own_store("device-a");
        let now = crate::now_unix_ms();
        let id = store_a
            .insert_meeting(&NewMeeting {
                device_id: "device-a".into(),
                start_unix_ms: now,
                end_unix_ms: None,
                title: "Today".into(),
                attendees: vec![],
                trigger: MeetingTrigger::Manual,
                audio_path: None,
                status: memory::MeetingStatus::Recording,
            })
            .unwrap();
        for (seq, created) in [(7, now - 2 * 86_400_000), (8, now + 1)] {
            conn.execute(
                "INSERT INTO sync_row (device_id, local_seq, row_type, ciphertext, created_at_unix_ms) VALUES ('device-a', ?1, 'items', 'x', ?2)",
                params![seq, created],
            )
            .await
            .unwrap();
            store_a.record_pushed_meeting_row(id, seq, "items").unwrap();
        }
        store_a.forget_meeting(id).unwrap().unwrap();
        delete_forgotten_meeting_rows(&conn, &group_key, &store_a, "device-a").await;
        let mut rows = conn.query("SELECT local_seq FROM sync_row WHERE row_type = 'items'", ()).await.unwrap();
        assert_eq!(rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(), 7, "the earlier database's row stays");
        assert!(rows.next().await.unwrap().is_none(), "this meeting's row is gone");
        assert!(store_a.forgotten_pushed_meeting_rows(10).unwrap().is_empty());
    }

    /// Review fix C: an earlier database's tombstone for an id this database reuses never drops a peer's speaker
    /// names for the new meeting.
    #[tokio::test]
    async fn a_stale_own_tombstone_never_drops_peer_speaker_names() {
        use memory::speaker_names::NameStatus;
        let (_dir, conn, group_key) = relay_and_store().await;
        let (store_a, a_meeting) = own_store("device-a");
        store_a.save_meeting_voices(a_meeting, &[(0, vec![1.0, 0.0])], true).unwrap();
        let (store_b, b_meeting) = meeting_on_b(&conn, &group_key, &store_a, a_meeting).await;
        let stale = memory::MeetingForgotten {
            origin_device_id: "device-a".into(),
            origin_epoch: Some("an-earlier-database".into()),
            source_meeting_id: a_meeting,
            forgotten_at_unix_ms: 1,
            source_start_unix_ms: None,
        };
        assert!(store_a.apply_meeting_forgotten(&stale).unwrap().is_none());
        assert!(store_a.is_meeting_forgotten("device-a", None, a_meeting).unwrap(), "matches epoch-lessly");

        tick();
        store_b.upsert_person("sam@x.com", "Sam", Some("sam@x.com")).unwrap();
        store_b.set_meeting_speaker(b_meeting, 0, Some("sam@x.com"), Some(NameStatus::Confirmed)).unwrap();
        dispatch_outbox_at(&conn, &group_key, &store_b, "device-b", t0()).await.unwrap();
        pull_content(&conn, &group_key, &store_a, "device-a").await.unwrap();
        assert_eq!(store_a.meeting_speakers(a_meeting).unwrap()[0].name.as_deref(), Some("Sam"));
    }

    /// Review fix F: the cleanup runs after the outbox, so a forget's tombstone is on the relay before its
    /// meeting's server rows go.
    #[tokio::test]
    async fn a_round_cleans_up_after_the_outbox() {
        let (_dir, conn, group_key) = relay_and_store().await;
        let (store_a, a_meeting) = delivered_meeting(&conn, &group_key).await;
        store_a.forget_meeting(a_meeting).unwrap().unwrap();
        let phases = std::sync::Mutex::new(Vec::new());
        crate::content_round(&conn, &group_key, &store_a, "device-a", &|p| phases.lock().unwrap().push(p)).await.unwrap();
        assert_eq!(*phases.lock().unwrap(), ["schema", "push", "pull", "outbox", "forgotten cleanup"]);
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_MEETING_FORGOTTEN).await, 1);
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_MEETING).await, 0);
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_ITEMS).await, 0);
    }
}
