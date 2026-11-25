//! Outbox dispatcher (docs/adr/0001, slice 2): drains `memory`'s `sync_outbox` into the
//! relay's `sync_row` table. Runs right after the legacy `push_content` scan inside the
//! same `sync_content` round. Both paths insert the same plaintext under the same
//! `(device_id, local_seq, row_type)` key with `ON CONFLICT DO NOTHING`, so they coexist
//! until the legacy scan is retired; a row either path already delivered is a no-op for
//! the other. Delivery is leased per operation and acknowledged only by the lease owner,
//! so a crash between the relay write and the local ack simply resends the same key.

use std::collections::HashMap;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use libsql::Connection;
use memory::{ClaimedOutboxOperation, DeliveryFailureClass, MeetingStatus, Store};
use sync::GroupKey;

use crate::content::{
    all_peers_have_marker, all_peers_items_capable, content_row_aad, insert_with_tag_fallback,
    outbox_entity_identity, outbox_kind_tag, outbox_slice_tag, tag_for, CompactionBudget,
    ENTITY_TAG_COLUMN_SEEN, KIND_TAG_COLUMN_SEEN, SLICE_TAG_COLUMN_SEEN,
    MAX_SKIN_CHUNK_BYTES_PER_PASS, ROW_TYPE_TOPICS_CAPABILITY, ROW_TYPE_TOPIC_PAGE_CAPABILITY, ROW_TYPE_TOPIC_SNAPSHOT_CAPABILITY,
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
    /// split): acknowledged unsent with receipt `dropped:parent_meeting_gone`. Also rows about something a
    /// client delete forgot (`dropped:client_forgotten`).
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
    dispatch_outbox_until(conn, group_key, store, device_id, now_unix_ms, None).await
}

/// [`dispatch_outbox_at`] that stops at `deadline`: the rows not yet sent go back to pending, due at once,
/// and the pass returns `Ok`. A desktop round passes the time left in its `SYNC_TIMEOUT`, so a slow relay
/// delays the outbox instead of timing the whole round out (Nov 18 2026: every round failed "during
/// outbox" while its rows waited for hours).
pub(crate) async fn dispatch_outbox_until(
    conn: &Connection,
    group_key: &GroupKey,
    store: &Store,
    device_id: &str,
    now_unix_ms: i64,
    deadline: Option<tokio::time::Instant>,
) -> Result<OutboxDispatchSummary, SyncEngineError> {
    let past = || deadline.is_some_and(|d| tokio::time::Instant::now() >= d);
    let worker_id = format!("{device_id}:{}", std::process::id());
    let items_capable = all_peers_items_capable(conn, device_id).await?;
    // Topic rows are always enqueued, and left unclaimed while a peer can't apply them yet,
    // so a backlog of them never fills a pass. `push_content` backfills older rows once.
    let mut held: Vec<&str> = Vec::new();
    let topics_lacking = crate::content::peers_without_marker(conn, device_id, ROW_TYPE_TOPICS_CAPABILITY).await?;
    // How many devices hold the seed rows back (a removed device too): the exclusion setting travels in them, so
    // the shells warn on the exclude toggle while any does (X6 final review, Major 22). Informational only.
    if let Err(e) = store.set_topic_rows_held_devices(topics_lacking.len()) {
        eprintln!("[sync] topic-held device count not stored: {e}");
    }
    if !topics_lacking.is_empty() {
        held.extend([
            memory::OUTBOX_ROW_TYPE_TOPIC_SEED,
            memory::OUTBOX_ROW_TYPE_TOPIC_VERDICT,
        ]);
    }
    if !all_peers_have_marker(conn, device_id, ROW_TYPE_TOPIC_PAGE_CAPABILITY).await? {
        held.extend([
            memory::OUTBOX_ROW_TYPE_TOPIC_STEP,
            memory::OUTBOX_ROW_TYPE_TOPIC_EXTRACTION,
            memory::OUTBOX_ROW_TYPE_TOPIC_ROLE,
            memory::OUTBOX_ROW_TYPE_TOPIC_NAMES,
        ]);
    }
    if !all_peers_have_marker(conn, device_id, ROW_TYPE_TOPIC_SNAPSHOT_CAPABILITY).await? {
        held.push(memory::OUTBOX_ROW_TYPE_TOPIC_PAGE_SNAPSHOT);
    }
    // Client deletes are never held: a retired device that never updates would block them for good. A device
    // on an older build skips them and reads them again on its first pull once updated (the pull's catch-up).
    // The count is only the receipt's "N devices still need to update": a failed read or write keeps the last
    // count and never stops the pass.
    // Devices the user removed ("Remove this device") are left out of it; they release nothing.
    crate::content::record_pending_devices_and_resend_controls(conn, store, device_id).await;
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
    let mut kind_on = KIND_TAG_COLUMN_SEEN.get();
    let mut slice_on = SLICE_TAG_COLUMN_SEEN.get();
    let mut compaction = CompactionBudget::default();
    // ponytail: per-pass batching can leak one stale row per entity when SYNC_TIMEOUT hits mid-pass; upgrade path: delete before acknowledging
    let mut tags: Vec<String> = Vec::new();

    while processed < MAX_ROWS_PER_PASS && !past() {
        let batch = store
            .claim_due_outbox_batch_except(
                memory::OUTBOX_CONNECTOR,
                &worker_id,
                now_unix_ms,
                now_unix_ms + LEASE_MS,
                CLAIM_BATCH,
                &held,
            )
            .map_err(query_error)?;
        if batch.is_empty() {
            break;
        }
        for mut op in batch {
            if past() {
                store
                    .release_outbox_after_failure(
                        &op.operation_id,
                        &worker_id,
                        DeliveryFailureClass::Retryable,
                        "round_budget",
                        "held for the next round",
                        Some(now_unix_ms),
                        now_unix_ms,
                    )
                    .map_err(query_error)?;
                summary.deferred += 1;
                continue;
            }
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
            // A row about something a client delete forgot, leased before the delete cancelled the pending ones:
            // never sent.
            if store.outbox_op_forgotten(&op).map_err(query_error)? {
                store
                    .acknowledge_outbox_operation(&op.operation_id, &worker_id, "dropped:client_forgotten", now_unix_ms)
                    .map_err(query_error)?;
                summary.dropped += 1;
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
            let kind = if kind_on { outbox_kind_tag(group_key, &op) } else { None };
            let slice = if slice_on { outbox_slice_tag(group_key, &op) } else { None };
            match write_sync_row(conn, group_key, &op, tag.as_deref(), kind.as_deref(), slice.as_deref(), now_unix_ms).await {
                Ok((receipt, tagged, kind_tagged, slice_tagged)) => {
                    // The relay has no `kind_tag`: the rest of the pass goes without it.
                    if kind.is_some() && !kind_tagged {
                        kind_on = false;
                    }
                    // The relay has no `slice_tag`: the rest of the pass goes without it.
                    if slice.is_some() && !slice_tagged {
                        slice_on = false;
                    }
                    #[cfg(test)]
                    if let Some(key) = FORGET_SEED_DURING_WRITE.with(|c| c.borrow_mut().take()) {
                        store.forget_seed_topic(&key).map_err(query_error)?;
                    }
                    // Acknowledged before the batched DELETE (spec D8.2).
                    store
                        .acknowledge_outbox_operation(&op.operation_id, &worker_id, &receipt, now_unix_ms)
                        .map_err(query_error)?;
                    summary.delivered += 1;
                    // Forgotten while it was being written (past the cancel and the check above): its row goes too.
                    // A local error here never stops the pass; the op stays delivered and the send-time check
                    // still keeps later versions off the relay.
                    if let Err(e) = store.take_delivery_if_forgotten(&op, now_unix_ms) {
                        eprintln!("[sync] a row delivered after a client delete is not yet recorded for cleanup: {e}");
                    }
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

#[cfg(test)]
thread_local! {
    /// Tests: a client delete of this seed commits while the pass is writing a row (between the relay write
    /// and its acknowledgement).
    static FORGET_SEED_DURING_WRITE: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
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
        // A step or step read of this device's own meeting, `meeting:{device}:{id}`.
        memory::OUTBOX_ROW_TYPE_TOPIC_STEP | memory::OUTBOX_ROW_TYPE_TOPIC_EXTRACTION => {
            let v = serde_json::from_str::<serde_json::Value>(&op.payload_json).ok()?;
            let key = v.get("source_key")?.as_str()?;
            key.strip_prefix("meeting:")?
                .strip_prefix(op.device_id.as_str())?
                .strip_prefix(':')?
                .parse()
                .ok()
        }
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

/// Writes one op's row. The bools are true when it went in with the entity tag / kind tag / slice tag; a
/// relay that predates any of the columns gets the INSERT without it.
async fn write_sync_row(
    conn: &Connection,
    group_key: &GroupKey,
    op: &ClaimedOutboxOperation,
    tag: Option<&str>,
    kind_tag: Option<&str>,
    slice_tag: Option<&str>,
    now_unix_ms: i64,
) -> Result<(String, bool, bool, bool), SyncEngineError> {
    let aad = content_row_aad(&op.device_id, op.local_seq, &op.row_type);
    let ciphertext = STANDARD.encode(sync::encrypt_payload(group_key, op.payload_json.as_bytes(), &aad));
    let row = [(
        op.local_seq,
        op.row_type.as_str(),
        ciphertext,
        tag.map(str::to_string),
        kind_tag.map(str::to_string),
        slice_tag.map(str::to_string),
    )];
    let (mut with_entity, mut with_kind, mut with_slice) = (tag.is_some(), kind_tag.is_some(), slice_tag.is_some());
    insert_with_tag_fallback(
        conn,
        &op.device_id,
        now_unix_ms,
        &row,
        &mut with_entity,
        &mut with_kind,
        &mut with_slice,
    )
    .await
        .map_err(|e| SyncEngineError::Query(e.to_string()))?;
    Ok((
        format!("sync_row:{}:{}:{}", op.device_id, op.local_seq, op.row_type),
        with_entity,
        with_kind,
        with_slice,
    ))
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
    use libsql::params;
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

    /// A desktop round's deadline: a pass that reaches it sends nothing more, returns Ok, and leaves the rows
    /// pending and due at once, so the next round sends them (Nov 18 2026: rounds timed out "during outbox").
    #[tokio::test]
    async fn a_pass_past_its_deadline_returns_ok_and_leaves_the_rows_due() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("relay.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        let group_key = GroupKey::generate();
        let (store_a, meeting_id) = own_store("device-a");
        store_a.complete_meeting(meeting_id, 2000, None).unwrap();
        let t0 = t0();

        let past = tokio::time::Instant::now() - std::time::Duration::from_millis(1);
        let summary = dispatch_outbox_until(&conn, &group_key, &store_a, "device-a", t0, Some(past)).await.unwrap();
        assert_eq!(summary.delivered, 0);
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_MEETING).await, 0);
        assert_eq!(states(&store_a), vec!["pending"]);

        // The next round, with time left, sends it.
        let later = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        let next = dispatch_outbox_until(&conn, &group_key, &store_a, "device-a", t0 + 1, Some(later)).await.unwrap();
        assert_eq!(next.delivered, 1);
        assert_eq!(states(&store_a), vec!["delivered"]);
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

    fn topic_store(device_id: &str) -> Store {
        let store = Store::open_in_memory().unwrap();
        store.set_own_device_id(device_id);
        store
    }

    fn topic_email(store: &Store, external_id: &str, subject: &str) {
        store
            .upsert_email_items(&[memory::NewEmailItem {
                device_id: "d".into(),
                external_id: external_id.into(),
                ts_unix_ms: 1000,
                from: "a@x.com".into(),
                to: "b@y.com".into(),
                subject: subject.into(),
                body_text: "Budget review".into(),
                gmail_thread_id: external_id.into(),
                account_email: "me@x.com".into(),
                gmail_labels: vec![],
                list_unsubscribe: false,
            }])
            .unwrap();
    }

    /// A past Apple Calendar (EventKit) event: not Google user data, so TypeSafe may judge
    /// it while `TYPESAFE_GOOGLE_DATA_CONFIRMED` is false (a Gmail email from
    /// `topic_email` is held from the judge queue). Its source key
    /// (`calendar:<series>`) is the same on every device.
    fn topic_event(store: &Store, external_id: &str, title: &str) {
        store
            .upsert_calendar_event_items(&[memory::NewCalendarEventItem {
                device_id: "d".into(),
                source: "apple_calendar".into(),
                external_id: external_id.into(),
                ts_unix_ms: 1000,
                title: title.into(),
                actors: vec!["a@x.com".into(), "b@y.com".into()],
            }])
            .unwrap();
    }

    fn topic_index(store: &Store) {
        while memory::topics::index::run_topic_index_job(store, 200).unwrap() {}
        while memory::topics::cluster::run_topic_cluster_job(store, 200).unwrap() {}
    }

    /// One full `sync_content` round (push with markers, pull, outbox) against the test relay.
    async fn topic_round(conn: &Connection, group_key: &GroupKey, store: &Store, device_id: &str) {
        crate::content_round(conn, group_key, store, device_id, &|_| {}).await.unwrap();
    }

    fn outbox_count(store: &Store, row_type: &str) -> usize {
        store.list_outbox_operations().unwrap().iter().filter(|o| o.1 == row_type).count()
    }

    #[tokio::test]
    async fn topic_seed_round_trips_and_later_edit_wins() {
        use crate::content::ROW_TYPE_TOPIC_SEED;
        use memory::topics::seeds::SeedKind;
        let (_dir, conn, group_key) = relay_and_store().await;
        let (a, b) = (topic_store("device-a"), topic_store("device-b"));
        let seed = a.create_seed_topic("Acme", &["Acme Corp".into()], Some(SeedKind::Customer)).unwrap();
        topic_round(&conn, &group_key, &a, "device-a").await;
        topic_round(&conn, &group_key, &b, "device-b").await;
        let got = b.list_seed_topics().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].key.as_str(), got[0].name.as_str()), (seed.key.as_str(), "Acme"));
        assert_eq!((got[0].aliases.clone(), got[0].kind), (seed.aliases.clone(), Some(SeedKind::Customer)));
        assert_eq!(outbox_count(&b, ROW_TYPE_TOPIC_SEED), 0, "an applied seed is never re-enqueued");

        b.update_seed_topic(&seed.key, "Acme Co", &[], None).unwrap();
        tick();
        a.update_seed_topic(&seed.key, "Acme Inc", &[], Some(SeedKind::Customer)).unwrap();
        for _ in 0..2 {
            topic_round(&conn, &group_key, &a, "device-a").await;
            topic_round(&conn, &group_key, &b, "device-b").await;
        }
        for s in [&a, &b] {
            let l = s.list_seed_topics().unwrap();
            assert_eq!((l.len(), l[0].name.as_str(), l[0].kind), (1, "Acme Inc", Some(SeedKind::Customer)));
        }
    }

    #[tokio::test]
    async fn exclusion_syncs_between_devices() {
        use memory::topics::client_controls::ClientControl;
        let (_dir, conn, group_key) = relay_and_store().await;
        let (a, b) = (topic_store("device-a"), topic_store("device-b"));
        let seed = a.create_seed_topic("Acme", &["Acme Corp".into()], None).unwrap();
        let c = ClientControl { exclude_hosted: true, keep_months: Some(12) };
        a.set_client_control(&seed.key, &c).unwrap();
        topic_round(&conn, &group_key, &a, "device-a").await;
        topic_round(&conn, &group_key, &b, "device-b").await;
        assert_eq!(b.client_control(&seed.key).unwrap(), c);
        assert_eq!(b.excluded_terms().unwrap(), vec!["Acme".to_string(), "Acme Corp".to_string()]);

        tick();
        b.set_client_control(&seed.key, &ClientControl::default()).unwrap();
        for _ in 0..2 {
            topic_round(&conn, &group_key, &b, "device-b").await;
            topic_round(&conn, &group_key, &a, "device-a").await;
        }
        assert_eq!(a.client_control(&seed.key).unwrap(), ClientControl::default());
        assert!(a.excluded_terms().unwrap().is_empty());
    }

    #[tokio::test]
    async fn exclusion_survives_stale_rename_from_other_device() {
        use memory::topics::client_controls::ClientControl;
        let (_dir, conn, group_key) = relay_and_store().await;
        let (a, b) = (topic_store("device-a"), topic_store("device-b"));
        let seed = a.create_seed_topic("Acme", &[], None).unwrap();
        // A holds an older control row (exclusion off), synced to B.
        a.set_client_control(&seed.key, &ClientControl { exclude_hosted: false, keep_months: None }).unwrap();
        topic_round(&conn, &group_key, &a, "device-a").await;
        topic_round(&conn, &group_key, &b, "device-b").await;
        assert!(!b.client_control(&seed.key).unwrap().exclude_hosted);
        tick();
        b.set_client_control(&seed.key, &ClientControl { exclude_hosted: true, keep_months: None }).unwrap();
        tick();
        // A renames the seed later, before pulling B's change: its record carries the old row.
        a.update_seed_topic(&seed.key, "Acme Inc", &[], None).unwrap();
        for _ in 0..3 {
            topic_round(&conn, &group_key, &a, "device-a").await;
            topic_round(&conn, &group_key, &b, "device-b").await;
        }
        for s in [&a, &b] {
            assert!(s.client_control(&seed.key).unwrap().exclude_hosted);
            assert_eq!(s.list_seed_topics().unwrap()[0].name, "Acme Inc");
        }
    }

    #[tokio::test]
    async fn merged_exclusion_reaches_peer() {
        use memory::topics::client_controls::ClientControl;
        let (_dir, conn, group_key) = relay_and_store().await;
        let (a, b) = (topic_store("device-a"), topic_store("device-b"));
        let x = a.create_seed_topic("Xco", &[], None).unwrap();
        let y = a.create_seed_topic("Yco", &[], None).unwrap();
        a.set_client_control(&x.key, &ClientControl { exclude_hosted: true, keep_months: None }).unwrap();
        tick();
        a.merge_topics(y.topic_id, x.topic_id).unwrap();
        for _ in 0..2 {
            topic_round(&conn, &group_key, &a, "device-a").await;
            topic_round(&conn, &group_key, &b, "device-b").await;
        }
        assert!(b.client_control(&y.key).unwrap().exclude_hosted);
    }

    #[tokio::test]
    async fn seed_merge_and_delete_apply_on_pull() {
        let (_dir, conn, group_key) = relay_and_store().await;
        let (a, b) = (topic_store("device-a"), topic_store("device-b"));
        let x = a.create_seed_topic("Acme", &[], None).unwrap();
        let y = a.create_seed_topic("Acme Inc", &["Acme Incorporated".into()], None).unwrap();
        let z = a.create_seed_topic("Board", &[], None).unwrap();
        topic_round(&conn, &group_key, &a, "device-a").await;
        topic_round(&conn, &group_key, &b, "device-b").await;
        assert_eq!(b.list_seed_topics().unwrap().len(), 3);

        a.merge_topics(x.topic_id, y.topic_id).unwrap();
        a.delete_seed_topic(&z.key).unwrap();
        topic_round(&conn, &group_key, &a, "device-a").await;
        topic_round(&conn, &group_key, &b, "device-b").await;
        let l = b.list_seed_topics().unwrap();
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].key, x.key);
        for want in ["Acme", "Acme Inc", "Acme Incorporated"] {
            assert!(l[0].aliases.iter().any(|a| a == want), "missing {want}");
        }
    }

    /// I2: a merge on one device and a later edit of the merged seed on another: both end merged.
    #[tokio::test]
    async fn topic_merge_survives_a_later_edit_on_another_device() {
        let (_dir, conn, group_key) = relay_and_store().await;
        let (a, b) = (topic_store("device-a"), topic_store("device-b"));
        let x = a.create_seed_topic("Acme", &[], None).unwrap();
        let y = a.create_seed_topic("Acme Inc", &[], None).unwrap();
        topic_round(&conn, &group_key, &a, "device-a").await;
        topic_round(&conn, &group_key, &b, "device-b").await;

        a.merge_topics(x.topic_id, y.topic_id).unwrap();
        tick();
        b.update_seed_topic(&y.key, "Acme Incorporated", &[], None).unwrap();
        for _ in 0..2 {
            topic_round(&conn, &group_key, &a, "device-a").await;
            topic_round(&conn, &group_key, &b, "device-b").await;
        }
        for (name, s) in [("a", &a), ("b", &b)] {
            let l = s.list_seed_topics().unwrap();
            assert_eq!(l.len(), 1, "device {name}: {l:?}");
            assert_eq!(l[0].key, x.key, "device {name}");
        }
    }

    #[tokio::test]
    async fn verdict_round_trips_and_prevents_a_second_judgment() {
        use crate::content::ROW_TYPE_TOPIC_VERDICT;
        let (_dir, conn, group_key) = relay_and_store().await;
        let (a, b) = (topic_store("device-a"), topic_store("device-b"));
        let seed = a.create_seed_topic("Maximus", &[], None).unwrap();
        topic_event(&a, "e1", "Lunch plans");
        topic_event(&b, "e1", "Lunch plans");
        topic_round(&conn, &group_key, &a, "device-a").await;
        topic_round(&conn, &group_key, &b, "device-b").await;
        topic_index(&a);
        topic_index(&b);
        // Both devices queue the source for Jev.
        assert_eq!(b.next_judge_candidates(10).unwrap().len(), 1);
        let c = a.next_judge_candidates(10).unwrap();
        assert_eq!(c.len(), 1);
        a.record_topic_verdict(c[0].source_id, Some(seed.topic_id), Some(0.9), "jev", 5).unwrap();

        topic_round(&conn, &group_key, &a, "device-a").await;
        topic_round(&conn, &group_key, &b, "device-b").await;
        assert!(b.next_judge_candidates(10).unwrap().is_empty(), "the pulled verdict decides it on b");
        assert_eq!(b.list_seed_topics().unwrap()[0].item_count, 1);
        assert_eq!(outbox_count(&b, ROW_TYPE_TOPIC_VERDICT), 0, "an applied verdict is never re-enqueued");
    }

    #[tokio::test]
    async fn verdict_pulled_before_its_source_applies_when_indexed() {
        let (_dir, conn, group_key) = relay_and_store().await;
        let (a, b) = (topic_store("device-a"), topic_store("device-b"));
        let seed = a.create_seed_topic("Maximus", &[], None).unwrap();
        topic_event(&a, "e1", "Lunch plans");
        topic_index(&a);
        let c = a.next_judge_candidates(10).unwrap();
        a.record_topic_verdict(c[0].source_id, Some(seed.topic_id), Some(0.9), "jev", 5).unwrap();
        topic_round(&conn, &group_key, &a, "device-a").await;
        topic_round(&conn, &group_key, &b, "device-b").await;

        topic_event(&b, "e1", "Lunch plans");
        topic_index(&b);
        assert!(b.next_judge_candidates(10).unwrap().is_empty());
        assert_eq!(b.list_seed_topics().unwrap()[0].item_count, 1);
    }

    /// Limited Use: a Gmail email is never offered to TypeSafe on either device, before or
    /// after a sync round, while TypeSafe has not confirmed it does not train on API data.
    #[tokio::test]
    async fn gmail_source_is_held_from_typesafe_on_both_devices() {
        let (_dir, conn, group_key) = relay_and_store().await;
        let (a, b) = (topic_store("device-a"), topic_store("device-b"));
        a.create_seed_topic("Maximus", &[], None).unwrap();
        topic_email(&a, "e1", "Lunch plans");
        topic_email(&b, "e1", "Lunch plans");
        topic_index(&a);
        let held = !memory::limited_use::TYPESAFE_GOOGLE_DATA_CONFIRMED;
        assert_eq!(a.next_judge_candidates(10).unwrap().is_empty(), held);
        topic_round(&conn, &group_key, &a, "device-a").await;
        topic_round(&conn, &group_key, &b, "device-b").await;
        topic_index(&a);
        topic_index(&b);
        for s in [&a, &b] {
            assert_eq!(s.next_judge_candidates(10).unwrap().is_empty(), held);
        }
    }

    /// Held topic rows never fill a pass: a person row queued behind 150 of them still goes out.
    #[tokio::test]
    async fn held_topic_rows_do_not_starve_other_rows() {
        use crate::content::{push_marker, ROW_TYPE_ITEMS_CAPABILITY, ROW_TYPE_PERSON, ROW_TYPE_TOPIC_SEED};
        let (_dir, conn, group_key) = relay_and_store().await;
        push_marker(&conn, &group_key, "device-old", ROW_TYPE_ITEMS_CAPABILITY).await.unwrap();
        let a = topic_store("device-a");
        for i in 0..150 {
            a.create_seed_topic(&format!("Seed {i}"), &[], None).unwrap();
        }
        a.upsert_person("sam@x.com", "Sam", Some("sam@x.com")).unwrap();
        dispatch_outbox_at(&conn, &group_key, &a, "device-a", t0()).await.unwrap();
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_PERSON).await, 1);
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_TOPIC_SEED).await, 0);
    }

    #[tokio::test]
    async fn topic_rows_wait_for_every_device_capability() {
        use crate::content::{push_marker, ROW_TYPE_ITEMS_CAPABILITY, ROW_TYPE_TOPICS_CAPABILITY, ROW_TYPE_TOPIC_SEED};
        let (_dir, conn, group_key) = relay_and_store().await;
        // A peer on an older build: it announces items, not topics.
        push_marker(&conn, &group_key, "device-old", ROW_TYPE_ITEMS_CAPABILITY).await.unwrap();
        let a = Store::open_in_memory().unwrap();
        a.create_seed_topic("Before sync", &[], None).unwrap(); // no device id yet: never enqueued
        a.set_own_device_id("device-a");
        a.create_seed_topic("Acme", &[], None).unwrap();

        let t = t0();
        assert_eq!(a.topic_rows_held_devices().unwrap(), None, "not measured before a pass");
        dispatch_outbox_at(&conn, &group_key, &a, "device-a", t).await.unwrap();
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_TOPIC_SEED).await, 0);
        assert_eq!(states(&a), vec!["pending".to_string()], "held, untouched");
        // The shells' exclude-toggle warning (X6 final review, Major 22): one device holds the seed rows back.
        assert_eq!(a.topic_rows_held_devices().unwrap(), Some(1));

        push_marker(&conn, &group_key, "device-old", ROW_TYPE_TOPICS_CAPABILITY).await.unwrap();
        crate::content::push_content(&conn, &group_key, &a, "device-a").await.unwrap();
        dispatch_outbox_at(&conn, &group_key, &a, "device-a", t + DEFERRED_RECHECK_MS + 1).await.unwrap();
        assert_eq!(a.topic_rows_held_devices().unwrap(), Some(0));
        let b = topic_store("device-b");
        pull_content(&conn, &group_key, &b, "device-b").await.unwrap();
        assert_eq!(b.list_seed_topics().unwrap().len(), 2, "the backfill sends the seed made before sync");

        let ops = outbox_count(&a, ROW_TYPE_TOPIC_SEED);
        crate::content::push_content(&conn, &group_key, &a, "device-a").await.unwrap();
        dispatch_outbox_at(&conn, &group_key, &a, "device-a", t + 2 * DEFERRED_RECHECK_MS).await.unwrap();
        assert_eq!(outbox_count(&a, ROW_TYPE_TOPIC_SEED), ops, "the backfill runs once");
    }

    const DAY: i64 = 86_400_000;

    /// An email from yesterday, so the step queue (last 30 days) takes it.
    fn recent_email(store: &Store, external_id: &str, subject: &str) {
        store
            .upsert_email_items(&[memory::NewEmailItem {
                device_id: "d".into(),
                external_id: external_id.into(),
                ts_unix_ms: crate::now_unix_ms() - DAY,
                from: "Pat <pat@x.com>".into(),
                to: "me@x.com".into(),
                subject: subject.into(),
                body_text: "Please send the SOW by Friday.".into(),
                gmail_thread_id: external_id.into(),
                account_email: "me@x.com".into(),
                gmail_labels: vec![],
                list_unsubscribe: false,
            }])
            .unwrap();
    }

    /// Reads every queued source with each candidate line labelled `mine`.
    fn read_steps(store: &Store) {
        for src in store.next_step_sources(10).unwrap() {
            let labels: Vec<memory::topics::steps::LineLabel> = src
                .candidates
                .iter()
                .map(|_| memory::topics::steps::LineLabel {
                    direction: Some("mine"),
                    is_date: false,
                })
                .collect();
            store
                .record_step_extraction(&src, &labels, &[], "m", crate::now_unix_ms())
                .unwrap();
        }
    }

    /// The page of the store's only seed topic.
    fn page(store: &Store) -> memory::topics::page::TopicPage {
        let id = store.list_seed_topics().unwrap()[0].topic_id;
        store.topic_page(id, crate::now_unix_ms()).unwrap().unwrap()
    }

    async fn topic_rounds(conn: &Connection, group_key: &GroupKey, a: &Store, b: &Store) {
        for _ in 0..2 {
            topic_round(conn, group_key, a, "device-a").await;
            topic_round(conn, group_key, b, "device-b").await;
        }
    }

    #[tokio::test]
    async fn topic_step_round_trips_and_later_edit_wins() {
        use memory::topics::steps::StepAction;
        let (_dir, conn, group_key) = relay_and_store().await;
        let (a, b) = (topic_store("device-a"), topic_store("device-b"));
        a.create_seed_topic("Maximus", &[], None).unwrap();
        recent_email(&a, "e1", "Maximus SOW");
        topic_index(&a);
        read_steps(&a);
        let key = page(&a).you_owe[0].key.clone();
        topic_rounds(&conn, &group_key, &a, &b).await;
        let got = page(&b).you_owe;
        assert_eq!(got.len(), 1);
        assert_eq!(
            (got[0].key.as_str(), got[0].text.as_str()),
            (key.as_str(), "Please send the SOW by Friday.")
        );
        assert_eq!(
            outbox_count(&b, "topic_step"),
            0,
            "an applied step is never re-enqueued"
        );

        b.apply_step_action(&key, &StepAction::Note { text: "from b".into() })
            .unwrap();
        tick();
        a.apply_step_action(&key, &StepAction::Note { text: "from a".into() })
            .unwrap();
        topic_rounds(&conn, &group_key, &a, &b).await;
        for s in [&a, &b] {
            assert_eq!(page(s).you_owe[0].note.as_deref(), Some("from a"));
        }
    }

    /// Jev is never paid twice: a source read on one device is not read again on another.
    #[tokio::test]
    async fn pulled_extraction_prevents_a_second_read() {
        let (_dir, conn, group_key) = relay_and_store().await;
        let (a, b) = (topic_store("device-a"), topic_store("device-b"));
        a.create_seed_topic("Maximus", &[], None).unwrap();
        recent_email(&a, "e1", "Maximus SOW");
        topic_index(&a);
        read_steps(&a);
        topic_rounds(&conn, &group_key, &a, &b).await;

        recent_email(&b, "e1", "Maximus SOW");
        topic_index(&b);
        assert!(
            b.next_step_sources(10).unwrap().is_empty(),
            "read on a already"
        );
        assert_eq!(page(&b).you_owe.len(), 1);
        assert_eq!(
            outbox_count(&b, "topic_extraction"),
            0,
            "an applied extraction is never re-enqueued"
        );
    }

    /// A finished "Maximus sync" meeting on a (device-a) with two action items, filed on seed
    /// "Maximus" and read into two `mine` steps. Returns the item texts.
    fn read_meeting(a: &Store) -> [String; 2] {
        a.create_seed_topic("Maximus", &[], None).unwrap();
        let start = crate::now_unix_ms() - DAY;
        let mid = a
            .insert_meeting(&NewMeeting {
                device_id: "device-a".into(),
                start_unix_ms: start,
                end_unix_ms: None,
                title: "Maximus sync".into(),
                attendees: vec![],
                trigger: MeetingTrigger::Calendar,
                audio_path: None,
                status: memory::MeetingStatus::Recording,
            })
            .unwrap();
        a.complete_meeting(mid, start + 1000, None).unwrap();
        let items = [
            "Send the deck to Priya".to_string(),
            "Book the venue".to_string(),
        ];
        a.update_meeting_summary(mid, "t", &[], &items, &[], "p", "m")
            .unwrap();
        topic_index(a);
        read_steps(a);
        items
    }

    #[tokio::test]
    async fn pulled_done_meeting_step_completes_the_item_when_the_meeting_arrives() {
        use memory::topics::steps::StepAction;
        let (_dir, conn, group_key) = relay_and_store().await;
        let (a, b) = (topic_store("device-a"), topic_store("device-b"));
        let items = read_meeting(&a);
        let key_of = |s: &Store, text: &str| {
            page(s)
                .you_owe
                .into_iter()
                .find(|v| v.text == text)
                .unwrap()
                .key
        };
        a.apply_step_action(&key_of(&a, &items[0]), &StepAction::Done)
            .unwrap();
        let book = key_of(&a, &items[1]);
        // b pulls the meeting and the done step; its action items appear only once it reads it.
        topic_rounds(&conn, &group_key, &a, &b).await;
        topic_index(&b);
        b.next_step_sources(10).unwrap();
        let b_mid = b.recent_meetings(1).unwrap()[0].id;
        let completed = |text: &str| {
            b.list_meeting_action_items(b_mid)
                .unwrap()
                .into_iter()
                .find(|i| i.text == text)
                .unwrap()
                .completed
        };
        assert!(
            completed(&items[0]),
            "the pulled done step checks the item off on arrival"
        );
        assert!(!completed(&items[1]));

        // With the meeting here, a later done step checks its item off when pulled.
        a.apply_step_action(&book, &StepAction::Done).unwrap();
        topic_rounds(&conn, &group_key, &a, &b).await;
        assert!(completed(&items[1]));
    }

    /// "Forget meeting everywhere" wins over topic sync: a step edited on the recorder after the
    /// forget is dropped on pull, and the recorder removes its step rows from the relay.
    #[tokio::test]
    async fn forgotten_meeting_steps_are_not_resurrected_by_sync() {
        use memory::topics::steps::StepAction;
        let (_dir, conn, group_key) = relay_and_store().await;
        let (a, b) = (topic_store("device-a"), topic_store("device-b"));
        read_meeting(&a);
        let key = page(&a).you_owe[0].key.clone();
        topic_rounds(&conn, &group_key, &a, &b).await;
        assert_eq!(page(&b).you_owe.len(), 2);

        let b_mid = b.recent_meetings(1).unwrap()[0].id;
        b.forget_meeting(b_mid).unwrap().unwrap();
        assert!(
            page(&b).you_owe.is_empty(),
            "forgetting deletes the pulled steps"
        );
        // a edits a step and sends it before it hears of the forget.
        a.apply_step_action(&key, &StepAction::Note { text: "late".into() })
            .unwrap();
        dispatch_outbox_at(&conn, &group_key, &a, "device-a", t0())
            .await
            .unwrap();
        topic_rounds(&conn, &group_key, &a, &b).await;
        let pb = page(&b);
        assert!(
            pb.you_owe.is_empty() && pb.done.is_empty(),
            "not resurrected"
        );
        assert!(page(&a).you_owe.is_empty(), "a applied b's tombstone");
        crate::content::delete_forgotten_meeting_rows(&conn, &group_key, &a, "device-a").await;
        assert_eq!(
            relay_rows(&conn, "device-a", "topic_step").await,
            0,
            "the recorder removes its step rows"
        );
    }

    #[tokio::test]
    async fn role_and_names_round_trip() {
        let (_dir, conn, group_key) = relay_and_store().await;
        let (a, b) = (topic_store("device-a"), topic_store("device-b"));
        let seed = a.create_seed_topic("Maximus", &[], None).unwrap();
        a.set_topic_role(&seed.key, "I run the rollout").unwrap();
        a.set_self_names(&["Mike P".into()]).unwrap();
        topic_rounds(&conn, &group_key, &a, &b).await;
        let role = page(&b).role;
        assert_eq!(
            (role.text.as_str(), role.edited),
            ("I run the rollout", true)
        );
        assert_eq!(b.self_names().unwrap(), vec!["Mike P".to_string()]);
        assert_eq!(
            outbox_count(&b, "topic_role") + outbox_count(&b, "topic_names"),
            0
        );

        tick();
        b.set_topic_role(&seed.key, "").unwrap();
        b.set_self_names(&["Mike".into(), "MP".into()]).unwrap();
        topic_rounds(&conn, &group_key, &a, &b).await;
        assert!(!page(&a).role.edited, "b's later reset wins");
        assert_eq!(
            a.self_names().unwrap(),
            vec!["Mike".to_string(), "MP".to_string()]
        );
    }

    /// Builds that announce `topic_page_capability` still drop snapshot rows, so a snapshot waits
    /// for `topic_snapshot_capability` from every peer while other topic rows go on.
    #[tokio::test]
    async fn snapshot_rows_wait_for_every_peer_marker() {
        use crate::content::{push_marker, ROW_TYPE_ITEMS_CAPABILITY, ROW_TYPE_TOPICS_CAPABILITY};
        let (_dir, conn, group_key) = relay_and_store().await;
        for marker in [
            ROW_TYPE_ITEMS_CAPABILITY,
            ROW_TYPE_TOPICS_CAPABILITY,
            "topic_page_capability",
        ] {
            push_marker(&conn, &group_key, "device-old", marker).await.unwrap();
        }
        let a = topic_store("device-a");
        a.create_seed_topic("Maximus", &[], None).unwrap();
        recent_email(&a, "e1", "Maximus SOW");
        topic_index(&a);
        read_steps(&a);
        a.set_self_names(&["Mike P".into()]).unwrap();
        a.publish_topic_page_snapshots("device-a", crate::now_unix_ms()).unwrap();
        assert_eq!(outbox_count(&a, "topic_page_snapshot"), 1);
        assert!(outbox_count(&a, "topic_step") >= 1);

        let t = t0();
        dispatch_outbox_at(&conn, &group_key, &a, "device-a", t).await.unwrap();
        assert_eq!(
            relay_rows(&conn, "device-a", "topic_page_snapshot").await,
            0,
            "held"
        );
        assert!(
            relay_rows(&conn, "device-a", "topic_step").await >= 1,
            "other topic rows still go"
        );

        push_marker(&conn, &group_key, "device-old", "topic_snapshot_capability")
            .await
            .unwrap();
        dispatch_outbox_at(&conn, &group_key, &a, "device-a", t + DEFERRED_RECHECK_MS + 1)
            .await
            .unwrap();
        assert_eq!(relay_rows(&conn, "device-a", "topic_page_snapshot").await, 1);
    }

    /// Piece-1 builds announce `topics_capability` but drop unknown row types, so step, extraction,
    /// role and names rows wait for `topic_page_capability`.
    #[tokio::test]
    async fn topic_page_rows_wait_for_the_new_capability() {
        use crate::content::{push_marker, ROW_TYPE_ITEMS_CAPABILITY, ROW_TYPE_TOPICS_CAPABILITY};
        let (_dir, conn, group_key) = relay_and_store().await;
        push_marker(&conn, &group_key, "device-old", ROW_TYPE_ITEMS_CAPABILITY)
            .await
            .unwrap();
        push_marker(&conn, &group_key, "device-old", ROW_TYPE_TOPICS_CAPABILITY)
            .await
            .unwrap();
        let a = Store::open_in_memory().unwrap();
        a.create_seed_topic("Maximus", &[], None).unwrap();
        recent_email(&a, "e1", "Maximus SOW");
        topic_index(&a);
        read_steps(&a); // no device id yet: never enqueued
        a.set_own_device_id("device-a");
        a.set_self_names(&["Mike P".into()]).unwrap();

        let t = t0();
        crate::content::push_content(&conn, &group_key, &a, "device-a")
            .await
            .unwrap();
        dispatch_outbox_at(&conn, &group_key, &a, "device-a", t)
            .await
            .unwrap();
        assert_eq!(
            relay_rows(&conn, "device-a", "topic_names").await,
            0,
            "held"
        );
        assert_eq!(relay_rows(&conn, "device-a", "topic_step").await, 0);
        assert_eq!(outbox_count(&a, "topic_step"), 0, "no backfill yet");

        push_marker(&conn, &group_key, "device-old", "topic_page_capability")
            .await
            .unwrap();
        crate::content::push_content(&conn, &group_key, &a, "device-a")
            .await
            .unwrap();
        dispatch_outbox_at(
            &conn,
            &group_key,
            &a,
            "device-a",
            t + DEFERRED_RECHECK_MS + 1,
        )
        .await
        .unwrap();
        let b = topic_store("device-b");
        pull_content(&conn, &group_key, &b, "device-b")
            .await
            .unwrap();
        assert_eq!(
            page(&b).you_owe.len(),
            1,
            "the backfill sends the step made before sync"
        );
        assert_eq!(b.self_names().unwrap(), vec!["Mike P".to_string()]);

        let ops = outbox_count(&a, "topic_step");
        crate::content::push_content(&conn, &group_key, &a, "device-a")
            .await
            .unwrap();
        dispatch_outbox_at(
            &conn,
            &group_key,
            &a,
            "device-a",
            t + 2 * DEFERRED_RECHECK_MS,
        )
        .await
        .unwrap();
        assert_eq!(
            outbox_count(&a, "topic_step"),
            ops,
            "the backfill runs once"
        );
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
    async fn outbox_transcript_items_carry_the_kind_tag() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("relay.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        let group_key = GroupKey::generate();
        let (store_a, meeting_id) = own_store("device-a");
        store_a
            .insert_transcript_segments(&[NewTranscriptSegment {
                meeting_id,
                start_cs: 0,
                speaker_label: "Me".into(),
                text: "pelican".into(),
            }])
            .unwrap();
        store_a.complete_meeting(meeting_id, 2000, None).unwrap();

        dispatch_outbox_at(&conn, &group_key, &store_a, "device-a", t0()).await.unwrap();

        let tag = sync::kind_tag(&group_key, "transcript");
        let mut rows = conn
            .query("SELECT COUNT(*) FROM sync_row WHERE row_type = 'items' AND kind_tag = ?1", params![tag])
            .await
            .unwrap();
        assert_eq!(rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(), 1);
        let mut rows = conn
            .query("SELECT COUNT(*) FROM sync_row WHERE row_type = 'meeting' AND kind_tag IS NOT NULL", ())
            .await
            .unwrap();
        assert_eq!(rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(), 0);
    }

    #[tokio::test]
    async fn outbox_transcript_items_carry_the_slice_tag() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("relay.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        let group_key = GroupKey::generate();
        let (store_a, meeting_id) = own_store("device-a");
        store_a
            .insert_transcript_segments(&[NewTranscriptSegment {
                meeting_id,
                start_cs: 0,
                speaker_label: "Me".into(),
                text: "pelican".into(),
            }])
            .unwrap();
        store_a.complete_meeting(meeting_id, 2000, None).unwrap();

        dispatch_outbox_at(&conn, &group_key, &store_a, "device-a", t0()).await.unwrap();

        let mut rows = conn
            .query("SELECT slice_tag FROM sync_row WHERE row_type = 'items'", ())
            .await
            .unwrap();
        let tag = rows.next().await.unwrap().unwrap().get::<Option<String>>(0).unwrap();
        assert!(tag.is_some_and(|t| t.len() == 24), "the transcript item is slice tagged");
        let mut rows = conn
            .query("SELECT COUNT(*) FROM sync_row WHERE row_type = 'meeting' AND slice_tag IS NOT NULL", ())
            .await
            .unwrap();
        assert_eq!(rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(), 0);
    }

    #[tokio::test]
    async fn outbox_against_a_relay_without_slice_tag_still_delivers() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("relay.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        conn.execute("ALTER TABLE sync_row DROP COLUMN slice_tag", ()).await.unwrap();
        let group_key = GroupKey::generate();
        let (store_a, meeting_id) = own_store("device-a");
        store_a
            .insert_transcript_segments(&[NewTranscriptSegment {
                meeting_id,
                start_cs: 0,
                speaker_label: "Me".into(),
                text: "pelican".into(),
            }])
            .unwrap();
        store_a.complete_meeting(meeting_id, 2000, None).unwrap();

        let summary = dispatch_outbox_at(&conn, &group_key, &store_a, "device-a", t0()).await.unwrap();

        assert!(summary.delivered >= 1);
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_ITEMS).await, 1);
        let tag = sync::kind_tag(&group_key, "transcript");
        let mut rows = conn
            .query("SELECT COUNT(*) FROM sync_row WHERE row_type = 'items' AND kind_tag = ?1", params![tag])
            .await
            .unwrap();
        assert_eq!(rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(), 1, "kind_tag still written");
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

    /// #740 (known gap): a continued meeting (Mac `reopen_meeting`) goes back to recording, and
    /// a line written then keeps its local_seq and is held in the outbox until the meeting ends
    /// again, while captures numbered after it reach the relay first. The line then lands below
    /// a caught-up puller's cursor, which never reads there again. To be fixed push-side.
    #[tokio::test]
    #[ignore = "known gap, #740"]
    async fn a_caught_up_mac_gets_a_continued_meetings_late_line() {
        let (_dir, conn, group_key) = relay_and_store().await;
        let (store_a, meeting_id) = own_store("device-a");
        let mac = peer("device-b");
        let round = |at: i64| {
            let (conn, group_key, store_a, mac) = (&conn, &group_key, &store_a, &mac);
            async move {
                push_content(conn, group_key, store_a, "device-a")
                    .await
                    .unwrap();
                dispatch_outbox_at(conn, group_key, store_a, "device-a", at)
                    .await
                    .unwrap();
                pull_content(conn, group_key, mac, "device-b")
                    .await
                    .unwrap();
            }
        };
        let capture = |text: &str, hash: i64| {
            store_a
                .insert_capture_item(&memory::NewCaptureItem {
                    device_id: "device-a".into(),
                    ts_unix_ms: crate::now_unix_ms(),
                    app_bundle_id: "com.a".into(),
                    window_title: "W".into(),
                    url: None,
                    text: text.into(),
                    hash,
                    source: "ax".into(),
                })
                .unwrap();
        };
        store_a.complete_meeting(meeting_id, 2000, None).unwrap();
        round(t0()).await;
        assert!(mac
            .resolve_sync_meeting_map("device-a", meeting_id)
            .unwrap()
            .is_some());

        // Continued: recording again, captures, a line, more captures, all synced while recording.
        store_a.reopen_meeting(meeting_id).unwrap();
        capture("osprey", 1);
        store_a
            .insert_transcript_segments(&[NewTranscriptSegment {
                meeting_id,
                start_cs: 0,
                speaker_label: "Me".into(),
                text: "pelican".into(),
            }])
            .unwrap();
        capture("heron", 2);
        round(t0() + DEFERRED_RECHECK_MS).await;

        store_a.complete_meeting(meeting_id, 3000, None).unwrap();
        round(t0() + 2 * DEFERRED_RECHECK_MS).await;
        assert!(
            has_line(&mac, "pelican"),
            "the caught-up Mac gets the continued meeting's line"
        );
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
        assert_eq!(*phases.lock().unwrap(), ["schema", "push", "pull", "outbox", "forgotten cleanup", "kind backfill"]);
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_MEETING_FORGOTTEN).await, 1);
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_MEETING).await, 0);
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_ITEMS).await, 0);
    }

    // ---- Client deletes (docs/superpowers/specs/2026-11-12-client-controls-design.md 4.3, 13.3) ----

    use memory::{ClientItemKind, TombstoneOutcome};

    const CLIENT: &str = "Zorblax";
    const ROW_TYPE_CLIENT_ITEM_FORGOTTEN: &str = memory::OUTBOX_ROW_TYPE_CLIENT_ITEM_FORGOTTEN;
    const ROW_TYPE_TOPIC_SEED_PURGED: &str = memory::OUTBOX_ROW_TYPE_TOPIC_SEED_PURGED;

    fn client_capture(store: &Store, device: &str) {
        store
            .insert_capture_item(&memory::NewCaptureItem {
                device_id: device.into(),
                ts_unix_ms: crate::now_unix_ms() - 1_000,
                app_bundle_id: "com.apple.Safari".into(),
                window_title: format!("{CLIENT} proposal"),
                url: None,
                text: format!("Draft for {CLIENT}"),
                hash: 7,
                source: "ax".into(),
            })
            .unwrap()
            .unwrap();
    }

    fn client_event_with_id(store: &Store, device: &str, external_id: &str, title: &str) {
        store
            .upsert_calendar_event_items(&[memory::NewCalendarEventItem {
                device_id: device.into(),
                source: "google_calendar".into(),
                external_id: external_id.into(),
                ts_unix_ms: crate::now_unix_ms() + DAY,
                title: title.into(),
                actors: vec![],
            }])
            .unwrap();
    }

    fn client_event(store: &Store, device: &str, title: &str) {
        client_event_with_id(store, device, "ev-1", title);
    }

    /// The `items` rows of `kind` that name the client.
    fn client_items(store: &Store, kind: &str) -> Vec<memory::ExportItemRow> {
        store
            .export_items_page(kind, 0, 1_000)
            .unwrap()
            .into_iter()
            .filter(|r| r.title.as_deref().unwrap_or("").contains(CLIENT) || r.body_text.contains(CLIENT))
            .collect()
    }

    fn items_kind(kind: ClientItemKind) -> &'static str {
        match kind {
            ClientItemKind::Capture => "capture",
            ClientItemKind::CalendarEvent => "calendar_event",
            ClientItemKind::Email => "email",
            _ => unreachable!(),
        }
    }

    /// What the delete engine (Task 4) does for one item: queue the tombstone first, then delete it here
    /// (in the test, the same delete a peer runs).
    fn forget_item(store: &Store, kind: ClientItemKind) {
        let before = client_items(store, items_kind(kind)).len();
        let id = client_items(store, items_kind(kind))[0].id;
        assert_eq!(store.enqueue_client_item_forgotten(kind, &id.to_string()).unwrap(), TombstoneOutcome::Queued);
        let queued: memory::ClientItemForgotten = store
            .list_outbox_operations()
            .unwrap()
            .into_iter()
            .rev()
            .find(|o| o.1 == ROW_TYPE_CLIENT_ITEM_FORGOTTEN)
            .map(|o| serde_json::from_str(&o.3).unwrap())
            .unwrap();
        // A capture made here also has its `capture_item` twin.
        assert!(store.apply_client_item_forgotten(&queued).unwrap() >= 1);
        assert_eq!(client_items(store, items_kind(kind)).len(), before - 1);
    }

    /// An older build that has never announced `client_controls_capability`.
    async fn without_marker(conn: &Connection, device_id: &str) {
        conn.execute(
            "DELETE FROM sync_row WHERE device_id = ?1 AND row_type = ?2",
            params![device_id, crate::content::ROW_TYPE_CLIENT_CONTROLS_CAPABILITY],
        )
        .await
        .unwrap();
    }

    /// One round of `store` as a build from before client deletes: it skips the tombstone row types and
    /// announces no marker.
    async fn old_build_round(conn: &Connection, group_key: &GroupKey, store: &Store, device_id: &str) {
        crate::content::OLD_BUILD_PULLER.with(|c| c.set(true));
        topic_round(conn, group_key, store, device_id).await;
        crate::content::OLD_BUILD_PULLER.with(|c| c.set(false));
        without_marker(conn, device_id).await;
    }

    /// Every plaintext `device_id` pushed of `row_type`.
    async fn relay_plaintexts(conn: &Connection, group_key: &GroupKey, device_id: &str, row_type: &str) -> Vec<String> {
        let mut rows = conn
            .query(
                "SELECT local_seq, ciphertext FROM sync_row WHERE device_id = ?1 AND row_type = ?2",
                params![device_id, row_type],
            )
            .await
            .unwrap();
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.unwrap() {
            let seq: i64 = row.get(0).unwrap();
            let ciphertext = STANDARD.decode(row.get::<String>(1).unwrap()).unwrap();
            let aad = content_row_aad(device_id, seq, row_type);
            out.push(String::from_utf8(sync::decrypt_payload(group_key, &ciphertext, &aad).unwrap()).unwrap());
        }
        out
    }

    #[tokio::test]
    async fn new_tombstone_applies_on_second_device() {
        let (_dir, conn, group_key) = relay_and_store().await;
        let (a, b) = (topic_store("device-a"), topic_store("device-b"));
        client_capture(&a, "device-a");
        client_event(&a, "device-a", &format!("{CLIENT} kickoff"));
        let seed = a.create_seed_topic(CLIENT, &[format!("{CLIENT} Ltd")], None).unwrap();
        // Mail is not synced: each computer fetches its own copy of the same message.
        recent_email(&a, "m1", &format!("{CLIENT} SOW"));
        recent_email(&b, "m1", &format!("{CLIENT} SOW"));
        topic_rounds(&conn, &group_key, &a, &b).await;
        assert_eq!(client_items(&b, "capture").len(), 1);
        assert_eq!(client_items(&b, "calendar_event").len(), 1);
        assert_eq!(b.list_seed_topics().unwrap().len(), 1);

        for kind in [ClientItemKind::Capture, ClientItemKind::CalendarEvent, ClientItemKind::Email] {
            forget_item(&a, kind);
        }
        assert_eq!(a.forget_seed_topic(&seed.key).unwrap(), TombstoneOutcome::Queued);
        topic_rounds(&conn, &group_key, &a, &b).await;

        for kind in ["capture", "calendar_event", "email"] {
            assert!(client_items(&a, kind).is_empty(), "{kind} on A");
            assert!(client_items(&b, kind).is_empty(), "{kind} on B");
        }
        assert!(b.list_seed_topics().unwrap().is_empty(), "the seed is purged on B");
        assert!(b.is_client_item_forgotten(memory::client_forget::FORGOTTEN_SEED_KIND, &seed.key).unwrap());
        // B's full-text index pages keep no deleted term either, as on the deleting device (X6 final review,
        // Major 4): the pull ran the owed index clean-up once after applying.
        assert_eq!(b.fts_pages_naming(&CLIENT.to_lowercase()).unwrap(), 0, "B's index pages still hold the name");
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_CLIENT_ITEM_FORGOTTEN).await, 3);
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_TOPIC_SEED_PURGED).await, 1);
        assert!(
            b.list_outbox_operations().unwrap().iter().all(|o| o.1 != ROW_TYPE_CLIENT_ITEM_FORGOTTEN && o.1 != ROW_TYPE_TOPIC_SEED_PURGED),
            "an applied tombstone is never sent on"
        );
    }

    /// A device that never updates (retired, or left on an old build) must not block a delete: tombstones go at
    /// once, and the count is only what the receipt shows.
    #[tokio::test]
    async fn tombstones_are_sent_without_waiting_for_older_devices() {
        use crate::content::{push_marker, ROW_TYPE_ITEMS_CAPABILITY};
        let (_dir, conn, group_key) = relay_and_store().await;
        push_marker(&conn, &group_key, "device-old", ROW_TYPE_ITEMS_CAPABILITY).await.unwrap();
        let a = topic_store("device-a");
        client_event(&a, "device-a", &format!("{CLIENT} kickoff"));
        let seed = a.create_seed_topic(CLIENT, &[], None).unwrap();
        forget_item(&a, ClientItemKind::CalendarEvent);
        a.forget_seed_topic(&seed.key).unwrap();

        dispatch_outbox_at(&conn, &group_key, &a, "device-a", t0()).await.unwrap();
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_CLIENT_ITEM_FORGOTTEN).await, 1);
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_TOPIC_SEED_PURGED).await, 1);
        assert_eq!(a.pending_tombstone_devices().unwrap(), Some(1), "device-old still needs to update");
    }

    #[tokio::test]
    async fn device_joining_later_does_not_download_deleted_items() {
        let (_dir, conn, group_key) = relay_and_store().await;
        let (a, b) = (topic_store("device-a"), topic_store("device-b"));
        // B's own capture and event, pushed by B.
        client_capture(&b, "device-b");
        client_event(&b, "device-b", &format!("{CLIENT} kickoff"));
        let seed = a.create_seed_topic(CLIENT, &[], None).unwrap();
        topic_rounds(&conn, &group_key, &a, &b).await;
        assert_eq!(client_items(&a, "capture").len(), 1);

        forget_item(&a, ClientItemKind::Capture);
        forget_item(&a, ClientItemKind::CalendarEvent);
        a.forget_seed_topic(&seed.key).unwrap();
        topic_rounds(&conn, &group_key, &a, &b).await;
        assert!(client_items(&b, "capture").is_empty(), "the recorder applied it too");

        let c = topic_store("device-c");
        for _ in 0..2 {
            topic_round(&conn, &group_key, &c, "device-c").await;
        }
        let phone = Store::open_in_memory().unwrap();
        crate::pull_only_with_connection(&conn, &group_key, &phone, "phone", &PullOptions::default()).await.unwrap();
        for (name, store) in [("C", &c), ("phone", &phone)] {
            assert!(client_items(store, "capture").is_empty(), "{name}");
            assert!(client_items(store, "calendar_event").is_empty(), "{name}");
            assert!(store.list_seed_topics().unwrap().is_empty(), "{name}");
        }
    }

    /// B, on an older build, keeps pushing an event A has forgotten (a re-push after an edit): A and any new
    /// device drop it on arrival. Once B updates it applies the tombstone it skipped (the catch-up).
    #[tokio::test]
    async fn older_device_row_cannot_resurrect_after_all_peers_updated() {
        let (_dir, conn, group_key) = relay_and_store().await;
        let (a, b) = (topic_store("device-a"), topic_store("device-b"));
        client_event(&b, "device-b", &format!("{CLIENT} kickoff"));
        old_build_round(&conn, &group_key, &b, "device-b").await;
        topic_round(&conn, &group_key, &a, "device-a").await;
        assert_eq!(client_items(&a, "calendar_event").len(), 1);

        forget_item(&a, ClientItemKind::CalendarEvent);
        topic_round(&conn, &group_key, &a, "device-a").await;
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_CLIENT_ITEM_FORGOTTEN).await, 1, "sent at once");
        assert_eq!(a.pending_tombstone_devices().unwrap(), Some(1));

        // B, still old, pulls the tombstone (skips it) and pushes an edit of the event.
        old_build_round(&conn, &group_key, &b, "device-b").await;
        tick();
        client_event(&b, "device-b", &format!("{CLIENT} kickoff, moved"));
        old_build_round(&conn, &group_key, &b, "device-b").await;
        assert_eq!(client_items(&b, "calendar_event").len(), 1, "the old build kept it");
        topic_round(&conn, &group_key, &a, "device-a").await;
        assert!(client_items(&a, "calendar_event").is_empty(), "an older device's row is dropped");

        // B updates: its first pull reads the skipped tombstone again.
        topic_round(&conn, &group_key, &b, "device-b").await;
        assert!(client_items(&b, "calendar_event").is_empty(), "B applied it once updated");
        topic_round(&conn, &group_key, &a, "device-a").await;
        assert_eq!(a.pending_tombstone_devices().unwrap(), Some(0));
        assert!(client_items(&a, "calendar_event").is_empty());

        let c = topic_store("device-c");
        for _ in 0..2 {
            topic_round(&conn, &group_key, &c, "device-c").await;
        }
        assert!(client_items(&c, "calendar_event").is_empty());
    }

    /// A phone on an older build pulls a tombstone, skips it and moves its cursor past it. After the update its
    /// first pull-only round reads it again and applies it, with no cursor reset.
    #[tokio::test]
    async fn old_puller_catches_up_after_update() {
        let (_dir, conn, group_key) = relay_and_store().await;
        let a = topic_store("device-a");
        client_event(&a, "device-a", &format!("{CLIENT} kickoff"));
        topic_round(&conn, &group_key, &a, "device-a").await;
        let phone = Store::open_in_memory().unwrap();
        let options = PullOptions::default();
        let pull = || crate::pull_only_with_connection(&conn, &group_key, &phone, "phone", &options);
        crate::content::OLD_BUILD_PULLER.with(|c| c.set(true));
        pull().await.unwrap();
        forget_item(&a, ClientItemKind::CalendarEvent);
        topic_round(&conn, &group_key, &a, "device-a").await;
        pull().await.unwrap();
        crate::content::OLD_BUILD_PULLER.with(|c| c.set(false));
        assert_eq!(client_items(&phone, "calendar_event").len(), 1, "the old build skipped the tombstone");
        let cursor = phone.get_sync_cursor("device-a").unwrap();

        pull().await.unwrap();
        assert!(client_items(&phone, "calendar_event").is_empty(), "applied on the first pull after the update");
        assert_eq!(phone.get_sync_cursor("device-a").unwrap(), cursor, "no cursor reset");
    }

    /// This device's own relay rows of what it forgot are deleted, and those of a peer's tombstone naming them.
    #[tokio::test]
    async fn own_relay_copies_of_forgotten_items_are_deleted() {
        let (_dir, conn, group_key) = relay_and_store().await;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_sync_row_entity ON sync_row(device_id, entity_tag, local_seq) \
             WHERE entity_tag IS NOT NULL",
            (),
        )
        .await
        .unwrap();
        let (a, b) = (topic_store("device-a"), topic_store("device-b"));
        client_capture(&a, "device-a");
        client_event(&a, "device-a", &format!("{CLIENT} kickoff"));
        client_capture(&b, "device-b");
        let seed = a.create_seed_topic(CLIENT, &[], None).unwrap();
        topic_rounds(&conn, &group_key, &a, &b).await;
        assert!(relay_rows(&conn, "device-a", ROW_TYPE_ITEMS).await >= 2);
        assert_eq!(relay_rows(&conn, "device-b", ROW_TYPE_ITEMS).await, 1);
        assert!(relay_rows(&conn, "device-a", crate::content::ROW_TYPE_TOPIC_SEED).await >= 1);

        // A forgets its own capture and event, the seed, and B's capture.
        for _ in 0..2 {
            forget_item(&a, ClientItemKind::Capture);
        }
        forget_item(&a, ClientItemKind::CalendarEvent);
        a.forget_seed_topic(&seed.key).unwrap();
        topic_rounds(&conn, &group_key, &a, &b).await;

        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_ITEMS).await, 0, "A's own capture and event rows");
        assert_eq!(relay_rows(&conn, "device-a", crate::content::ROW_TYPE_TOPIC_SEED).await, 0, "the named seed rows");
        assert_eq!(relay_rows(&conn, "device-b", ROW_TYPE_ITEMS).await, 0, "B deleted its own on applying A's tombstone");
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_CLIENT_ITEM_FORGOTTEN).await, 3, "the tombstones stay");
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_TOPIC_SEED_PURGED).await, 1);
        assert!(a.client_relay_cleanup(10).unwrap().is_empty() && b.client_relay_cleanup(10).unwrap().is_empty());
    }

    /// A row leased by a pass that timed out before the delete is not cancelled with the pending ones; it is
    /// acknowledged unsent when it is claimed again.
    #[tokio::test]
    async fn a_leased_row_of_a_forgotten_seed_is_never_sent() {
        let (_dir, conn, group_key) = relay_and_store().await;
        let a = topic_store("device-a");
        let seed = a.create_seed_topic(CLIENT, &[], None).unwrap();
        let t = t0();
        let leased = a
            .claim_due_outbox_batch_except(memory::OUTBOX_CONNECTOR, "dead-worker", t, t + 1, 10, &[])
            .unwrap();
        assert_eq!(leased.len(), 1);
        a.forget_seed_topic(&seed.key).unwrap();
        assert!(states(&a).contains(&"leased".to_string()), "the delete leaves a leased row alone");

        let summary = dispatch_outbox_at(&conn, &group_key, &a, "device-a", t + LEASE_MS + 1).await.unwrap();
        assert_eq!(summary.dropped, 1);
        assert_eq!(relay_rows(&conn, "device-a", crate::content::ROW_TYPE_TOPIC_SEED).await, 0);
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_TOPIC_SEED_PURGED).await, 1);
    }

    #[tokio::test]
    async fn tombstone_row_has_no_client_text() {
        let (_dir, conn, group_key) = relay_and_store().await;
        let a = topic_store("device-a");
        // A calendar UID can carry the client's domain.
        let uid = format!("kickoff-1@{}.com", CLIENT.to_lowercase());
        client_capture(&a, "device-a");
        client_event_with_id(&a, "device-a", &uid, &format!("{CLIENT} kickoff"));
        recent_email(&a, "m1", &format!("{CLIENT} SOW"));
        let seed = a.create_seed_topic(CLIENT, &[format!("{CLIENT} Ltd")], None).unwrap();
        for kind in [ClientItemKind::Capture, ClientItemKind::CalendarEvent, ClientItemKind::Email] {
            forget_item(&a, kind);
        }
        a.forget_seed_topic(&seed.key).unwrap();
        dispatch_outbox_at(&conn, &group_key, &a, "device-a", t0()).await.unwrap();

        let client = CLIENT.to_lowercase();
        let tombstones = relay_plaintexts(&conn, &group_key, "device-a", ROW_TYPE_CLIENT_ITEM_FORGOTTEN).await;
        let purges = relay_plaintexts(&conn, &group_key, "device-a", ROW_TYPE_TOPIC_SEED_PURGED).await;
        assert_eq!((tombstones.len(), purges.len()), (3, 1));
        for plaintext in tombstones.iter().chain(&purges) {
            assert!(!plaintext.to_lowercase().contains(&client), "{plaintext}");
        }
        for plaintext in &tombstones {
            let v: serde_json::Value = serde_json::from_str(plaintext).unwrap();
            let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
            keys.sort();
            assert_eq!(keys, ["forgotten_at_unix_ms", "kind", "opaque_id"]);
        }
        let purge: memory::topics::seeds::TopicSeedPayload = serde_json::from_str(&purges[0]).unwrap();
        assert!(purge.purged && purge.name.is_empty() && purge.aliases.is_empty());
        for (_, row_type, _, payload) in a.list_outbox_operations().unwrap() {
            if row_type == ROW_TYPE_CLIENT_ITEM_FORGOTTEN || row_type == ROW_TYPE_TOPIC_SEED_PURGED {
                assert!(!payload.to_lowercase().contains(&client));
            }
        }
    }

    /// A device that applied a seed's purge drops every later topic row about it: the seed record, its
    /// steps, step reads and page snapshot (here, A still sends them all after B learnt of the purge).
    #[tokio::test]
    async fn a_purged_seed_drops_its_late_topic_rows() {
        use memory::topics::steps::StepAction;
        let (_dir, conn, group_key) = relay_and_store().await;
        let (a, b) = (topic_store("device-a"), topic_store("device-b"));
        let seed = a.create_seed_topic(CLIENT, &[], None).unwrap();
        recent_email(&a, "e1", &format!("{CLIENT} SOW"));
        topic_index(&a);
        read_steps(&a);
        let step = page(&a).you_owe[0].key.clone();
        assert!(a.publish_topic_page_snapshots("device-a", crate::now_unix_ms()).unwrap() > 0);
        b.apply_synced_topic_seed(&memory::topics::seeds::TopicSeedPayload::purged(&seed.key, 1)).unwrap();

        topic_rounds(&conn, &group_key, &a, &b).await;
        assert!(relay_rows(&conn, "device-a", crate::content::ROW_TYPE_TOPIC_STEP).await > 0);
        assert!(relay_rows(&conn, "device-a", crate::content::ROW_TYPE_TOPIC_PAGE_SNAPSHOT).await > 0);
        assert!(b.list_seed_topics().unwrap().is_empty(), "the named record is dropped");
        assert!(!b.apply_step_action(&step, &StepAction::Done).unwrap(), "the step is dropped");
        assert!(b.topic_page_snapshot(&seed.key).unwrap().is_none(), "the page snapshot is dropped");
    }

    /// A peer that updates to a build reading client controls gets every client's settings again: its older
    /// build applied the seed records but dropped the control fields (Nov 19 2026, a PC updated from 0.9.72
    /// never learned an exclusion set before its update). Once, not every pass.
    #[tokio::test]
    async fn a_peer_that_updates_gets_the_client_settings_again() {
        use crate::content::{
            push_marker, ROW_TYPE_CLIENT_CONTROLS_CAPABILITY, ROW_TYPE_ITEMS_CAPABILITY, ROW_TYPE_TOPICS_CAPABILITY,
        };
        use memory::topics::client_controls::ClientControl;
        let (_dir, conn, group_key) = relay_and_store().await;
        let a = topic_store("device-a");
        let seed = a.create_seed_topic("Acme", &[], None).unwrap();
        a.set_client_control(&seed.key, &ClientControl { exclude_hosted: true, keep_months: None }).unwrap();
        // An older build that has topics (0.9.72) but not client controls.
        for marker in [ROW_TYPE_ITEMS_CAPABILITY, ROW_TYPE_TOPICS_CAPABILITY] {
            push_marker(&conn, &group_key, "device-old", marker).await.unwrap();
        }
        let seeds = |s: &Store| s.list_outbox_operations().unwrap().iter().filter(|o| o.1 == "topic_seed").count();

        dispatch_outbox_at(&conn, &group_key, &a, "device-a", t0()).await.unwrap();
        assert_eq!(a.pending_tombstone_devices().unwrap(), Some(1));
        let sent = seeds(&a);
        dispatch_outbox_at(&conn, &group_key, &a, "device-a", t0() + 1).await.unwrap();
        assert_eq!(seeds(&a), sent, "nothing new while the peer has not updated");

        push_marker(&conn, &group_key, "device-old", ROW_TYPE_CLIENT_CONTROLS_CAPABILITY).await.unwrap();
        dispatch_outbox_at(&conn, &group_key, &a, "device-a", t0() + 2).await.unwrap();
        assert_eq!(a.pending_tombstone_devices().unwrap(), Some(0));
        assert_eq!(seeds(&a), sent + 1, "the client's seed record goes out again");
        // Sent on the real clock (the test passes ran on a fixed one, before the record's due time).
        topic_round(&conn, &group_key, &a, "device-a").await;
        let ops: Vec<_> = a.list_outbox_operations().unwrap().into_iter().map(|o| (o.1, o.2)).collect();
        assert!(ops.iter().all(|o| o.1 == "delivered"), "{ops:?}");
        // A topic round queues its own seed records; count from after it.
        let after = seeds(&a);

        dispatch_outbox_at(&conn, &group_key, &a, "device-a", t0() + 3).await.unwrap();
        assert_eq!(seeds(&a), after, "once, not every pass");

        // The updated peer applies it.
        let b = topic_store("device-old");
        topic_round(&conn, &group_key, &b, "device-old").await;
        assert!(b.client_control(&seed.key).unwrap().exclude_hosted);
    }

    #[tokio::test]
    async fn pending_devices_counts_peers_without_marker() {
        use crate::content::{push_marker, ROW_TYPE_CLIENT_CONTROLS_CAPABILITY, ROW_TYPE_ITEMS_CAPABILITY};
        let (_dir, conn, group_key) = relay_and_store().await;
        let a = topic_store("device-a");
        assert_eq!(a.pending_tombstone_devices().unwrap(), None, "unknown until a pass measures it");
        dispatch_outbox_at(&conn, &group_key, &a, "device-a", t0()).await.unwrap();
        assert_eq!(a.pending_tombstone_devices().unwrap(), Some(0), "alone on the relay");

        for old in ["device-old1", "device-old2"] {
            push_marker(&conn, &group_key, old, ROW_TYPE_ITEMS_CAPABILITY).await.unwrap();
        }
        push_marker(&conn, &group_key, "device-new", ROW_TYPE_CLIENT_CONTROLS_CAPABILITY).await.unwrap();
        dispatch_outbox_at(&conn, &group_key, &a, "device-a", t0()).await.unwrap();
        assert_eq!(a.pending_tombstone_devices().unwrap(), Some(2));

        push_marker(&conn, &group_key, "device-old1", ROW_TYPE_CLIENT_CONTROLS_CAPABILITY).await.unwrap();
        dispatch_outbox_at(&conn, &group_key, &a, "device-a", t0()).await.unwrap();
        assert_eq!(a.pending_tombstone_devices().unwrap(), Some(1));
    }

    /// The phone (pull-only, `pull_only_with_connection`) applies client tombstones through the same pull. The
    /// files of what it removes reach the app through the one existing channel: no client item owns a file on a
    /// peer, so the files reported are a forgotten meeting's, from `take_forgotten_meeting_files`. A phone that
    /// pulled while on an older build is `old_puller_catches_up_after_update`.
    #[tokio::test]
    async fn phone_pull_only_applies_client_tombstones_and_reports_files() {
        let (_dir, conn, group_key) = relay_and_store().await;
        let (store_a, a_meeting) = delivered_meeting(&conn, &group_key).await;
        client_event(&store_a, "device-a", &format!("{CLIENT} kickoff"));
        topic_round(&conn, &group_key, &store_a, "device-a").await;
        let phone = Store::open_in_memory().unwrap();
        let options = PullOptions::default();
        let pull = || crate::pull_only_with_connection(&conn, &group_key, &phone, "phone", &options);
        pull().await.unwrap();
        assert_eq!(client_items(&phone, "calendar_event").len(), 1);
        assert!(has_a_meeting(&phone, a_meeting));

        forget_item(&store_a, ClientItemKind::CalendarEvent);
        store_a.forget_meeting(a_meeting).unwrap().unwrap();
        topic_round(&conn, &group_key, &store_a, "device-a").await;
        assert!(
            markers_on_relay(&conn, "phone").await.0.iter().any(|m| m == crate::content::ROW_TYPE_CLIENT_CONTROLS_CAPABILITY),
            "the phone announced the marker"
        );
        assert_eq!(store_a.pending_tombstone_devices().unwrap(), Some(0), "and is counted as current");

        crate::content::take_forgotten_meeting_files();
        pull().await.unwrap();
        assert!(client_items(&phone, "calendar_event").is_empty());
        assert!(phone
            .is_client_item_forgotten("calendar_event", &memory::client_forget::hashed("google_calendar:ev-1"))
            .unwrap());
        assert!(!has_a_meeting(&phone, a_meeting));
        assert_eq!(crate::content::take_forgotten_meeting_files().len(), 1, "the phone's removed files go to the app");
    }

    /// The marker rows `device_id` has on the relay, and how many of its rows are anything else.
    async fn markers_on_relay(conn: &Connection, device_id: &str) -> (Vec<String>, i64) {
        let mut rows = conn
            .query("SELECT row_type, local_seq FROM sync_row WHERE device_id = ?1 ORDER BY row_type", params![device_id])
            .await
            .unwrap();
        let (mut markers, mut other) = (Vec::new(), 0);
        while let Some(row) = rows.next().await.unwrap() {
            let (row_type, seq): (String, i64) = (row.get(0).unwrap(), row.get(1).unwrap());
            if seq == 0 && row_type.ends_with("_capability") {
                markers.push(row_type);
            } else {
                other += 1;
            }
        }
        (markers, other)
    }

    /// The phone's pull-only round announces `client_controls_capability`, so desktops count it in "N devices
    /// still need to update". It also announces every other marker whose rows the shared pull applies, never
    /// skins (the phone skips them): a phone that announced only the client marker would be a peer without the
    /// topic markers, and desktops would hold their topic rows for good. It writes nothing else, and keeps its
    /// own count of devices still to update for its sync line.
    #[tokio::test]
    async fn phone_publishes_capability_marker() {
        use crate::content::{
            push_marker, ROW_TYPE_CLIENT_CONTROLS_CAPABILITY, ROW_TYPE_ITEMS_CAPABILITY, ROW_TYPE_TOPICS_CAPABILITY,
            ROW_TYPE_TOPIC_PAGE_CAPABILITY, ROW_TYPE_TOPIC_SNAPSHOT_CAPABILITY,
        };
        let (_dir, conn, group_key) = relay_and_store().await;
        let a = topic_store("device-a");
        topic_round(&conn, &group_key, &a, "device-a").await;
        let phone = Store::open_in_memory().unwrap();
        let options = PullOptions::default();
        let pull = || crate::pull_only_with_connection(&conn, &group_key, &phone, "phone", &options);
        pull().await.unwrap();

        let (mut markers, other) = markers_on_relay(&conn, "phone").await;
        markers.sort();
        let mut want = vec![
            ROW_TYPE_CLIENT_CONTROLS_CAPABILITY,
            ROW_TYPE_ITEMS_CAPABILITY,
            ROW_TYPE_TOPICS_CAPABILITY,
            ROW_TYPE_TOPIC_PAGE_CAPABILITY,
            ROW_TYPE_TOPIC_SNAPSHOT_CAPABILITY,
        ];
        want.sort();
        assert_eq!(markers, want, "no skins marker: the phone never applies skins");
        assert_eq!(other, 0, "the phone writes no content");
        assert_eq!(phone.pending_tombstone_devices().unwrap(), Some(0), "the phone keeps its own count");

        // The desktop counts the phone and holds nothing back because of it.
        let seed = a.create_seed_topic(CLIENT, &[], None).unwrap();
        topic_round(&conn, &group_key, &a, "device-a").await;
        assert!(relay_rows(&conn, "device-a", crate::content::ROW_TYPE_TOPIC_SEED).await >= 1, "seed rows are not held");
        assert_eq!(a.pending_tombstone_devices().unwrap(), Some(0));
        pull().await.unwrap();
        assert!(phone.list_seed_topics().unwrap().iter().any(|s| s.key == seed.key));

        // A device on an older build: the phone's line says one still needs to update.
        push_marker(&conn, &group_key, "device-old", ROW_TYPE_ITEMS_CAPABILITY).await.unwrap();
        pull().await.unwrap();
        assert_eq!(phone.pending_tombstone_devices().unwrap(), Some(1));
        topic_round(&conn, &group_key, &a, "device-a").await;
        assert_eq!(a.pending_tombstone_devices().unwrap(), Some(1), "the phone is not the one counted");
    }

    /// "Remove this device" (Task 13): a retired desktop that never updates stops being counted in "N devices still
    /// need to update", on every device (phones too), once a desktop publishes its retirement on the relay. It is
    /// only a count: no tombstone was ever held for it, the relay's tombstone rows are the same, and the topic gates
    /// that do hold rows for it are unchanged. Only a device in the list can be removed, never this device, and a
    /// phone's pull-only round never publishes a removal.
    #[tokio::test]
    async fn a_removed_device_stops_being_counted_and_releases_nothing() {
        use crate::content::{push_marker, ROW_TYPE_DEVICE_RETIRED, ROW_TYPE_ITEMS_CAPABILITY, ROW_TYPE_TOPIC_SEED};
        let (_dir, conn, group_key) = relay_and_store().await;
        let a = topic_store("device-a");
        a.create_seed_topic(CLIENT, &[], None).unwrap();
        client_event(&a, "device-a", &format!("{CLIENT} kickoff"));
        // A desktop on an older build, long retired: it has rows on the relay, and no client marker.
        push_marker(&conn, &group_key, "device-old", ROW_TYPE_ITEMS_CAPABILITY).await.unwrap();
        topic_round(&conn, &group_key, &a, "device-a").await;
        forget_item(&a, ClientItemKind::CalendarEvent);
        topic_round(&conn, &group_key, &a, "device-a").await;
        let phone = Store::open_in_memory().unwrap();
        let options = PullOptions::default();
        let pull = || crate::pull_only_with_connection(&conn, &group_key, &phone, "phone", &options);
        pull().await.unwrap();
        topic_round(&conn, &group_key, &a, "device-a").await;
        assert_eq!(a.pending_tombstone_devices().unwrap(), Some(1));
        assert_eq!(a.pending_tombstone_device_ids().unwrap(), Some(vec!["device-old".to_string()]));
        assert_eq!(phone.pending_tombstone_devices().unwrap(), Some(1));
        let tombstones = relay_rows(&conn, "device-a", ROW_TYPE_CLIENT_ITEM_FORGOTTEN).await;
        assert!(tombstones >= 1, "the tombstone was sent at once, not held for the old device");
        let seeds = relay_rows(&conn, "device-a", ROW_TYPE_TOPIC_SEED).await;

        // Refused: this device, a device not in the list.
        assert!(!a.queue_retired_device("device-a").unwrap());
        assert!(!a.queue_retired_device("phone").unwrap(), "a current device is not in the list");
        assert!(!a.queue_retired_device("nope").unwrap());
        // A phone's own store may hold the request, but its pull-only round never publishes it.
        assert!(phone.queue_retired_device("device-old").unwrap());
        pull().await.unwrap();
        assert_eq!(relay_rows(&conn, "device-old", ROW_TYPE_DEVICE_RETIRED).await, 0, "a phone cannot remove a device");
        assert_eq!(phone.pending_tombstone_devices().unwrap(), Some(1), "and its count is the relay's again");

        // The desktop removes it: counted out here at once, and on the relay at its next round.
        assert!(a.queue_retired_device("device-old").unwrap());
        assert_eq!(a.pending_tombstone_devices().unwrap(), Some(0));
        assert_eq!(a.pending_tombstone_device_ids().unwrap(), Some(vec![]));
        topic_round(&conn, &group_key, &a, "device-a").await;
        assert_eq!(relay_rows(&conn, "device-old", ROW_TYPE_DEVICE_RETIRED).await, 1);
        assert!(a.retired_device_queue().unwrap().is_empty(), "published");
        assert_eq!(a.pending_tombstone_devices().unwrap(), Some(0), "the outbox pass leaves it out too");
        pull().await.unwrap();
        assert_eq!(phone.pending_tombstone_devices().unwrap(), Some(0), "the phone leaves it out too");
        let b = topic_store("device-b");
        topic_round(&conn, &group_key, &b, "device-b").await;
        assert_eq!(b.pending_tombstone_devices().unwrap(), Some(0), "so does another desktop");

        // Nothing was released or removed: the tombstones are the same rows, the gates hold what they held.
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_CLIENT_ITEM_FORGOTTEN).await, tombstones);
        assert_eq!(relay_rows(&conn, "device-a", ROW_TYPE_TOPIC_SEED).await, seeds, "the topic gate is unchanged");
        assert_eq!(relay_rows(&conn, "device-old", ROW_TYPE_ITEMS_CAPABILITY).await, 1, "the old device's rows stay");
    }

    /// A phone that was offline while a desktop deleted a client (items and the seed) applies the whole delete
    /// on its next pull. Resuming a catch-up that failed part-way on the phone path is
    /// `a_failing_peer_does_not_block_the_live_pull_and_catch_up_resumes`; an update from an older build is
    /// `old_puller_catches_up_after_update`.
    #[tokio::test]
    async fn phone_offline_during_a_delete_applies_it_later() {
        let (_dir, conn, group_key) = relay_and_store().await;
        let a = topic_store("device-a");
        let seed = a.create_seed_topic(CLIENT, &[], None).unwrap();
        client_event(&a, "device-a", &format!("{CLIENT} kickoff"));
        topic_round(&conn, &group_key, &a, "device-a").await;
        let phone = Store::open_in_memory().unwrap();
        let options = PullOptions::default();
        let pull = || crate::pull_only_with_connection(&conn, &group_key, &phone, "phone", &options);
        pull().await.unwrap();
        assert_eq!(client_items(&phone, "calendar_event").len(), 1);
        assert_eq!(phone.list_seed_topics().unwrap().len(), 1);

        // The phone is offline for several of A's rounds.
        forget_item(&a, ClientItemKind::CalendarEvent);
        a.forget_seed_topic(&seed.key).unwrap();
        for _ in 0..3 {
            topic_round(&conn, &group_key, &a, "device-a").await;
        }
        assert!(
            markers_on_relay(&conn, "phone").await.0.iter().any(|m| m == crate::content::ROW_TYPE_CLIENT_CONTROLS_CAPABILITY),
            "the phone announced the marker"
        );
        assert_eq!(a.pending_tombstone_devices().unwrap(), Some(0), "and is counted as current");

        pull().await.unwrap();
        assert!(client_items(&phone, "calendar_event").is_empty());
        assert!(phone.list_seed_topics().unwrap().is_empty(), "the seed is purged");
        assert!(phone.is_client_item_forgotten("calendar_event", &memory::client_forget::hashed("google_calendar:ev-1")).unwrap());
    }

    /// The catch-up never holds up the live pull: a peer whose catch-up fails is retried next round, and the
    /// database counts as caught up only once every peer is done.
    #[tokio::test]
    async fn a_failing_peer_does_not_block_the_live_pull_and_catch_up_resumes() {
        let (_dir, conn, group_key) = relay_and_store().await;
        let (a, b) = (topic_store("device-a"), topic_store("device-b"));
        client_event_with_id(&a, "device-a", "ev-a#1", &format!("{CLIENT} kickoff A"));
        client_event_with_id(&b, "device-b", "ev-b#1", &format!("{CLIENT} kickoff B"));
        topic_rounds(&conn, &group_key, &a, &b).await;
        let phone = Store::open_in_memory().unwrap();
        let options = PullOptions::default();
        let pull = || crate::pull_only_with_connection(&conn, &group_key, &phone, "phone", &options);
        crate::content::OLD_BUILD_PULLER.with(|c| c.set(true));
        pull().await.unwrap();
        forget_item(&a, ClientItemKind::CalendarEvent);
        forget_item(&b, ClientItemKind::CalendarEvent);
        topic_rounds(&conn, &group_key, &a, &b).await;
        pull().await.unwrap();
        crate::content::OLD_BUILD_PULLER.with(|c| c.set(false));
        assert_eq!(client_items(&phone, "calendar_event").len(), 2, "the old build skipped both tombstones");

        // Updated, but B's catch-up fails this round; a new event from A still arrives.
        client_event_with_id(&a, "device-a", "ev-new#1", &format!("{CLIENT} follow-up"));
        topic_round(&conn, &group_key, &a, "device-a").await;
        crate::content::CATCH_UP_FAILS_FOR.with(|c| *c.borrow_mut() = Some("device-b".into()));
        pull().await.unwrap();
        crate::content::CATCH_UP_FAILS_FOR.with(|c| *c.borrow_mut() = None);
        let titles: Vec<String> = client_items(&phone, "calendar_event").into_iter().filter_map(|r| r.title).collect();
        assert!(titles.iter().any(|t| t.ends_with("follow-up")), "the live pull ran: {titles:?}");
        assert!(!titles.iter().any(|t| t.ends_with("kickoff A")), "A's catch-up applied");
        assert!(titles.iter().any(|t| t.ends_with("kickoff B")), "B's waits");
        assert_eq!(phone.get_meta("sync_tombstone_types_caught_up").unwrap(), None);

        pull().await.unwrap();
        assert!(!client_items(&phone, "calendar_event").iter().any(|r| r.title.as_deref().is_some_and(|t| t.ends_with("kickoff B"))));
        assert_eq!(phone.get_meta("sync_tombstone_types_caught_up").unwrap().as_deref(), Some("1"));
    }

    /// X6 final review, Major 12: a relay so slow that every round runs out of catch-up time on its first page still
    /// finishes: each row handled is saved at once, so every round moves forward and none starts again from 0.
    #[tokio::test]
    async fn a_catch_up_that_always_runs_out_of_time_still_finishes() {
        let (_dir, conn, group_key) = relay_and_store().await;
        let a = topic_store("device-a");
        for i in 0..4 {
            client_event_with_id(&a, "device-a", &format!("ev-{i}#1"), &format!("{CLIENT} kickoff {i}"));
        }
        topic_round(&conn, &group_key, &a, "device-a").await;
        let phone = Store::open_in_memory().unwrap();
        let options = PullOptions::default();
        let pull = || crate::pull_only_with_connection(&conn, &group_key, &phone, "phone", &options);
        crate::content::OLD_BUILD_PULLER.with(|c| c.set(true));
        pull().await.unwrap();
        for _ in 0..4 {
            forget_item(&a, ClientItemKind::CalendarEvent);
        }
        topic_round(&conn, &group_key, &a, "device-a").await;
        pull().await.unwrap();
        crate::content::OLD_BUILD_PULLER.with(|c| c.set(false));
        assert_eq!(client_items(&phone, "calendar_event").len(), 4, "the old build skipped the tombstones");

        // Every row takes longer than the whole budget; one query spans few rows.
        let budget = std::time::Duration::from_millis(40);
        crate::content::CATCH_UP_TEST_PACE.with(|c| c.set(Some((budget, std::time::Duration::from_millis(60)))));
        crate::content::CATCH_UP_TEST_SPAN.with(|c| c.set(Some(3)));
        let progress = || phone.get_meta("sync_tombstone_catch_up:1:device-a").unwrap();
        let mut seen: Vec<Option<String>> = Vec::new();
        let mut rounds = 0;
        while phone.get_meta("sync_tombstone_types_caught_up").unwrap().is_none() {
            rounds += 1;
            assert!(rounds <= 40, "the catch-up never finished: {seen:?}");
            pull().await.unwrap();
            let now = progress();
            if now.as_deref() != Some("done") {
                assert_ne!(seen.last(), Some(&now), "a round moved nothing forward: {seen:?}");
            }
            seen.push(now);
        }
        crate::content::CATCH_UP_TEST_PACE.with(|c| c.set(None));
        crate::content::CATCH_UP_TEST_SPAN.with(|c| c.set(None));
        assert!(rounds > 2, "each round ran out of time: {seen:?}");
        assert!(client_items(&phone, "calendar_event").is_empty(), "every skipped delete applied");
    }

    /// The relay cleanup of a purged seed's steps is bounded by the forget: a step re-filed for the same
    /// source afterwards (same key, another topic) is not deleted with them.
    #[tokio::test]
    async fn a_refiled_step_survives_the_pending_cleanup() {
        let (_dir, conn, group_key) = relay_and_store().await;
        let a = topic_store("device-a");
        let seed = a.create_seed_topic(CLIENT, &[], None).unwrap();
        recent_email(&a, "e1", &format!("{CLIENT} SOW"));
        topic_index(&a);
        read_steps(&a);
        let step = page(&a).you_owe[0].key.clone();
        dispatch_outbox_at(&conn, &group_key, &a, "device-a", t0()).await.unwrap();
        let sent = relay_rows(&conn, "device-a", crate::content::ROW_TYPE_TOPIC_STEP).await;
        assert!(sent > 0);

        a.forget_seed_topic(&seed.key).unwrap();
        let pending = a.client_relay_cleanup(100).unwrap();
        assert!(pending.iter().any(|t| t.0.starts_with("seq:topic_step:")), "{pending:?}");
        // The same source is filed again, on a new topic, before the cleanup runs.
        tick();
        let other = a.create_seed_topic("SOW", &[], None).unwrap();
        topic_index(&a);
        read_steps(&a);
        assert_eq!(page(&a).you_owe[0].key, step, "same step key");
        dispatch_outbox_at(&conn, &group_key, &a, "device-a", t0() + 1).await.unwrap();

        crate::content::delete_forgotten_client_rows(&conn, &a, "device-a").await;
        let steps = relay_plaintexts(&conn, &group_key, "device-a", crate::content::ROW_TYPE_TOPIC_STEP).await;
        assert!(!steps.is_empty(), "the re-filed step survives");
        for plaintext in &steps {
            assert!(plaintext.contains(&other.key), "only the re-filed version is left: {plaintext}");
        }
    }

    /// No raw calendar id (which can carry the client's domain) stays in the cleanup table: not on the deleting
    /// device after its round, not on a peer that never pushed the event, not on a phone that only pulls.
    #[tokio::test]
    async fn no_raw_calendar_id_is_kept_for_cleanup() {
        let (_dir, conn, group_key) = relay_and_store().await;
        let (a, b) = (topic_store("device-a"), topic_store("device-b"));
        let uid = format!("kickoff-1@{}.com#1700000000", CLIENT.to_lowercase());
        client_event_with_id(&a, "device-a", &uid, &format!("{CLIENT} kickoff"));
        topic_rounds(&conn, &group_key, &a, &b).await;
        let phone = Store::open_in_memory().unwrap();
        crate::pull_only_with_connection(&conn, &group_key, &phone, "phone", &PullOptions::default()).await.unwrap();
        assert_eq!(client_items(&phone, "calendar_event").len(), 1);

        forget_item(&a, ClientItemKind::CalendarEvent);
        topic_rounds(&conn, &group_key, &a, &b).await;
        crate::pull_only_with_connection(&conn, &group_key, &phone, "phone", &PullOptions::default()).await.unwrap();
        assert!(client_items(&phone, "calendar_event").is_empty());
        let domain = CLIENT.to_lowercase();
        for (name, store) in [("A", &a), ("B", &b), ("phone", &phone)] {
            for (target, _, _) in store.client_relay_cleanup(100).unwrap() {
                assert!(!target.contains(&domain) && !target.starts_with("raw:"), "{name}: {target}");
            }
        }
        assert!(b.client_relay_cleanup(100).unwrap().is_empty(), "B pushed no row of the event");
        assert!(phone.client_relay_cleanup(100).unwrap().is_empty(), "a phone pushes nothing");
    }

    /// An op already being written when the delete commits: once acknowledged, it is seen to be forgotten, its
    /// relay row is recorded for deletion and its outbox copy (with the seed's name) goes.
    #[tokio::test]
    async fn an_op_delivered_after_the_forget_has_its_relay_row_removed() {
        let (_dir, conn, group_key) = relay_and_store().await;
        let a = topic_store("device-a");
        let seed = a.create_seed_topic(CLIENT, &[], None).unwrap();
        let t = t0();
        let worker = "in-flight";
        let op = a
            .claim_due_outbox_batch_except(memory::OUTBOX_CONNECTOR, worker, t, t + LEASE_MS, 10, &[])
            .unwrap()
            .remove(0);
        // The worker writes the row; the delete commits before it acknowledges.
        write_sync_row(&conn, &group_key, &op, None, None, None, t).await.unwrap();
        a.forget_seed_topic(&seed.key).unwrap();
        assert!(a.acknowledge_outbox_operation(&op.operation_id, worker, "sync_row:x", t).unwrap());
        assert!(a.take_delivery_if_forgotten(&op, t).unwrap());
        assert_eq!(relay_rows(&conn, "device-a", crate::content::ROW_TYPE_TOPIC_SEED).await, 1);

        crate::content::delete_forgotten_client_rows(&conn, &a, "device-a").await;
        assert_eq!(relay_rows(&conn, "device-a", crate::content::ROW_TYPE_TOPIC_SEED).await, 0);
        let client = CLIENT.to_lowercase();
        assert!(a.list_outbox_operations().unwrap().iter().all(|o| !o.3.to_lowercase().contains(&client)));
    }

    /// The same, through the dispatcher itself: the delete commits while the pass is writing the seed's row,
    /// and the pass's acknowledgement sees it and records the row for deletion.
    #[tokio::test]
    async fn a_delete_during_the_dispatch_write_still_removes_the_row() {
        let (_dir, conn, group_key) = relay_and_store().await;
        let a = topic_store("device-a");
        let seed = a.create_seed_topic(CLIENT, &[], None).unwrap();
        FORGET_SEED_DURING_WRITE.with(|c| *c.borrow_mut() = Some(seed.key.clone()));
        dispatch_outbox_at(&conn, &group_key, &a, "device-a", t0()).await.unwrap();
        assert_eq!(relay_rows(&conn, "device-a", crate::content::ROW_TYPE_TOPIC_SEED).await, 1, "written before the delete");

        crate::content::delete_forgotten_client_rows(&conn, &a, "device-a").await;
        assert_eq!(relay_rows(&conn, "device-a", crate::content::ROW_TYPE_TOPIC_SEED).await, 0);
        let client = CLIENT.to_lowercase();
        assert!(a.list_outbox_operations().unwrap().iter().all(|o| !o.3.to_lowercase().contains(&client)));
    }
}
