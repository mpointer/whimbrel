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

use crate::content::{all_peers_items_capable, content_row_aad};
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
    let mut parent_terminal: HashMap<i64, bool> = HashMap::new();
    let mut processed = 0;

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
        for op in batch {
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
            match write_sync_row(conn, group_key, &op, now_unix_ms).await {
                Ok(receipt) => {
                    store
                        .acknowledge_outbox_operation(&op.operation_id, &worker_id, &receipt, now_unix_ms)
                        .map_err(query_error)?;
                    summary.delivered += 1;
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
    Ok(summary)
}

/// `items`-format rows carry the transcript of a meeting that may still be recording;
/// the legacy `unpushed_item_rows` scan applies the same gate at query time.
fn deferral_reason(
    op: &ClaimedOutboxOperation,
    store: &Store,
    parent_terminal: &mut HashMap<i64, bool>,
) -> Option<&'static str> {
    if op.row_type != memory::OUTBOX_ROW_TYPE_ITEMS {
        return None;
    }
    let meeting_id = serde_json::from_str::<serde_json::Value>(&op.payload_json)
        .ok()?
        .get("source_meeting_id")?
        .as_i64()?;
    let terminal = *parent_terminal.entry(meeting_id).or_insert_with(|| {
        store
            .get_meeting(meeting_id)
            .ok()
            .flatten()
            .is_some_and(|m| matches!(m.status, MeetingStatus::Completed | MeetingStatus::Cancelled))
    });
    (!terminal).then_some("parent_meeting_not_terminal")
}

async fn write_sync_row(
    conn: &Connection,
    group_key: &GroupKey,
    op: &ClaimedOutboxOperation,
    now_unix_ms: i64,
) -> Result<String, SyncEngineError> {
    let aad = content_row_aad(&op.device_id, op.local_seq, &op.row_type);
    let ciphertext = sync::encrypt_payload(group_key, op.payload_json.as_bytes(), &aad);
    conn.execute(
        "INSERT INTO sync_row (device_id, local_seq, row_type, ciphertext, created_at_unix_ms) \
         VALUES (?1, ?2, ?3, ?4, ?5) \
         ON CONFLICT (device_id, local_seq, row_type) DO NOTHING",
        params![
            op.device_id.as_str(),
            op.local_seq,
            op.row_type.as_str(),
            STANDARD.encode(&ciphertext).as_str(),
            now_unix_ms
        ],
    )
    .await
    .map_err(|e| SyncEngineError::Query(e.to_string()))?;
    Ok(format!("sync_row:{}:{}:{}", op.device_id, op.local_seq, op.row_type))
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
    }
}
