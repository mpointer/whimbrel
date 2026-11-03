//! Content sync: pushes/pulls this device's rows to/from an account's Turso data-plane
//! database, and applies pulled peer rows directly into the local store
//! (docs/superpowers/plans/2026-07-30-sync-content-data-layer.md).
//! Post-cutover the pushed wire format is `items` rows (`capture`, `transcript` and
//! `calendar_event` kinds) plus `meeting`, which has no `items` representation and so
//! always uses its own row type. The old `capture_item`/`transcript_segment` row types
//! stay supported permanently on the pull side, for legacy data already in flight on the
//! relay and for peers that have not yet cut over
//! (docs/superpowers/specs/2026-08-07-items-sync-migration-design.md).
//! Unlike settings sync, there is no delta returned to the caller — everything is
//! applied inside this module via `memory::Store`'s public API.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use libsql::{params, params::Params, Connection, Value};
use memory::{AppliedTranscriptSegment, Store, SyncableRow};
#[cfg(not(test))]
use std::sync::atomic::{AtomicBool, Ordering};
use sync::GroupKey;

use crate::{now_unix_ms, SyncEngineError};

pub(crate) const ROW_TYPE_CAPTURE_ITEM: &str = "capture_item";
pub(crate) const ROW_TYPE_MEETING: &str = "meeting";
pub(crate) const ROW_TYPE_TRANSCRIPT_SEGMENT: &str = "transcript_segment";
pub(crate) const ROW_TYPE_ITEMS: &str = "items";
pub(crate) const ROW_TYPE_ROUTINE_RUN: &str = "routine_run";
pub(crate) const ROW_TYPE_MEETING_PREP: &str = "meeting_prep";
pub(crate) const ROW_TYPE_PROFILE_SUGGESTION: &str = "profile_suggestion";
pub(crate) const ROW_TYPE_LIVE_ALERT: &str = "live_alert";
pub(crate) const ROW_TYPE_ITEMS_CAPABILITY: &str = "items_capability";
/// Skin sync (docs/superpowers/plans/2026-10-22-skins-windows-and-sync.md, Task 13): a user skin's
/// version, its images in chunks, and the marker a device pushes once it applies them.
pub(crate) const ROW_TYPE_SKIN: &str = memory::OUTBOX_ROW_TYPE_SKIN;
pub(crate) const ROW_TYPE_SKIN_IMAGE_CHUNK: &str = memory::OUTBOX_ROW_TYPE_SKIN_IMAGE_CHUNK;
pub(crate) const ROW_TYPE_SKINS_CAPABILITY: &str = "skins_capability";

/// The relay bytes of image chunks one pass moves from (or to) one peer, so a skin with large images
/// spreads over several rounds instead of holding the store or overrunning `SYNC_TIMEOUT`. Counted on
/// the sealed, base64-encoded row, which is what crosses the network.
pub(crate) const MAX_SKIN_CHUNK_BYTES_PER_PASS: usize = 4 * 1024 * 1024;

/// A `transcript_segment`'s wire payload carries its parent meeting's `local_seq`
/// (docs/superpowers/plans/2026-07-30-sync-content-data-layer.md's FK-resolution
/// design) purely for `pull_content`'s cursor-blocking bound — kept unchanged. FK
/// *resolution* now prefers `source_meeting_id` (the parent meeting's stable
/// `meeting.id` on its source device), falling back to the `meeting_local_seq` +
/// `sync_row_map` path for historical rows pushed before this field existed
/// (docs/superpowers/specs/2026-08-05-sync-meeting-duplication-fix-design.md).
/// `#[serde(flatten)]` keeps `AppliedTranscriptSegment`'s own fields at the top level
/// alongside both.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct TranscriptSegmentPayload {
    pub meeting_local_seq: i64,
    #[serde(default)]
    pub source_meeting_id: Option<i64>,
    #[serde(flatten)]
    pub segment: AppliedTranscriptSegment,
}

/// A `meeting`'s wire payload carries the pushing device's own local `meeting.id` as
/// `source_meeting_id` — a stable identity across all of a meeting's re-pushes, unlike
/// `local_seq` which is deliberately re-stamped on every lifecycle update
/// (docs/superpowers/specs/2026-08-05-sync-meeting-duplication-fix-design.md).
/// `#[serde(default)]` so a historical row pushed before this fix (no
/// `source_meeting_id` field at all) deserializes with `None` rather than failing.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct MeetingPayload {
    #[serde(default)]
    pub source_meeting_id: Option<i64>,
    #[serde(flatten)]
    pub meeting: memory::AppliedMeeting,
}

/// An `items`-format row's wire payload. `source_meeting_id` is `Some` only for
/// `kind == "transcript"` (the pushing device's own `meeting.id`, resolved on the
/// puller against `sync_meeting_map` exactly like `TranscriptSegmentPayload`'s field
/// of the same name). `meeting_local_seq` is the same fallback `TranscriptSegmentPayload`
/// carries, and is load-bearing for exactly one reachable case: the peer applied the
/// parent meeting from a legacy pre-`source_meeting_id` envelope, so `apply_parent_row`
/// never recorded a `sync_meeting_map` entry for it and `source_meeting_id` can never
/// resolve. `backfill_transcript_segments_batch` mints transcript items for an already-
/// completed, already-pushed meeting without re-stamping `meeting.local_seq`, so those
/// items are pushed with the meeting NOT re-pushed alongside them -- without this
/// fallback they would pin the peer's cursor at `item_seq - 1` permanently, stalling
/// the whole pull stream from that peer, not merely dropping the one row.
/// `#[serde(default)]` on both so an items row pushed before this field existed still
/// deserializes.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct ItemPayload {
    #[serde(default)]
    pub source_meeting_id: Option<i64>,
    #[serde(default)]
    pub meeting_local_seq: Option<i64>,
    #[serde(flatten)]
    pub item: memory::AppliedItem,
}

pub async fn ensure_content_schema(conn: &Connection) -> Result<(), SyncEngineError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS sync_row (
            device_id TEXT NOT NULL,
            local_seq INTEGER NOT NULL,
            row_type TEXT NOT NULL,
            ciphertext TEXT NOT NULL,
            created_at_unix_ms INTEGER NOT NULL,
            entity_tag TEXT,
            PRIMARY KEY (device_id, local_seq, row_type)
        );
        CREATE INDEX IF NOT EXISTS idx_sync_row_created ON sync_row(created_at_unix_ms);",
    )
    .await
    .map_err(|e| SyncEngineError::Query(e.to_string()))?;
    // ponytail: one process-wide flag, so an account switch to an unmigrated DB pushes untagged until restart; key it by URL if that matters
    if !ENTITY_TAG_COLUMN_SEEN.get() && ensure_entity_tag_column(conn).await {
        ENTITY_TAG_COLUMN_SEEN.set();
    }
    Ok(())
}

/// A process-wide "seen once, true for good" latch. Under `cfg(test)` it is per thread, so
/// parallel tests (each on its own thread, each with its own relay) cannot see each other's.
pub(crate) struct Latch(
    #[cfg(not(test))] AtomicBool,
    #[cfg(test)] &'static std::thread::LocalKey<std::cell::Cell<bool>>,
);

impl Latch {
    pub(crate) fn get(&self) -> bool {
        #[cfg(not(test))]
        return self.0.load(Ordering::Relaxed);
        #[cfg(test)]
        return self.0.with(|c| c.get());
    }

    pub(crate) fn set(&self) {
        #[cfg(not(test))]
        self.0.store(true, Ordering::Relaxed);
        #[cfg(test)]
        self.0.with(|c| c.set(true));
    }

    #[cfg(test)]
    pub(crate) fn reset(&self) {
        self.0.with(|c| c.set(false));
    }
}

#[cfg(test)]
thread_local! {
    static COLUMN_SEEN_CELL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static INDEX_SEEN_CELL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Set once this process has confirmed `sync_row.entity_tag`. `push_content` runs after
/// `ensure_content_schema`, so while it is false the column is known missing.
#[cfg(not(test))]
pub(crate) static ENTITY_TAG_COLUMN_SEEN: Latch = Latch(AtomicBool::new(false));
#[cfg(test)]
pub(crate) static ENTITY_TAG_COLUMN_SEEN: Latch = Latch(&COLUMN_SEEN_CELL);

/// Set once this process has seen the ops index on the relay.
#[cfg(not(test))]
static COMPACTION_INDEX_SEEN: Latch = Latch(AtomicBool::new(false));
#[cfg(test)]
pub(crate) static COMPACTION_INDEX_SEEN: Latch = Latch(&INDEX_SEEN_CELL);

/// The index ops creates out of band (scripts/sync-row-entity-tag.sql). Clients never create it.
pub(crate) const COMPACTION_INDEX: &str = "idx_sync_row_entity";

/// Whether compaction may run: only once the ops index exists, since without it each
/// DELETE's MAX subquery scans the device's rows (Review Focus 1). A true result is cached
/// for the process; callers check at most once per round, so a false one is rechecked next
/// round and the ops step turns compaction on without a restart. A failed check is false.
pub(crate) async fn compaction_index_ready(conn: &Connection) -> bool {
    if COMPACTION_INDEX_SEEN.get() {
        return true;
    }
    let Ok(mut rows) = conn
        .query(
            "SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = ?1",
            params![COMPACTION_INDEX],
        )
        .await
    else {
        return false;
    };
    let ready = matches!(rows.next().await, Ok(Some(_)));
    if ready {
        COMPACTION_INDEX_SEEN.set();
    }
    ready
}

/// Makes sure an existing relay's `sync_row` has `entity_tag`. Best effort: many devices run
/// this at once, so a "duplicate column" error means another device won the race and counts as
/// success. Any other failure returns `false` (pushes stay untagged) and never fails the sync.
/// Never creates `idx_sync_row_entity`; only the ops SQL does.
pub(crate) async fn ensure_entity_tag_column(conn: &Connection) -> bool {
    if let Ok(mut rows) = conn.query("PRAGMA table_info(sync_row)", ()).await {
        while let Ok(Some(row)) = rows.next().await {
            if row.get::<String>(1).is_ok_and(|name| name == "entity_tag") {
                return true;
            }
        }
    }
    add_entity_tag_column(conn).await
}

async fn add_entity_tag_column(conn: &Connection) -> bool {
    match conn
        .execute("ALTER TABLE sync_row ADD COLUMN entity_tag TEXT", ())
        .await
    {
        Ok(_) => true,
        Err(e) if e.to_string().contains("duplicate column name") => true,
        Err(e) => {
            eprintln!("[sync] entity_tag column not added: {e}");
            false
        }
    }
}

/// Mirrors `field_aad` in `lib.rs` exactly, for the same reason: binds the ciphertext
/// to its own row identity so someone with Turso write access (no group key) can't
/// relabel it under a different `(device_id, local_seq, row_type)`.
pub(crate) fn content_row_aad(device_id: &str, local_seq: i64, row_type: &str) -> Vec<u8> {
    let device_bytes = device_id.as_bytes();
    let type_bytes = row_type.as_bytes();
    let mut aad = Vec::with_capacity(4 + device_bytes.len() + 8 + 4 + type_bytes.len());
    aad.extend_from_slice(&(device_bytes.len() as u32).to_be_bytes());
    aad.extend_from_slice(device_bytes);
    aad.extend_from_slice(&local_seq.to_be_bytes());
    aad.extend_from_slice(&(type_bytes.len() as u32).to_be_bytes());
    aad.extend_from_slice(type_bytes);
    aad
}

pub(crate) fn encode_row(row: &SyncableRow) -> Result<(i64, &'static str, String), SyncEngineError> {
    let (local_seq, row_type, json) = match row {
        SyncableRow::CaptureItem { local_seq, item } => (
            *local_seq,
            ROW_TYPE_CAPTURE_ITEM,
            serde_json::to_string(item),
        ),
        SyncableRow::Meeting {
            local_seq,
            source_meeting_id,
            meeting,
        } => {
            let payload = MeetingPayload {
                source_meeting_id: Some(*source_meeting_id),
                meeting: meeting.clone(),
            };
            (
                *local_seq,
                ROW_TYPE_MEETING,
                serde_json::to_string(&payload),
            )
        }
        SyncableRow::TranscriptSegment {
            local_seq,
            meeting_local_seq,
            source_meeting_id,
            segment,
        } => {
            let payload = TranscriptSegmentPayload {
                meeting_local_seq: *meeting_local_seq,
                source_meeting_id: Some(*source_meeting_id),
                segment: segment.clone(),
            };
            (
                *local_seq,
                ROW_TYPE_TRANSCRIPT_SEGMENT,
                serde_json::to_string(&payload),
            )
        }
        SyncableRow::Item {
            local_seq,
            source_meeting_id,
            meeting_local_seq,
            item,
        } => {
            let payload = ItemPayload {
                source_meeting_id: *source_meeting_id,
                meeting_local_seq: *meeting_local_seq,
                item: item.clone(),
            };
            (*local_seq, ROW_TYPE_ITEMS, serde_json::to_string(&payload))
        }
    };
    let json = json.map_err(|_| SyncEngineError::Crypto)?;
    Ok((local_seq, row_type, json))
}

/// The stable name of the entity a legacy-scan row is a version of, the input to its
/// `entity_tag`. `None` means the row is never tagged, so never compacted. Must equal
/// `outbox_entity_identity` for the same entity (spec D3,
/// docs/superpowers/specs/2026-10-30-sync-row-compaction-design.md).
pub(crate) fn entity_identity(row: &SyncableRow) -> Option<String> {
    match row {
        SyncableRow::Meeting { source_meeting_id, .. } => Some(format!("{ROW_TYPE_MEETING}:{source_meeting_id}")),
        // `source` is a colon-free token, so a `:` inside `external_id` cannot collide.
        SyncableRow::Item { item, .. } if item.kind == "calendar_event" => match (&item.source, &item.external_id) {
            (Some(source), Some(external_id)) => Some(format!("{ROW_TYPE_ITEMS}:calendar_event:{source}:{external_id}")),
            _ => None,
        },
        _ => None,
    }
}

/// The outbox twin of `entity_identity`: each tagged type already enqueues its
/// stable id as `aggregate_id`.
pub(crate) fn outbox_entity_identity(op: &memory::ClaimedOutboxOperation) -> Option<String> {
    match op.row_type.as_str() {
        ROW_TYPE_MEETING | ROW_TYPE_ROUTINE_RUN | ROW_TYPE_MEETING_PREP | ROW_TYPE_SKIN | ROW_TYPE_SKIN_IMAGE_CHUNK => {
            Some(format!("{}:{}", op.row_type, op.aggregate_id))
        }
        _ => None,
    }
}

/// The `entity_tag` for an identity from `entity_identity` or `outbox_entity_identity`. Both
/// push paths make tags only through here, with `Store::sync_entity_epoch`.
/// The epoch keeps a wiped database's reused ids (same Keychain `device_id`, restarted
/// `local_seq` and meeting ids) from matching an earlier database's rows on the relay.
pub(crate) fn tag_for(group_key: &GroupKey, device_id: &str, epoch: &str, identity: &str) -> String {
    sync::entity_tag(group_key, device_id, &format!("{epoch}:{identity}"))
}

/// Unconditionally (re-)announces this device's Slice-3 capability every sync round —
/// a fixed sentinel at `local_seq = 0` (real content local_seq values start at `1`,
/// confirmed via `next_local_seq`'s `RETURNING next_seq - 1` starting from `next_seq`'s
/// default of `1`), so it never collides with real content and is idempotent under
/// `ON CONFLICT DO NOTHING` (docs/superpowers/specs/2026-08-07-items-sync-migration-design.md
/// point 5 — decoupling capability announcement from actual content push is what avoids
/// the symmetric bootstrapping deadlock a content-based readiness check would have).
/// `local_seq = 0` also keeps the marker out of every peer's `fetch_rows_since` window
/// (which selects `local_seq > cursor`, cursor starting at 0), so it is never pulled.
async fn push_items_capability_marker(
    conn: &Connection,
    group_key: &GroupKey,
    device_id: &str,
) -> Result<(), SyncEngineError> {
    push_marker(conn, group_key, device_id, ROW_TYPE_ITEMS_CAPABILITY).await?;
    // Every device that pushes applies skin rows (the phone only pulls, and skips them), so it says
    // so the same way: a peer seeing this for the first time re-pushes every skin it has.
    push_marker(conn, group_key, device_id, ROW_TYPE_SKINS_CAPABILITY).await
}

/// One capability marker at `local_seq` 0, idempotent (see `push_items_capability_marker`).
async fn push_marker(
    conn: &Connection,
    group_key: &GroupKey,
    device_id: &str,
    marker: &str,
) -> Result<(), SyncEngineError> {
    let aad = content_row_aad(device_id, 0, marker);
    let ciphertext = sync::encrypt_payload(group_key, b"1", &aad);
    conn.execute(
        "INSERT INTO sync_row (device_id, local_seq, row_type, ciphertext, created_at_unix_ms) \
         VALUES (?1, 0, ?2, ?3, ?4) \
         ON CONFLICT (device_id, local_seq, row_type) DO NOTHING",
        params![
            device_id,
            marker,
            STANDARD.encode(&ciphertext).as_str(),
            now_unix_ms()
        ],
    )
    .await
    .map_err(|e| SyncEngineError::Query(e.to_string()))?;
    Ok(())
}

/// True once every OTHER device_id present in `sync_row` has announced its own
/// `items_capability` marker — the flag-day cutover gate. Vacuously true when there
/// are no other devices yet (a brand-new account, or the first device ever to sync).
///
/// Accepted limitation: this gate only sees devices that already have rows in `sync_row`.
/// A genuinely NEW device joining the account later on a pre-Slice-3 build is invisible
/// here, and would silently and permanently drop every `items`-format push it later pulls
/// (the design spec's own note covers the downgraded-existing-device case; this is the
/// unengineered-around sibling). See "Known limitation" in
/// docs/superpowers/specs/2026-08-07-items-sync-migration-design.md.
pub(crate) async fn all_peers_items_capable(
    conn: &Connection,
    own_device_id: &str,
) -> Result<bool, SyncEngineError> {
    for peer in distinct_peer_device_ids(conn, own_device_id).await? {
        let mut rows = conn
            .query(PEER_MARKER_SQL, params![peer, ROW_TYPE_ITEMS_CAPABILITY])
            .await
            .map_err(|e| SyncEngineError::Query(e.to_string()))?;
        let has_marker = rows
            .next()
            .await
            .map_err(|e| SyncEngineError::Query(e.to_string()))?
            .is_some();
        if !has_marker {
            return Ok(false);
        }
    }
    Ok(true)
}

/// The smallest `device_id` after `?1`, by primary-key seek. Turso bills rows read, and the
/// old `SELECT DISTINCT device_id ... WHERE device_id != ?` read every row of `sync_row`
/// several times per sync round; this reads one row per device.
const NEXT_PEER_SQL: &str =
    "SELECT device_id FROM sync_row WHERE device_id > ?1 ORDER BY device_id LIMIT 1";

/// A peer's `items_capability` marker, always written at `local_seq` 0
/// (`push_items_capability_marker`): a primary-key point lookup.
const PEER_MARKER_SQL: &str =
    "SELECT 1 FROM sync_row WHERE device_id = ?1 AND local_seq = 0 AND row_type = ?2";

/// Caps how many rows one `push_content` call sends, the push-side twin of
/// `MAX_ROWS_PER_PULL_PASS`. Before this cap, push sent *every* unpushed row one network
/// round trip at a time and saved the push cursor only after the whole loop. With a
/// large backlog, `SYNC_TIMEOUT` cancelled the pass before the cursor was saved, so the
/// next round restarted from the same place and timed out again, forever. Because
/// `sync_content` pushes before it pulls, a device stuck like this never pulled
/// anything either. This was confirmed live on 2026-09-10: a Mac with a 2,180-row
/// backlog failed every round at exactly 30 s and never pulled a new peer's rows.
/// Bounded passes plus per-chunk cursor saves drain any backlog across the 60 s cycle.
const MAX_ROWS_PER_PUSH_PASS: usize = 500;

/// Rows per multi-row INSERT: one network round trip each, not one per row. Six
/// parameters per row keeps a chunk (300) under SQLite's 999-variable floor. It also
/// caps a chunk's distinct tags at 50, the most `compact_entity_tags` takes.
const PUSH_CHUNK_ROWS: usize = 50;

/// One multi-row INSERT of a chunk of encoded `(local_seq, row_type, ciphertext, tag)`
/// rows. `with_tag` adds the `entity_tag` column (NULL for untagged rows).
async fn insert_push_chunk(
    conn: &Connection,
    device_id: &str,
    now: i64,
    rows: &[(i64, &'static str, String, Option<String>)],
    with_tag: bool,
) -> Result<u64, libsql::Error> {
    let width = if with_tag { 6 } else { 5 };
    let mut placeholders = Vec::with_capacity(rows.len());
    let mut values: Vec<Value> = Vec::with_capacity(rows.len() * width);
    for (i, (local_seq, row_type, ciphertext, tag)) in rows.iter().enumerate() {
        let p = i * width;
        let slots: Vec<String> = (p + 1..=p + width).map(|n| format!("?{n}")).collect();
        placeholders.push(format!("({})", slots.join(", ")));
        values.extend([
            Value::Text(device_id.to_string()),
            Value::Integer(*local_seq),
            Value::Text(row_type.to_string()),
            Value::Text(ciphertext.clone()),
            Value::Integer(now),
        ]);
        if with_tag {
            values.push(tag.clone().map_or(Value::Null, Value::Text));
        }
    }
    let columns = if with_tag {
        "device_id, local_seq, row_type, ciphertext, created_at_unix_ms, entity_tag"
    } else {
        "device_id, local_seq, row_type, ciphertext, created_at_unix_ms"
    };
    let sql = format!(
        "INSERT INTO sync_row ({columns}) VALUES {} \
         ON CONFLICT (device_id, local_seq, row_type) DO NOTHING",
        placeholders.join(", ")
    );
    conn.execute(&sql, Params::Positional(values)).await
}

fn row_local_seq(row: &SyncableRow) -> i64 {
    match row {
        SyncableRow::CaptureItem { local_seq, .. }
        | SyncableRow::Meeting { local_seq, .. }
        | SyncableRow::TranscriptSegment { local_seq, .. }
        | SyncableRow::Item { local_seq, .. } => *local_seq,
    }
}

/// This pass's rows: oldest `local_seq` first, at most `cap`. Ascending order is what
/// makes saving the cursor after each chunk safe: every row at or below the saved
/// value has been sent.
fn plan_push_pass(mut rows: Vec<SyncableRow>, cap: usize) -> Vec<SyncableRow> {
    rows.sort_by_key(row_local_seq);
    rows.truncate(cap);
    rows
}

/// Pushes this device's unpushed rows (per `store.get_sync_push_cursor()`), at most
/// `MAX_ROWS_PER_PUSH_PASS` per call and oldest first, encrypting each and inserting in
/// chunks into `sync_row`. The push cursor is saved after each chunk. Idempotent *within* a wire format:
/// re-running after a partial-cycle failure re-pushes already-pushed rows harmlessly
/// (`ON CONFLICT DO NOTHING`) rather than erroring. Not idempotent ACROSS the cutover:
/// `row_type` is part of `sync_row`'s primary key, so replaying a pre-cutover `local_seq`
/// range post-cutover writes `items` rows alongside the old-format rows already there
/// rather than deduping against them (only reachable via a restored backup or a manual
/// push-cursor reset, not by any normal retry).
pub async fn push_content(
    conn: &Connection,
    group_key: &GroupKey,
    store: &Store,
    device_id: &str,
) -> Result<(), SyncEngineError> {
    push_items_capability_marker(conn, group_key, device_id).await?;

    let last_pushed = store
        .get_sync_push_cursor()
        .map_err(|e| SyncEngineError::Query(e.to_string()))?;
    let cutover_ready = all_peers_items_capable(conn, device_id).await?;

    // `meeting` has no `items` representation, so it is pushed in its own format on
    // both sides of the cutover.
    let mut rows = store
        .unpushed_meeting_rows(device_id, last_pushed)
        .map_err(|e| SyncEngineError::Query(e.to_string()))?;
    if cutover_ready {
        rows.extend(
            store
                .unpushed_item_rows(device_id, last_pushed)
                .map_err(|e| SyncEngineError::Query(e.to_string()))?,
        );
    } else {
        // Pre-cutover: push capture_item/transcript_segment in the old format instead
        // of items. unpushed_rows() also re-selects meeting rows, so drop those here
        // to avoid double-pushing what unpushed_meeting_rows already added above.
        let legacy_rows = store
            .unpushed_rows(device_id, last_pushed)
            .map_err(|e| SyncEngineError::Query(e.to_string()))?;
        rows.extend(
            legacy_rows
                .into_iter()
                .filter(|r| !matches!(r, SyncableRow::Meeting { .. })),
        );
    }

    let rows = plan_push_pass(rows, MAX_ROWS_PER_PUSH_PASS);
    let mut highest = last_pushed;
    // Known missing (ensure_content_schema ran this round and could not confirm it): push
    // untagged rather than send a tagged INSERT that is bound to fail.
    // An unreadable epoch also means untagged: tagging must never fail the round.
    let mut epoch = if ENTITY_TAG_COLUMN_SEEN.get() {
        store
            .sync_entity_epoch()
            .inspect_err(|e| eprintln!("[sync] pushing untagged, no entity epoch: {e}"))
            .ok()
    } else {
        None
    };
    let mut compaction = CompactionBudget::default();
    for chunk in rows.chunks(PUSH_CHUNK_ROWS) {
        let now = now_unix_ms();
        let mut encoded = Vec::with_capacity(chunk.len());
        let mut tags: Vec<String> = Vec::new();
        let mut chunk_highest = highest;
        for row in chunk {
            let (local_seq, row_type, payload_json) = encode_row(row)?;
            let aad = content_row_aad(device_id, local_seq, row_type);
            let ciphertext = sync::encrypt_payload(group_key, payload_json.as_bytes(), &aad);
            let tag = epoch
                .as_deref()
                .zip(entity_identity(row))
                .map(|(epoch, id)| tag_for(group_key, device_id, epoch, &id));
            if let Some(t) = &tag {
                if !tags.contains(t) {
                    tags.push(t.clone());
                }
            }
            encoded.push((local_seq, row_type, STANDARD.encode(&ciphertext), tag));
            chunk_highest = chunk_highest.max(local_seq);
        }
        let inserted = if tags.is_empty() {
            insert_push_chunk(conn, device_id, now, &encoded, false).await
        } else {
            match insert_push_chunk(conn, device_id, now, &encoded, true).await {
                Err(e) if is_missing_entity_tag_column(&e.to_string()) => {
                    // A relay that predates `entity_tag`: push untagged, compact nothing, and
                    // stop tagging for the rest of this call (as the outbox does).
                    #[cfg(test)]
                    FAILED_TAGGED_INSERTS.with(|c| c.set(c.get() + 1));
                    tags.clear();
                    epoch = None;
                    insert_push_chunk(conn, device_id, now, &encoded, false).await
                }
                other => other,
            }
        };
        inserted.map_err(|e| SyncEngineError::Query(e.to_string()))?;
        // Before the cursor save (spec D8.1): a pass cut short here re-sends the chunk
        // (a no-op INSERT) and the DELETE runs then. Rows stay tagged without the index;
        // only the DELETE waits for it.
        compaction.compact(conn, device_id, &tags).await;
        // Save progress per chunk: a pass cut short by SYNC_TIMEOUT keeps what it sent.
        if chunk_highest > highest {
            store
                .set_sync_push_cursor(chunk_highest)
                .map_err(|e| SyncEngineError::Query(e.to_string()))?;
            highest = chunk_highest;
        }
    }
    Ok(())
}

#[derive(Clone)]
struct RowEnvelope {
    local_seq: i64,
    row_type: String,
    ciphertext_base64: String,
}

async fn distinct_peer_device_ids(
    conn: &Connection,
    own_device_id: &str,
) -> Result<Vec<String>, SyncEngineError> {
    // Hop from device to device with `NEXT_PEER_SQL`. The own device is skipped by moving
    // the cursor past it, never by a `!=` filter, which would read all of its rows.
    let mut ids = Vec::new();
    let mut after = String::new();
    loop {
        let mut rows = conn
            .query(NEXT_PEER_SQL, params![after.as_str()])
            .await
            .map_err(|e| SyncEngineError::Query(e.to_string()))?;
        let Some(row) = rows
            .next()
            .await
            .map_err(|e| SyncEngineError::Query(e.to_string()))?
        else {
            return Ok(ids);
        };
        after = row
            .get::<String>(0)
            .map_err(|e| SyncEngineError::Query(e.to_string()))?;
        if after != own_device_id {
            ids.push(after.clone());
        }
    }
}

/// Caps how many peer rows one `pull_content` call applies. The apply loop below calls
/// `Store`'s synchronous rusqlite methods directly inside this `async fn` with no
/// `.await` between rows -- unlike `push_content`'s per-row loop (which awaits a real
/// network write to Turso each iteration and so yields naturally), a large unbounded
/// batch here never yields control back to the executor. That means `SYNC_TIMEOUT`'s
/// `tokio::time::timeout` wrapper can never actually preempt it: cooperative timeout
/// cancellation only works at an `.await` point, and this loop has none. On a freshly
/// enrolled device pulling weeks of a peer's history, that made sync never complete --
/// confirmed live via `content-sync` logs showing `syncNow: starting` with no matching
/// completion line, across every attempt, including after a relaunch. Bounding the pull
/// keeps each pass well under the timeout and lets the existing cursor-advancement logic
/// make durable, resumable progress across the periodic 60s sync cycle instead.
const MAX_ROWS_PER_PULL_PASS: i64 = 500;

/// How long one pass may spend applying a peer's rows, on top of the row cap. Once the
/// embedding model exists, applying a row embeds it (`apply_synced_*` call
/// `Store::embed`), which costs ~0.1-1 s per row on a laptop CPU, so 500 rows can hold
/// the store for many minutes. Every shell serializes store access behind this call (the
/// Windows store actor, the Mac's `accessQueue`), so everything else in the app waits:
/// seen live on Windows while a Mac pushed a two-week backlog. The pass stops at the
/// budget and saves the cursor at the last row it processed; the rest follows on later
/// rounds. Per peer, so one peer's backlog can't starve another's pulls.
const PULL_PEER_TIME_BUDGET: std::time::Duration = std::time::Duration::from_secs(3);

/// Most pending-dependency rows fetched by one batched query (keeps the SQL well under
/// SQLite's bound-parameter limit).
const PENDING_LOOKUP_CHUNK: usize = 200;

// Counts `sync_row` reads on the pull path, so tests can assert the pending pass is batched.
#[cfg(test)]
thread_local! {
    static ROW_QUERIES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn count_row_query() {
    #[cfg(test)]
    ROW_QUERIES.with(|c| c.set(c.get() + 1));
}

// Counts compaction DELETEs, so tests can assert a push issued none.
#[cfg(test)]
thread_local! {
    static COMPACTION_DELETES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn take_compaction_deletes() -> usize {
    COMPACTION_DELETES.with(|c| c.replace(0))
}

/// Deletes this device's superseded versions of `tags` (spec D1): every row of a tag below
/// that tag's MAX `local_seq`, so the newest version always survives and a late older
/// version is removed by its own push. Scoped to `device_id`, so another device's rows are
/// never touched; NULL tags never match `IN`. Best effort: an error is logged, never
/// returned, and the next push of that entity heals it. Callers pass at most 50 tags.
/// Push paths go through `CompactionBudget` instead, which adds the index check and the
/// per-round time budget; this single unbudgeted call is for tests.
#[cfg(test)]
pub(crate) async fn compact_entity_tags(conn: &Connection, device_id: &str, tags: &[String]) -> u64 {
    compact_within(conn, device_id, tags, COMPACTION_TIMEOUT).await.unwrap_or(0)
}

/// One pass's compaction (the legacy push and the outbox each make their own): checks the
/// ops index at most once, and spends at most `COMPACTION_TIMEOUT` in total on DELETEs,
/// counted from the first one, so the index check runs before the clock starts. After a
/// timeout or once that time is used up, the rest of the pass skips compaction. Rows are
/// still tagged, and the next push of each entity catches up (spec D1). A round runs both
/// passes, so up to 10 s in all, within the 30 s `SYNC_TIMEOUT`. Without this, ten 50-row
/// chunks at 5 s each could outlast `SYNC_TIMEOUT` and starve pull and the outbox.
#[derive(Default)]
pub(crate) struct CompactionBudget {
    index_ready: Option<bool>,
    deadline: Option<tokio::time::Instant>,
}

impl CompactionBudget {
    pub(crate) async fn compact(&mut self, conn: &Connection, device_id: &str, tags: &[String]) -> u64 {
        if tags.is_empty() {
            return 0;
        }
        if self.index_ready.is_none() {
            self.index_ready = Some(compaction_index_ready(conn).await);
        }
        if self.index_ready != Some(true) {
            return 0;
        }
        // A timeout uses up exactly what was left, so it also ends compaction for the round.
        let now = tokio::time::Instant::now();
        let left = self.deadline.get_or_insert(now + COMPACTION_TIMEOUT).saturating_duration_since(now);
        if left.is_zero() {
            return 0;
        }
        compact_within(conn, device_id, tags, left).await.unwrap_or(0)
    }
}

/// One compaction DELETE under `limit`. `None` means it timed out.
async fn compact_within(
    conn: &Connection,
    device_id: &str,
    tags: &[String],
    limit: std::time::Duration,
) -> Option<u64> {
    if tags.is_empty() {
        return Some(0);
    }
    #[cfg(test)]
    COMPACTION_DELETES.with(|c| c.set(c.get() + 1));
    let mut values = vec![Value::Text(device_id.to_string())];
    values.extend(tags.iter().map(|t| Value::Text(t.clone())));
    let sql = compaction_delete_sql(tags.len());
    let delete = async {
        // A local relay answers synchronously, so a test cannot make a real DELETE slow.
        #[cfg(test)]
        if let Some(delay) = INJECTED_DELETE_DELAY.with(|d| d.get()) {
            tokio::time::sleep(delay).await;
        }
        conn.execute(&sql, Params::Positional(values)).await
    };
    bounded_compaction(delete, limit).await
}

#[cfg(test)]
thread_local! {
    static FAILED_TAGGED_INSERTS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    pub(crate) static INJECTED_DELETE_DELAY: std::cell::Cell<Option<std::time::Duration>> = const { std::cell::Cell::new(None) };
}

/// The most one round spends on compaction DELETEs, and the limit on a single one.
const COMPACTION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// The spec D1 statement for `tag_count` tags: `?1` is the device, `?2..` the tags.
fn compaction_delete_sql(tag_count: usize) -> String {
    let placeholders: Vec<String> = (2..tag_count + 2).map(|i| format!("?{i}")).collect();
    format!(
        "DELETE FROM sync_row WHERE device_id = ?1 AND entity_tag IN ({}) \
         AND local_seq < (SELECT MAX(s.local_seq) FROM sync_row s \
         WHERE s.device_id = sync_row.device_id AND s.entity_tag = sync_row.entity_tag)",
        placeholders.join(", ")
    )
}

/// Runs a compaction DELETE under `limit`. An error or a timeout is logged and counts as
/// skipped; neither ever fails the round. `None` means it timed out.
///
/// Dropping a timed-out remote `execute` is safe for later statements on the same
/// connection (libsql 0.9.30, `hrana/stream.rs`): every autocommit statement is sent with
/// a Close and has no baton, the baton only changes after a response arrives, and the
/// stream lock is a `tokio::sync::Mutex` whose guard is released on drop.
async fn bounded_compaction(
    delete: impl std::future::Future<Output = Result<u64, libsql::Error>>,
    limit: std::time::Duration,
) -> Option<u64> {
    match tokio::time::timeout(limit, delete).await {
        Ok(Ok(n)) => Some(n),
        Ok(Err(e)) => {
            eprintln!("[sync] compaction skipped: {e}");
            Some(0)
        }
        Err(_) => {
            eprintln!("[sync] compaction skipped for this round: no reply within {limit:?}");
            None
        }
    }
}

/// True when an INSERT failed because the relay's `sync_row` predates `entity_tag`
/// ("table sync_row has no column named entity_tag").
pub(crate) fn is_missing_entity_tag_column(error: &str) -> bool {
    error.contains("no column named entity_tag")
}

const FETCH_ROWS_NO_SKINS_SQL: &str =
    "SELECT local_seq, row_type, ciphertext FROM sync_row WHERE device_id = ?1 AND local_seq > ?2 AND row_type NOT IN ('skin', 'skin_image_chunk') ORDER BY local_seq ASC LIMIT ?3";
const FETCH_ROWS_SQL: &str =
    "SELECT local_seq, row_type, ciphertext FROM sync_row WHERE device_id = ?1 AND local_seq > ?2 ORDER BY local_seq ASC LIMIT ?3";

/// `skip_skins` leaves out skin rows at the query, so a puller that never applies them (the phone)
/// never downloads their images.
async fn fetch_rows_since(
    conn: &Connection,
    peer_device_id: &str,
    since_local_seq: i64,
    limit: i64,
    skip_skins: bool,
) -> Result<Vec<RowEnvelope>, SyncEngineError> {
    let sql = if skip_skins { FETCH_ROWS_NO_SKINS_SQL } else { FETCH_ROWS_SQL };
    let rows = conn
        .query(sql, params![peer_device_id, since_local_seq, limit])
        .await
        .map_err(|e| SyncEngineError::Query(e.to_string()))?;
    read_envelopes(rows).await
}

async fn read_envelopes(mut rows: libsql::Rows) -> Result<Vec<RowEnvelope>, SyncEngineError> {
    count_row_query();
    let mut envelopes = Vec::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| SyncEngineError::Query(e.to_string()))?
    {
        envelopes.push(RowEnvelope {
            local_seq: row
                .get(0)
                .map_err(|e| SyncEngineError::Query(e.to_string()))?,
            row_type: row
                .get(1)
                .map_err(|e| SyncEngineError::Query(e.to_string()))?,
            ciphertext_base64: row
                .get(2)
                .map_err(|e| SyncEngineError::Query(e.to_string()))?,
        });
    }
    Ok(envelopes)
}

/// One query for these exact `(local_seq, row_type)` rows of a peer, used to retry
/// pending-dependency entries (and their missing meetings) outside the cursor-bounded
/// window: those rows may sit well behind the peer's fetch horizon, since the cursor no
/// longer waits for them. Callers chunk `keys` to at most `PENDING_LOOKUP_CHUNK`. A row
/// that isn't on the relay is simply absent from the result.
async fn fetch_rows_by_key(
    conn: &Connection,
    peer_device_id: &str,
    keys: &[(i64, String)],
) -> Result<Vec<RowEnvelope>, SyncEngineError> {
    let placeholders = (0..keys.len()).map(|i| format!("?{}", i + 2)).collect::<Vec<_>>().join(", ");
    let sql = format!(
        "SELECT local_seq, row_type, ciphertext FROM sync_row WHERE device_id = ?1 AND local_seq IN ({placeholders})"
    );
    let mut values = vec![Value::Text(peer_device_id.to_string())];
    values.extend(keys.iter().map(|(seq, _)| Value::Integer(*seq)));
    let rows = conn
        .query(&sql, Params::Positional(values))
        .await
        .map_err(|e| SyncEngineError::Query(e.to_string()))?;
    let mut envelopes = read_envelopes(rows).await?;
    // The key is (local_seq, row_type); drop a same-seq row of another type.
    envelopes.retain(|env| keys.iter().any(|(seq, ty)| *seq == env.local_seq && *ty == env.row_type));
    Ok(envelopes)
}

/// Decrypts one row's ciphertext. Returns `None` (never an `Err`) on any failure —
/// wrong key, tampered ciphertext, or a relabeled row identity all fail AEAD
/// verification identically, and per this plan's Global Constraints a single bad row
/// must never abort the rest of a pull cycle.
fn decrypt_row(group_key: &GroupKey, peer_device_id: &str, env: &RowEnvelope) -> Option<Vec<u8>> {
    let ciphertext = STANDARD.decode(&env.ciphertext_base64).ok()?;
    let aad = content_row_aad(peer_device_id, env.local_seq, &env.row_type);
    sync::decrypt_payload(group_key, &ciphertext, &aad).ok()
}

/// Applies a decrypted `capture_item` or `meeting` row (never `transcript_segment` —
/// that goes through the dependent pass below) and records its `sync_row_map` entry.
/// Idempotent: the caller must check `resolve_sync_row_map` first and skip calling
/// this at all if already applied.
fn apply_parent_row(
    store: &Store,
    peer_device_id: &str,
    local_seq: i64,
    row_type: &str,
    plaintext: &[u8],
) -> Result<(), SyncEngineError> {
    let local_id = match row_type {
        ROW_TYPE_CAPTURE_ITEM => {
            let item: memory::AppliedCaptureItem =
                serde_json::from_slice(plaintext).map_err(|_| SyncEngineError::Crypto)?;
            store
                .apply_synced_capture_item(peer_device_id, local_seq, &item)
                .map_err(|e| SyncEngineError::Query(e.to_string()))?
        }
        ROW_TYPE_MEETING => {
            let payload: MeetingPayload =
                serde_json::from_slice(plaintext).map_err(|_| SyncEngineError::Crypto)?;
            // For a legacy row with no source_meeting_id (pushed before Task 1), fall
            // back to sync_row_map keyed by this exact local_seq — this is what makes
            // a re-fetch of the SAME already-applied local_seq idempotent (e.g. a
            // cursor rollback caused by a still-blocked segment) even without a stable
            // source_meeting_id to key on. A legacy row re-pushed under a genuinely
            // different local_seq still duplicates — there's no stable identity in
            // that payload to resolve it by; that gap is what source_meeting_id (Task
            // 1) exists to close going forward.
            let existing_local_id = match payload.source_meeting_id {
                Some(source_id) => store
                    .resolve_sync_meeting_map(peer_device_id, source_id)
                    .map_err(|e| SyncEngineError::Query(e.to_string()))?,
                None => match store
                    .resolve_sync_row_map(peer_device_id, local_seq, ROW_TYPE_MEETING)
                    .map_err(|e| SyncEngineError::Query(e.to_string()))?
                {
                    Some(id) => Some(id),
                    // No stamp (e.g. cleared): match an identical meeting rather than
                    // insert another copy of a legacy one.
                    None => store
                        .find_synced_meeting_by_content(peer_device_id, &payload.meeting)
                        .map_err(|e| SyncEngineError::Query(e.to_string()))?,
                },
            };
            let local_id = store
                .apply_synced_meeting(
                    peer_device_id,
                    local_seq,
                    existing_local_id,
                    &payload.meeting,
                )
                .map_err(|e| SyncEngineError::Query(e.to_string()))?;
            if let Some(source_id) = payload.source_meeting_id {
                store
                    .record_sync_meeting_map(peer_device_id, source_id, local_id)
                    .map_err(|e| SyncEngineError::Query(e.to_string()))?;
            }
            local_id
        }
        ROW_TYPE_ITEMS => {
            let payload: ItemPayload =
                serde_json::from_slice(plaintext).map_err(|_| SyncEngineError::Crypto)?;
            if payload.item.kind == "transcript" {
                // Handled by pull_content's dependent pass instead (needs FK resolution
                // that may not be ready yet) -- reaching this arm for a transcript-kind
                // items row means pull_content's classification routed it incorrectly.
                // Applying it here with thread_id = None would orphan it permanently.
                return Ok(());
            }
            store
                .apply_synced_item(peer_device_id, local_seq, None, &payload.item)
                .map_err(|e| SyncEngineError::Query(e.to_string()))?
        }
        ROW_TYPE_ROUTINE_RUN => {
            // A payload this build cannot decode (a status or shape from a newer app, or a
            // corrupt row) is skipped, not an error: one bad row must never stop a peer's later
            // rows from applying.
            let Ok(run) = serde_json::from_slice::<memory::AppliedRoutineRun>(plaintext) else {
                return Ok(());
            };
            // The origin's own run id is the identity, so a re-push or an updated version
            // replaces the row in place; a template this build does not know is ignored.
            match store
                .apply_synced_routine_run(peer_device_id, &run)
                .map_err(|e| SyncEngineError::Query(e.to_string()))?
            {
                Some(id) => id,
                None => return Ok(()),
            }
        }
        ROW_TYPE_MEETING_PREP => {
            // Skipped, not an error, when this build cannot decode it. It has no local row id, so
            // there is no `sync_row_map` entry to record: return before it.
            let Ok(row) = serde_json::from_slice::<memory::prep::MeetingPrepRow>(plaintext) else {
                return Ok(());
            };
            store
                .apply_synced_meeting_prep(&row)
                .map_err(|e| SyncEngineError::Query(e.to_string()))?;
            return Ok(());
        }
        ROW_TYPE_SKIN => {
            // Kept in the store, not the skins folder: `core/skins` writes it once its images are whole.
            let Ok(payload) = serde_json::from_slice::<memory::skin_sync::SkinSyncPayload>(plaintext) else {
                return Ok(());
            };
            store
                .apply_synced_skin(peer_device_id, &payload)
                .map_err(|e| SyncEngineError::Query(e.to_string()))?;
            return Ok(());
        }
        ROW_TYPE_SKIN_IMAGE_CHUNK => {
            let Ok(chunk) = serde_json::from_slice::<memory::skin_sync::SkinImageChunk>(plaintext) else {
                return Ok(());
            };
            store
                .apply_synced_skin_chunk(&chunk)
                .map_err(|e| SyncEngineError::Query(e.to_string()))?;
            return Ok(());
        }
        ROW_TYPE_PROFILE_SUGGESTION => {
            let Ok(suggestion) = serde_json::from_slice::<memory::profile::AppliedProfileSuggestion>(plaintext) else {
                return Ok(());
            };
            store
                .apply_synced_profile_suggestion(peer_device_id, &suggestion)
                .map_err(|e| SyncEngineError::Query(e.to_string()))?;
            return Ok(());
        }
        ROW_TYPE_LIVE_ALERT => {
            // Upserts on (origin, source_id); its meeting is resolved (or back-filled later)
            // through `sync_meeting_map`, so there is no `sync_row_map` entry to record.
            let Ok(alert) = serde_json::from_slice::<memory::live_alert::AppliedLiveAlert>(plaintext) else {
                return Ok(());
            };
            store
                .apply_synced_live_alert(peer_device_id, &alert)
                .map_err(|e| SyncEngineError::Query(e.to_string()))?;
            return Ok(());
        }
        _ => return Ok(()), // unknown row_type (including items_capability): forward-compatibility no-op, not an error
    };
    // Unconditional for BOTH branches above (insert and update) — required so a
    // historical transcript_segment's meeting_local_seq fallback lookup keeps
    // resolving for every snapshot a meeting has ever had, not just its latest one.
    store
        .record_sync_row_map(peer_device_id, local_seq, row_type, local_id)
        .map_err(|e| SyncEngineError::Query(e.to_string()))
}

/// Discovers every peer device with rows in `sync_row` and applies everything new
/// since this device's own `sync_cursor` for that peer, per the two-pass FK
/// resolution this task's module doc describes.
///
/// The peer's cursor always advances to the highest local_seq fetched this pass, even
/// when some fetched rows are still waiting on an unresolved dependency (typically a
/// transcript segment whose parent meeting hasn't arrived yet). Those rows are tracked
/// separately in `sync_pending_dependency` and retried by direct point-lookup on every
/// call, independent of the cursor -- previously the cursor itself was pinned just
/// below the earliest unresolved row, which combined with `MAX_ROWS_PER_PULL_PASS`
/// meant a single dependency that could never resolve (a corrupted parent row, or one
/// that will never be re-pushed) permanently capped how much of a peer's history this
/// device could ever see, well past just the one blocked row.
pub async fn pull_content(
    conn: &Connection,
    group_key: &GroupKey,
    store: &Store,
    own_device_id: &str,
) -> Result<(), SyncEngineError> {
    pull_content_with_options(conn, group_key, store, own_device_id, &PullOptions::default()).await
}

/// Knobs a puller can set on [`pull_content_with_options`]. `Default` is exactly what
/// [`pull_content`] does (Mac and Windows).
#[derive(Debug, Clone, Default)]
pub struct PullOptions {
    /// Skip screen-capture rows (legacy `capture_item`, and `items` of kind `capture`)
    /// whose own `ts_unix_ms` is older than now minus this many ms. Meetings,
    /// transcripts, calendar events and every other row type are never windowed.
    /// `None` applies everything.
    pub capture_window_ms: Option<i64>,
}

/// [`pull_content_with_options`] for a pull-only peer (the phone, which has no skins): skin rows are
/// left out at the query, so neither the skins nor their images are fetched or applied.
pub(crate) async fn pull_content_without_skins(
    conn: &Connection,
    group_key: &GroupKey,
    store: &Store,
    own_device_id: &str,
    options: &PullOptions,
) -> Result<(), SyncEngineError> {
    pull_content_with_budget(conn, group_key, store, own_device_id, PULL_PEER_TIME_BUDGET, options, true)
        .await
        .map(|more| MORE_TO_PULL.store(more, std::sync::atomic::Ordering::Relaxed))
}

/// [`pull_content`] with [`PullOptions`] (the phone's capture window).
pub async fn pull_content_with_options(
    conn: &Connection,
    group_key: &GroupKey,
    store: &Store,
    own_device_id: &str,
    options: &PullOptions,
) -> Result<(), SyncEngineError> {
    pull_content_with_budget(conn, group_key, store, own_device_id, PULL_PEER_TIME_BUDGET, options, false)
        .await
        .map(|more| MORE_TO_PULL.store(more, std::sync::atomic::Ordering::Relaxed))
}

/// Set by each pull: it stopped with rows still waiting, so the shell can come back soon.
static MORE_TO_PULL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether the last pull left rows on the relay.
pub fn more_to_pull() -> bool {
    MORE_TO_PULL.load(std::sync::atomic::Ordering::Relaxed)
}

/// [`pull_content`] with the per-peer time budget as a parameter. Each peer's pass
/// always processes at least one row, so a zero budget (tests) still makes progress.
async fn pull_content_with_budget(
    conn: &Connection,
    group_key: &GroupKey,
    store: &Store,
    own_device_id: &str,
    budget: std::time::Duration,
    options: &PullOptions,
    skip_skins: bool,
) -> Result<bool, SyncEngineError> {
    let mut more = false;
    // Capture window (the phone's storage budget). Decided AFTER decrypting, on the
    // item's own `ts_unix_ms`: the remote row's `created_at_unix_ms` is push time, not
    // capture time, so a backlog pushed today would all look new, and an `items` row's
    // kind (capture vs transcript vs calendar_event) is only known after decrypting
    // anyway. A skipped row still counts toward `processed_through`, so the cursor moves
    // past it and it is never re-read. Capture rows have no dependents (only transcripts
    // reference a parent, and that parent is a meeting), so skipping one blocks nothing.
    // ponytail: the ciphertext of a skipped row is still downloaded; dropping it at the
    // query (CASE on created_at for legacy capture_item rows) is the upgrade if relay
    // bandwidth on the phone matters.
    let capture_cutoff = options.capture_window_ms.map(|w| now_unix_ms() - w);
    let outside_window = |ts_unix_ms: i64| capture_cutoff.is_some_and(|cutoff| ts_unix_ms < cutoff);
    for peer in distinct_peer_device_ids(conn, own_device_id).await? {
        let cursor = store
            .get_sync_cursor(&peer)
            .map_err(|e| SyncEngineError::Query(e.to_string()))?;
        let known_skin_peer = store
            .is_skin_peer(&peer)
            .map_err(|e| SyncEngineError::Query(e.to_string()))?;
        if !skip_skins && !known_skin_peer {
            let mut marker = conn
                .query(PEER_MARKER_SQL, params![peer.as_str(), ROW_TYPE_SKINS_CAPABILITY])
                .await
                .map_err(|e| SyncEngineError::Query(e.to_string()))?;
            if marker
                .next()
                .await
                .map_err(|e| SyncEngineError::Query(e.to_string()))?
                .is_some()
            {
                store
                    .record_skin_peer(&peer)
                    .map_err(|e| SyncEngineError::Query(e.to_string()))?;
            }
        }
        // Snapshot before this pass records its own new pending rows: those were just
        // tried, so retrying them in the same round would only repeat the lookup.
        let pending_before = store
            .list_sync_pending_dependencies(&peer)
            .map_err(|e| SyncEngineError::Query(e.to_string()))?;
        let envelopes =
            fetch_rows_since(conn, &peer, cursor, MAX_ROWS_PER_PULL_PASS, skip_skins).await?;
        more |= envelopes.len() as i64 >= MAX_ROWS_PER_PULL_PASS;
        let peer_started = std::time::Instant::now();
        let mut chunk_bytes = 0usize;
        // The highest local_seq this pass handled (applied, skipped, or deferred to the
        // dependency pass): where the cursor goes, since the pass can stop early.
        let mut processed_through = cursor;

        // Two kinds of "needs the parent meeting resolved first" rows now exist:
        // legacy transcript_segment envelopes, and items-format envelopes whose
        // decrypted kind is "transcript". Both go into `segments`, already decoded
        // and already dedupe-checked against sync_row_map -- the second loop below
        // only needs to attempt FK resolution and apply. An items envelope's `kind`
        // can't be known without decrypting it, so unlike the previous version both
        // envelope shapes are now decrypted here rather than lazily in the second pass.
        let mut segments: Vec<(RowEnvelope, PendingSegment)> = Vec::new();
        for env in &envelopes {
            // PULL_PEER_TIME_BUDGET: stop once it's spent, having handled at least one row.
            if processed_through > cursor && peer_started.elapsed() >= budget {
                more = true;
                break;
            }
            if env.row_type == ROW_TYPE_SKIN_IMAGE_CHUNK {
                // The image budget: the rest of a large image follows next round (the skin stays held).
                if processed_through > cursor && chunk_bytes >= MAX_SKIN_CHUNK_BYTES_PER_PASS {
                    more = true;
                    break;
                }
                chunk_bytes += env.ciphertext_base64.len();
            }
            processed_through = env.local_seq;
            if env.row_type == ROW_TYPE_TRANSCRIPT_SEGMENT {
                if store
                    .resolve_sync_row_map(&peer, env.local_seq, &env.row_type)
                    .map_err(|e| SyncEngineError::Query(e.to_string()))?
                    .is_some()
                {
                    continue; // already applied
                }
                let Some(plaintext) = decrypt_row(group_key, &peer, env) else {
                    continue;
                };
                let Ok(decoded) = serde_json::from_slice::<TranscriptSegmentPayload>(&plaintext)
                else {
                    continue;
                };
                segments.push((
                    env.clone(),
                    PendingSegment::Legacy {
                        source_meeting_id: decoded.source_meeting_id,
                        meeting_local_seq: decoded.meeting_local_seq,
                        segment: decoded.segment,
                    },
                ));
                continue;
            }
            if env.row_type == ROW_TYPE_ITEMS {
                if store
                    .resolve_sync_row_map(&peer, env.local_seq, &env.row_type)
                    .map_err(|e| SyncEngineError::Query(e.to_string()))?
                    .is_some()
                {
                    continue; // already applied in a prior cycle
                }
                let Some(plaintext) = decrypt_row(group_key, &peer, env) else {
                    continue;
                };
                let Ok(decoded) = serde_json::from_slice::<ItemPayload>(&plaintext) else {
                    continue;
                };
                if decoded.item.kind == "capture" && outside_window(decoded.item.ts_unix_ms) {
                    continue;
                }
                if decoded.item.kind == "transcript" {
                    // Never apply here: apply_synced_item would stamp a NULL thread_id,
                    // permanently orphaning the row. Defer to the resolution pass below.
                    segments.push((
                        env.clone(),
                        PendingSegment::Item {
                            source_meeting_id: decoded.source_meeting_id,
                            meeting_local_seq: decoded.meeting_local_seq,
                            item: decoded.item,
                        },
                    ));
                } else {
                    let local_id = store
                        .apply_synced_item(&peer, env.local_seq, None, &decoded.item)
                        .map_err(|e| SyncEngineError::Query(e.to_string()))?;
                    store
                        .record_sync_row_map(&peer, env.local_seq, &env.row_type, local_id)
                        .map_err(|e| SyncEngineError::Query(e.to_string()))?;
                }
                continue;
            }
            // Meeting rows skip this check entirely and go straight through
            // apply_parent_row's upsert: every meeting push carries a brand-new
            // local_seq by design (docs/superpowers/specs/2026-08-05-sync-meeting-duplication-fix-design.md),
            // so this lookup would always miss for them anyway. capture_item is
            // unaffected by this bug and keeps its existing skip-guard.
            if env.row_type == ROW_TYPE_CAPTURE_ITEM
                && store
                    .resolve_sync_row_map(&peer, env.local_seq, &env.row_type)
                    .map_err(|e| SyncEngineError::Query(e.to_string()))?
                    .is_some()
            {
                continue; // already applied in a prior cycle
            }
            let Some(plaintext) = decrypt_row(group_key, &peer, env) else {
                continue;
            };
            if env.row_type == ROW_TYPE_CAPTURE_ITEM
                && capture_cutoff.is_some()
                && serde_json::from_slice::<memory::AppliedCaptureItem>(&plaintext)
                    .is_ok_and(|item| outside_window(item.ts_unix_ms))
            {
                continue;
            }
            apply_parent_row(store, &peer, env.local_seq, &env.row_type, &plaintext)?;
        }

        resolve_segments(conn, group_key, store, &peer, segments).await?;

        // Advance to the highest row this pass handled: every fetched row, unless the time
        // budget stopped it early, in which case the rest are fetched again next pass.
        // Unresolved dependencies are tracked in sync_pending_dependency and retried
        // independently above, so a row that may never resolve no longer caps how far
        // this device can ever pull.
        let new_cursor = processed_through;
        store
            .set_sync_cursor(&peer, new_cursor)
            .map_err(|e| SyncEngineError::Query(e.to_string()))?;

        // Retry every row still waiting on a dependency from an earlier pass. These can sit
        // well behind the cursor's window (the cursor no longer waits for them), so they're
        // looked up by key, in batches, AFTER the cursor is saved: the forward pull never
        // waits on this pass, and a meeting fetched above already resolves its segments
        // here. The pass has its own time budget, so a large pending set (227 entries hit
        // SYNC_TIMEOUT on the phone, one round trip each) can never stall the round; what
        // it doesn't reach stays pending for the next round. Always one chunk, like the
        // forward pass's one row, so a zero budget still progresses.
        let pending_started = std::time::Instant::now();
        let mut retry = Vec::new();
        for (dep_local_seq, dep_row_type) in pending_before {
            if store
                .resolve_sync_row_map(&peer, dep_local_seq, &dep_row_type)
                .map_err(|e| SyncEngineError::Query(e.to_string()))?
                .is_some()
            {
                // Already resolved by some other path -- just clear the stale entry.
                store
                    .delete_sync_pending_dependency(&peer, dep_local_seq, &dep_row_type)
                    .map_err(|e| SyncEngineError::Query(e.to_string()))?;
                continue;
            }
            retry.push((dep_local_seq, dep_row_type));
        }
        for (i, chunk) in retry.chunks(PENDING_LOOKUP_CHUNK).enumerate() {
            // ponytail: the budget is checked per chunk, so one chunk's applies run to the end
            // (fine without embedding; with it, check the budget per row in resolve_segments).
            if i > 0 && pending_started.elapsed() >= budget {
                break;
            }
            // A vanished row is absent and an undecodable one is dropped here: both stay
            // pending and are retried next round rather than lost.
            let pending = fetch_rows_by_key(conn, &peer, chunk)
                .await?
                .into_iter()
                .filter_map(|env| decode_pending_segment(group_key, &peer, &env).map(|p| (env, p)))
                .collect();
            resolve_segments(conn, group_key, store, &peer, pending).await?;
        }
    }
    Ok(more)
}

/// A row that can't be applied until its parent meeting resolves. Both variants
/// carry the `meeting_local_seq` fallback for a peer whose meeting was applied from
/// a legacy pre-`source_meeting_id` envelope (no `sync_meeting_map` entry exists to
/// resolve against). It is `Option` on `Item` only because an items row pushed
/// before that field existed deserializes to `None`.
// AppliedItem is much wider than AppliedTranscriptSegment. Boxing would buy nothing
// here: this Vec holds one entry per deferred row of a single pull cycle, and the
// payloads are moved into it exactly once.
#[allow(clippy::large_enum_variant)]
enum PendingSegment {
    Legacy {
        source_meeting_id: Option<i64>,
        meeting_local_seq: i64,
        segment: memory::AppliedTranscriptSegment,
    },
    Item {
        source_meeting_id: Option<i64>,
        meeting_local_seq: Option<i64>,
        item: memory::AppliedItem,
    },
}

/// Decrypts and decodes one row known (or suspected) to need FK resolution before
/// it can be applied. Used only by the pending-dependency retry pass below -- the
/// fresh-envelope scan already inlines its own decode (it also needs to apply
/// non-transcript items immediately, which this helper deliberately does not do).
fn decode_pending_segment(
    group_key: &GroupKey,
    peer_device_id: &str,
    env: &RowEnvelope,
) -> Option<PendingSegment> {
    let plaintext = decrypt_row(group_key, peer_device_id, env)?;
    match env.row_type.as_str() {
        ROW_TYPE_TRANSCRIPT_SEGMENT => {
            let decoded: TranscriptSegmentPayload = serde_json::from_slice(&plaintext).ok()?;
            Some(PendingSegment::Legacy {
                source_meeting_id: decoded.source_meeting_id,
                meeting_local_seq: decoded.meeting_local_seq,
                segment: decoded.segment,
            })
        }
        ROW_TYPE_ITEMS => {
            let decoded: ItemPayload = serde_json::from_slice(&plaintext).ok()?;
            if decoded.item.kind != "transcript" {
                return None;
            }
            Some(PendingSegment::Item {
                source_meeting_id: decoded.source_meeting_id,
                meeting_local_seq: decoded.meeting_local_seq,
                item: decoded.item,
            })
        }
        _ => None,
    }
}

/// Resolves each segment's parent meeting and applies it, or records it in
/// `sync_pending_dependency` for a later round. A meeting that isn't applied locally but
/// whose exact `meeting_local_seq` is known is fetched by key first (batched): it may never
/// have been in a fetch window at all, e.g. the cursor advanced past its local_seq while
/// the meeting was still in flight to the relay.
async fn resolve_segments(
    conn: &Connection,
    group_key: &GroupKey,
    store: &Store,
    peer: &str,
    segments: Vec<(RowEnvelope, PendingSegment)>,
) -> Result<(), SyncEngineError> {
    let fallback = |pending: &PendingSegment| match pending {
        PendingSegment::Legacy {
            source_meeting_id,
            meeting_local_seq,
            ..
        } => (*source_meeting_id, Some(*meeting_local_seq)),
        PendingSegment::Item {
            source_meeting_id,
            meeting_local_seq,
            ..
        } => (*source_meeting_id, *meeting_local_seq),
    };
    // Prefer the stable source_meeting_id (via sync_meeting_map). Fall back to the legacy
    // meeting_local_seq + sync_row_map lookup for historical rows pushed before
    // source_meeting_id existed, or if the primary lookup misses.
    let resolve = |source_meeting_id: Option<i64>, fallback_local_seq: Option<i64>| -> Result<Option<i64>, SyncEngineError> {
        if let Some(source_id) = source_meeting_id {
            if let Some(id) = store
                .resolve_sync_meeting_map(peer, source_id)
                .map_err(|e| SyncEngineError::Query(e.to_string()))?
            {
                return Ok(Some(id));
            }
        }
        match fallback_local_seq {
            Some(seq) => store
                .resolve_sync_row_map(peer, seq, ROW_TYPE_MEETING)
                .map_err(|e| SyncEngineError::Query(e.to_string())),
            None => Ok(None),
        }
    };

    let mut missing_meetings: Vec<(i64, String)> = Vec::new();
    for (_, pending) in &segments {
        let (source_meeting_id, fallback_local_seq) = fallback(pending);
        if let Some(seq) = fallback_local_seq {
            if resolve(source_meeting_id, fallback_local_seq)?.is_none()
                && !missing_meetings.iter().any(|(s, _)| *s == seq)
            {
                missing_meetings.push((seq, ROW_TYPE_MEETING.to_string()));
            }
        }
    }
    for chunk in missing_meetings.chunks(PENDING_LOOKUP_CHUNK) {
        for meeting_env in fetch_rows_by_key(conn, peer, chunk).await? {
            if let Some(plaintext) = decrypt_row(group_key, peer, &meeting_env) {
                apply_parent_row(store, peer, meeting_env.local_seq, &meeting_env.row_type, &plaintext)?;
            }
        }
    }

    for (env, pending) in &segments {
        let (source_meeting_id, fallback_local_seq) = fallback(pending);

        let resolved_meeting_id = resolve(source_meeting_id, fallback_local_seq)?;
        match resolved_meeting_id {
            Some(local_meeting_id) => {
                let local_id = match pending {
                    PendingSegment::Legacy { segment, .. } => store
                        .apply_synced_transcript_segment(
                            env.local_seq,
                            local_meeting_id,
                            segment,
                        )
                        .map_err(|e| SyncEngineError::Query(e.to_string()))?,
                    // The ONLY call site that may pass a transcript-kind item to
                    // apply_synced_item, and it is unreachable unless the meeting
                    // resolved -- the Some arm is what makes thread_id non-NULL.
                    PendingSegment::Item { item, .. } => store
                        .apply_synced_item(peer, env.local_seq, Some(local_meeting_id), item)
                        .map_err(|e| SyncEngineError::Query(e.to_string()))?,
                };
                store
                    .record_sync_row_map(peer, env.local_seq, &env.row_type, local_id)
                    .map_err(|e| SyncEngineError::Query(e.to_string()))?;
                // A no-op if this row wasn't pending (the common case: it resolved
                // on its first pass and never got here via list_sync_pending_dependencies).
                store
                    .delete_sync_pending_dependency(peer, env.local_seq, &env.row_type)
                    .map_err(|e| SyncEngineError::Query(e.to_string()))?;
            }
            None => {
                // Still unresolved -- track it for point-lookup retry on every future
                // pass. Unlike the old cursor-blocking design, this no longer holds
                // back anything else: the cursor advances past it, and the retry
                // pass looks it up in a batch rather than a frozen pull stream.
                store
                    .record_sync_pending_dependency(peer, env.local_seq, &env.row_type)
                    .map_err(|e| SyncEngineError::Query(e.to_string()))?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use memory::AppliedCaptureItem;

    /// The `sync_row` table as relays created it before `entity_tag` existed.
    pub(crate) const OLD_SYNC_ROW_DDL: &str =
        "CREATE TABLE sync_row (device_id TEXT NOT NULL, local_seq INTEGER NOT NULL, \
    row_type TEXT NOT NULL, ciphertext TEXT NOT NULL, created_at_unix_ms INTEGER NOT NULL, \
    PRIMARY KEY (device_id, local_seq, row_type))";

    async fn has_entity_tag(conn: &Connection) -> bool {
        let mut rows = conn.query("PRAGMA table_info(sync_row)", ()).await.unwrap();
        while let Some(row) = rows.next().await.unwrap() {
            if row.get::<String>(1).unwrap() == "entity_tag" {
                return true;
            }
        }
        false
    }

    #[tokio::test]
    async fn ensure_entity_tag_column_adds_the_column_to_an_old_table() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("relay.db")).await;
        conn.execute(OLD_SYNC_ROW_DDL, ()).await.unwrap();
        assert!(ensure_entity_tag_column(&conn).await);
        assert!(ensure_entity_tag_column(&conn).await);
        assert!(has_entity_tag(&conn).await);
        assert_eq!(
            single_i64(
                &conn,
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'idx_sync_row_entity'"
            )
            .await,
            0,
            "the client never creates the index"
        );
    }

    #[tokio::test]
    async fn a_duplicate_column_error_counts_as_success() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("relay.db")).await;
        conn.execute(OLD_SYNC_ROW_DDL, ()).await.unwrap();
        conn.execute("ALTER TABLE sync_row ADD COLUMN entity_tag TEXT", ())
            .await
            .unwrap();
        // The race loser: its check saw no column, then another device's ALTER landed first.
        assert!(add_entity_tag_column(&conn).await);
    }

    /// Shaped like `simultaneous_opens_of_one_database_all_succeed` (#455). Local libsql may
    /// serialize the two ALTERs, so this asserts convergence, not that the duplicate error fired.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn simultaneous_schema_setups_all_converge() {
        for round in 0..25 {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("relay.db");
            let mut conns = Vec::new();
            for _ in 0..2 {
                let db = libsql::Builder::new_local(&path).build().await.unwrap();
                let conn = db.connect().unwrap();
                conn.busy_timeout(std::time::Duration::from_secs(5))
                    .unwrap();
                conns.push((db, conn));
            }
            conns[0].1.execute(OLD_SYNC_ROW_DDL, ()).await.unwrap();
            let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
            let handles: Vec<_> = conns
                .iter()
                .map(|(_, conn)| {
                    let (conn, barrier) = (conn.clone(), barrier.clone());
                    tokio::spawn(async move {
                        barrier.wait().await;
                        ensure_entity_tag_column(&conn).await
                    })
                })
                .collect();
            let mut results = Vec::new();
            for h in handles {
                results.push(h.await.unwrap());
            }
            assert_eq!((results[0], results[1]), (true, true), "round {round}");
            assert!(has_entity_tag(&conns[0].1).await, "round {round}");
        }
    }

    #[tokio::test]
    async fn ensure_entity_tag_column_reports_false_when_the_alter_fails() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("relay.db")).await;
        conn.execute("CREATE VIEW sync_row AS SELECT 1 AS device_id", ())
            .await
            .unwrap();
        assert!(!ensure_entity_tag_column(&conn).await);
    }

    #[tokio::test]
    async fn a_new_relay_gets_the_column_from_create_table() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("relay.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        assert!(has_entity_tag(&conn).await);
    }

    async fn local_connection(path: &std::path::Path) -> Connection {
        let db = libsql::Builder::new_local(path).build().await.unwrap();
        db.connect().unwrap()
    }

    #[tokio::test]
    async fn push_content_inserts_rows_and_advances_the_push_cursor() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("shared.db");
        let group_key = GroupKey::generate();
        let conn = local_connection(&db_path).await;
        ensure_content_schema(&conn).await.unwrap();

        let store = Store::open_in_memory().unwrap();
        store
            .insert_meeting(&memory::NewMeeting {
                device_id: "device-a".to_string(),
                start_unix_ms: 1000,
                end_unix_ms: None,
                title: "Standup".to_string(),
                attendees: vec![],
                trigger: memory::MeetingTrigger::Calendar,
                audio_path: None,
                status: memory::MeetingStatus::Recording,
            })
            .map(|id| store.complete_meeting(id, 2000, None).unwrap())
            .unwrap();

        push_content(&conn, &group_key, &store, "device-a")
            .await
            .unwrap();

        // Excludes the items_capability sentinel, which push_content now always writes.
        let mut rows = conn
            .query("SELECT COUNT(*) FROM sync_row WHERE device_id = 'device-a' AND local_seq != 0", ())
            .await
            .unwrap();
        let count: i64 = rows.next().await.unwrap().unwrap().get(0).unwrap();
        assert_eq!(count, 1);
        assert!(store.get_sync_push_cursor().unwrap() > 0);
    }

    async fn count_pushed(conn: &Connection, device_id: &str) -> (i64, i64) {
        let mut rows = conn
            .query(
                "SELECT COUNT(*), COALESCE(MAX(local_seq), 0) FROM sync_row WHERE device_id = ?1 AND local_seq != 0",
                params![device_id],
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        (row.get(0).unwrap(), row.get(1).unwrap())
    }

    /// The 2026-09-10 Mac failure: a large backlog must drain across bounded passes, the
    /// push cursor must always equal the highest row actually sent, and the batched
    /// multi-row INSERT must produce rows a peer pulls and decrypts like before.
    #[tokio::test]
    async fn a_large_backlog_is_pushed_in_bounded_resumable_passes() {
        let dir = tempfile::tempdir().unwrap();
        let group_key = GroupKey::generate();
        let conn = local_connection(&dir.path().join("shared.db")).await;
        ensure_content_schema(&conn).await.unwrap();

        let store = Store::open_in_memory().unwrap();
        let backlog = MAX_ROWS_PER_PUSH_PASS * 2 + 200;
        for i in 0..backlog {
            store
                .insert_capture_item(&memory::NewCaptureItem {
                    device_id: "device-a".to_string(),
                    ts_unix_ms: 1_000 + i as i64,
                    app_bundle_id: "com.example.app".to_string(),
                    window_title: format!("Window {i}"),
                    url: None,
                    text: format!("capture number {i} with some distinct words"),
                    hash: i as i64 + 1,
                    source: "ax".to_string(),
                })
                .unwrap()
                .expect("distinct hashes are never deduped");
        }

        let mut expected = 0;
        for pass in 1..=3 {
            push_content(&conn, &group_key, &store, "device-a").await.unwrap();
            expected = (expected + MAX_ROWS_PER_PUSH_PASS).min(backlog);
            let (count, max_seq) = count_pushed(&conn, "device-a").await;
            assert_eq!(count as usize, expected, "pass {pass} sends at most one bounded pass");
            assert_eq!(store.get_sync_push_cursor().unwrap(), max_seq, "pass {pass}: the cursor is the highest row sent");
        }
        let cursor = store.get_sync_push_cursor().unwrap();
        push_content(&conn, &group_key, &store, "device-a").await.unwrap();
        assert_eq!(count_pushed(&conn, "device-a").await.0 as usize, backlog, "nothing left to send");
        assert_eq!(store.get_sync_push_cursor().unwrap(), cursor);

        // A peer pulls every row (the pull side caps its own passes too) and decrypts them.
        let store_b = Store::open_in_memory().unwrap();
        for _ in 0..(backlog / 500 + 2) {
            pull_content(&conn, &group_key, &store_b, "device-b").await.unwrap();
        }
        assert_eq!(store_b.recent(backlog + 10).unwrap().len(), backlog);
    }

    #[tokio::test]
    async fn push_content_is_idempotent_on_a_retried_row() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("shared.db");
        let group_key = GroupKey::generate();
        let conn = local_connection(&db_path).await;
        ensure_content_schema(&conn).await.unwrap();

        let store = Store::open_in_memory().unwrap();
        let id = store
            .insert_meeting(&memory::NewMeeting {
                device_id: "device-a".to_string(),
                start_unix_ms: 1000,
                end_unix_ms: None,
                title: "Standup".to_string(),
                attendees: vec![],
                trigger: memory::MeetingTrigger::Calendar,
                audio_path: None,
                status: memory::MeetingStatus::Recording,
            })
            .unwrap();
        store.complete_meeting(id, 2000, None).unwrap();

        push_content(&conn, &group_key, &store, "device-a")
            .await
            .unwrap();
        // Simulate a retry where the push cursor write never happened, by resetting it
        // manually and pushing again — the row must not error or duplicate.
        store.set_sync_push_cursor(0).unwrap();
        push_content(&conn, &group_key, &store, "device-a")
            .await
            .unwrap();

        let mut rows = conn
            .query("SELECT COUNT(*) FROM sync_row WHERE device_id = 'device-a' AND local_seq != 0", ())
            .await
            .unwrap();
        let count: i64 = rows.next().await.unwrap().unwrap().get(0).unwrap();
        assert_eq!(
            count, 1,
            "re-pushing an already-pushed row must not create a duplicate"
        );
    }

    #[tokio::test]
    async fn push_content_excludes_a_recording_meeting() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("shared.db");
        let group_key = GroupKey::generate();
        let conn = local_connection(&db_path).await;
        ensure_content_schema(&conn).await.unwrap();

        let store = Store::open_in_memory().unwrap();
        store
            .insert_meeting(&memory::NewMeeting {
                device_id: "device-a".to_string(),
                start_unix_ms: 1000,
                end_unix_ms: None,
                title: "Still Recording".to_string(),
                attendees: vec![],
                trigger: memory::MeetingTrigger::Calendar,
                audio_path: None,
                status: memory::MeetingStatus::Recording,
            })
            .unwrap();

        push_content(&conn, &group_key, &store, "device-a")
            .await
            .unwrap();

        let mut rows = conn
            .query(
                "SELECT COUNT(*) FROM sync_row WHERE local_seq != 0",
                (),
            )
            .await
            .unwrap();
        let count: i64 = rows.next().await.unwrap().unwrap().get(0).unwrap();
        assert_eq!(count, 0);
    }

    async fn seed_content_row(
        conn: &Connection,
        group_key: &GroupKey,
        device_id: &str,
        local_seq: i64,
        row_type: &str,
        payload_json: &str,
    ) {
        let aad = content_row_aad(device_id, local_seq, row_type);
        let ciphertext = sync::encrypt_payload(group_key, payload_json.as_bytes(), &aad);
        conn.execute(
            "INSERT INTO sync_row (device_id, local_seq, row_type, ciphertext, created_at_unix_ms) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![device_id, local_seq, row_type, STANDARD.encode(&ciphertext).as_str(), 0_i64],
        )
        .await
        .unwrap();
    }

    fn sample_new_capture_item(text: &str) -> memory::NewCaptureItem {
        memory::NewCaptureItem {
            device_id: "dev-a".to_string(),
            ts_unix_ms: 1000,
            app_bundle_id: "com.a".to_string(),
            window_title: "Local".to_string(),
            url: Some("https://example.com".to_string()),
            text: text.to_string(),
            hash: 9,
            source: "screen".to_string(),
        }
    }

    /// A peer's own marker, inserted directly — simulates the peer having already
    /// upgraded and run at least one sync round.
    async fn seed_peer_capability_marker(conn: &Connection, peer_device_id: &str) {
        conn.execute(
            "INSERT INTO sync_row (device_id, local_seq, row_type, ciphertext, created_at_unix_ms) VALUES (?1, 0, 'items_capability', 'x', 0)",
            params![peer_device_id],
        )
        .await
        .unwrap();
    }

    async fn seed_rows(conn: &Connection, device_id: &str, count: i64) {
        for seq in 1..=count {
            seed_content_row(conn, &GroupKey::from_bytes([7; 32]), device_id, seq, "items", "{}").await;
        }
    }

    #[tokio::test]
    async fn peer_device_ids_lists_each_peer_once_and_never_the_own_device() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("remote.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        for device in ["dev-a", "dev-own", "dev-b", "dev-c"] {
            seed_rows(&conn, device, 3).await;
        }
        let peers = distinct_peer_device_ids(&conn, "dev-own").await.unwrap();
        assert_eq!(peers, ["dev-a", "dev-b", "dev-c"]);
        assert!(distinct_peer_device_ids(&conn, "dev-a").await.unwrap().contains(&"dev-own".to_string()));
    }

    #[tokio::test]
    async fn all_peers_items_capable_needs_a_marker_from_every_peer() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("remote.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        seed_rows(&conn, "dev-own", 2).await;
        assert!(all_peers_items_capable(&conn, "dev-own").await.unwrap(), "no peers is vacuously true");

        seed_rows(&conn, "dev-a", 2).await;
        assert!(!all_peers_items_capable(&conn, "dev-own").await.unwrap());
        seed_peer_capability_marker(&conn, "dev-a").await;
        assert!(all_peers_items_capable(&conn, "dev-own").await.unwrap());

        seed_rows(&conn, "dev-b", 2).await;
        assert!(!all_peers_items_capable(&conn, "dev-own").await.unwrap(), "a second peer without a marker");
    }

    /// Turso bills rows read, so a per-sync query must seek the primary key, not scan.
    #[tokio::test]
    async fn the_peer_queries_seek_the_primary_key_instead_of_scanning() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("remote.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        // The last one is fetch_rows_by_key's shape with two keys.
        let by_key = "SELECT local_seq, row_type, ciphertext FROM sync_row WHERE device_id = ?1 AND local_seq IN (?2, ?3)";
        for sql in [NEXT_PEER_SQL, PEER_MARKER_SQL, by_key] {
            let args = vec![libsql::Value::Text("x".into()); sql.matches('?').count()];
            let mut rows = conn.query(&format!("EXPLAIN QUERY PLAN {sql}"), args).await.unwrap();
            let mut plan = Vec::new();
            while let Some(row) = rows.next().await.unwrap() {
                plan.push(row.get::<String>(3).unwrap());
            }
            let plan = plan.join(" | ");
            assert!(plan.contains("SEARCH sync_row"), "{sql}: {plan}");
            assert!(!plan.contains("SCAN"), "{sql}: {plan}");
        }
    }

    async fn single_i64(conn: &Connection, sql: &str) -> i64 {
        let mut rows = conn.query(sql, params![]).await.unwrap();
        rows.next().await.unwrap().unwrap().get(0).unwrap()
    }

    async fn marker_count(conn: &Connection, device_id: &str) -> i64 {
        let mut rows = conn
            .query("SELECT COUNT(*) FROM sync_row WHERE device_id = ?1 AND row_type = 'items_capability'", params![device_id])
            .await
            .unwrap();
        rows.next().await.unwrap().unwrap().get(0).unwrap()
    }

    /// The row_type of the highest-local_seq real content row this device pushed.
    async fn latest_pushed_row_type(conn: &Connection, device_id: &str) -> Option<String> {
        let mut rows = conn
            .query(
                "SELECT row_type FROM sync_row WHERE device_id = ?1 AND local_seq != 0 ORDER BY local_seq DESC LIMIT 1",
                params![device_id],
            )
            .await
            .unwrap();
        rows.next().await.unwrap().map(|r| r.get(0).unwrap())
    }

    fn sample_applied_meeting(title: &str) -> memory::AppliedMeeting {
        memory::AppliedMeeting {
            start_unix_ms: 1000,
            end_unix_ms: Some(2000),
            title: title.to_string(),
            attendees: vec![],
            trigger: memory::MeetingTrigger::Detected,
            status: memory::MeetingStatus::Completed,
            transcript_path: None,
            notes: None,
            summary_tldr: None,
            summary_decisions: vec![],
            summary_action_items: vec![],
            summary_open_questions: vec![],
            summary_provider: None,
            summary_model: None,
            keep_media: None,
        }
    }

    #[test]
    fn meeting_payload_deserializes_a_legacy_wire_row_missing_source_meeting_id() {
        // Simulates a `sync_row` written by a pre-fix build: just the flattened
        // `AppliedMeeting` fields, no `source_meeting_id` key at all.
        let legacy_json = serde_json::to_string(&sample_applied_meeting("Legacy")).unwrap();
        let decoded: MeetingPayload = serde_json::from_str(&legacy_json).unwrap();
        assert_eq!(decoded.source_meeting_id, None);
        assert_eq!(decoded.meeting.title, "Legacy");
    }

    #[test]
    fn meeting_payload_round_trips_source_meeting_id() {
        let payload = MeetingPayload {
            source_meeting_id: Some(100),
            meeting: sample_applied_meeting("Standup"),
        };
        let json = serde_json::to_string(&payload).unwrap();
        let decoded: MeetingPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.source_meeting_id, Some(100));
        assert_eq!(decoded.meeting.title, "Standup");
    }

    #[tokio::test]
    async fn pull_content_applies_a_peer_meeting_and_capture_item() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("shared.db");
        let group_key = GroupKey::generate();
        let conn = local_connection(&db_path).await;
        ensure_content_schema(&conn).await.unwrap();

        let meeting_json = serde_json::to_string(&sample_applied_meeting("Peer Standup")).unwrap();
        seed_content_row(
            &conn,
            &group_key,
            "device-a",
            1,
            ROW_TYPE_MEETING,
            &meeting_json,
        )
        .await;

        let capture_json = serde_json::to_string(&AppliedCaptureItem {
            ts_unix_ms: 1500,
            app_bundle_id: "com.apple.Safari".to_string(),
            window_title: "Peer Window".to_string(),
            url: None,
            text: "peer captured text".to_string(),
            hash: 7,
            source: Some("ax".to_string()),
        })
        .unwrap();
        seed_content_row(
            &conn,
            &group_key,
            "device-a",
            2,
            ROW_TYPE_CAPTURE_ITEM,
            &capture_json,
        )
        .await;

        let store_b = Store::open_in_memory().unwrap();
        pull_content(&conn, &group_key, &store_b, "device-b")
            .await
            .unwrap();

        let meeting_local_id = store_b
            .resolve_sync_row_map("device-a", 1, ROW_TYPE_MEETING)
            .unwrap()
            .unwrap();
        let applied_meeting = store_b.get_meeting(meeting_local_id).unwrap().unwrap();
        assert_eq!(applied_meeting.title, "Peer Standup");
        assert_eq!(applied_meeting.device_id, "device-a");

        let results = store_b.search_keyword("peer captured text", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(store_b.get_sync_cursor("device-a").unwrap(), 2);
    }

    #[tokio::test]
    async fn pull_content_applies_a_peer_routine_run_once_and_an_update_replaces_it() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("shared.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        let group_key = GroupKey::generate();
        let first = serde_json::to_string(&memory::AppliedRoutineRun {
            source_run_id: 4,
            template: "daily_brief".into(),
            ran_at_unix_ms: 9_000,
            status: memory::RunStatus::Success,
            output_summary: "first brief".into(),
            output_json: "{}".into(),
        })
        .unwrap();
        seed_content_row(&conn, &group_key, "device-a", 1, ROW_TYPE_ROUTINE_RUN, &first).await;
        let updated = first.replace("first brief", "revised brief");
        seed_content_row(&conn, &group_key, "device-a", 2, ROW_TYPE_ROUTINE_RUN, &updated).await;
        let unknown = serde_json::json!({
            "source_run_id": 5, "template": "from_the_future", "ran_at_unix_ms": 1,
            "status": "success", "output_summary": "x", "output_json": "{}"
        })
        .to_string();
        seed_content_row(&conn, &group_key, "device-a", 3, ROW_TYPE_ROUTINE_RUN, &unknown).await;

        let store = Store::open_in_memory().unwrap();
        pull_content(&conn, &group_key, &store, "device-b").await.unwrap();

        let runs = store.recent_routine_runs_across_all(10).unwrap();
        assert_eq!(runs.len(), 1, "the same origin run twice is one run; an unknown template is ignored");
        assert_eq!(runs[0].output_summary, "revised brief");
        assert_eq!(runs[0].origin_device_id.as_deref(), Some("device-a"));
        assert_eq!(store.get_sync_cursor("device-a").unwrap(), 3, "the cursor still advances past an ignored row");
    }

    #[tokio::test]
    async fn a_routine_run_row_that_cannot_be_decoded_is_skipped_and_does_not_abort_the_pull() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("shared.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        let group_key = GroupKey::generate();
        // A status this build has never heard of (e.g. from a newer app), then a good run.
        let future_status = serde_json::json!({
            "source_run_id": 1, "template": "daily_brief", "ran_at_unix_ms": 1,
            "status": "levitating", "output_summary": "x", "output_json": "{}"
        })
        .to_string();
        seed_content_row(&conn, &group_key, "device-a", 1, ROW_TYPE_ROUTINE_RUN, &future_status).await;
        seed_content_row(&conn, &group_key, "device-a", 2, ROW_TYPE_ROUTINE_RUN, "not json at all").await;
        let good = serde_json::to_string(&memory::AppliedRoutineRun {
            source_run_id: 3,
            template: "daily_brief".into(),
            ran_at_unix_ms: 5,
            status: memory::RunStatus::Success,
            output_summary: "good".into(),
            output_json: "{}".into(),
        })
        .unwrap();
        seed_content_row(&conn, &group_key, "device-a", 3, ROW_TYPE_ROUTINE_RUN, &good).await;

        let store = Store::open_in_memory().unwrap();
        pull_content(&conn, &group_key, &store, "device-b").await.expect("a bad routine_run row must not fail the pull");
        let runs = store.recent_routine_runs_across_all(10).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].output_summary, "good");
        assert_eq!(store.get_sync_cursor("device-a").unwrap(), 3);
    }

    #[tokio::test]
    async fn pull_content_applies_a_peer_meeting_prep_and_skips_a_bad_one() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("shared.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        let group_key = GroupKey::generate();
        let good = serde_json::json!({
            "event_id": "ev1", "title": "Sync", "start_unix_ms": 2_000_000, "generated_at_unix_ms": 5,
            "brief_json": "{}", "provider": "p", "model": "m"
        })
        .to_string();
        seed_content_row(&conn, &group_key, "device-a", 1, ROW_TYPE_MEETING_PREP, "not json at all").await;
        seed_content_row(&conn, &group_key, "device-a", 2, ROW_TYPE_MEETING_PREP, &good).await;

        let store = Store::open_in_memory().unwrap();
        pull_content(&conn, &group_key, &store, "device-b").await.expect("a bad meeting_prep row must not fail the pull");
        assert_eq!(store.get_meeting_prep("ev1").unwrap().unwrap().generated_at_unix_ms, 5);
        assert_eq!(store.get_sync_cursor("device-a").unwrap(), 2);
    }

    #[tokio::test]
    async fn pull_content_applies_a_peer_profile_suggestion_and_skips_a_bad_one() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("shared.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        let group_key = GroupKey::generate();
        let good = serde_json::json!({"source_id": 4, "text": "Prefers mornings", "created_at_unix_ms": 5}).to_string();
        seed_content_row(&conn, &group_key, "device-a", 1, ROW_TYPE_PROFILE_SUGGESTION, "not json at all").await;
        seed_content_row(&conn, &group_key, "device-a", 2, ROW_TYPE_PROFILE_SUGGESTION, &good).await;

        let store = Store::open_in_memory().unwrap();
        pull_content(&conn, &group_key, &store, "device-b").await.expect("a bad profile_suggestion row must not fail the pull");
        let pending = store.list_profile_suggestions("pending").unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].text, "Prefers mornings");
        assert_eq!(store.get_sync_cursor("device-a").unwrap(), 2);
    }

    #[tokio::test]
    async fn pull_content_bounds_one_pass_and_resumes_the_rest_on_the_next_call() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("shared.db");
        let group_key = GroupKey::generate();
        let conn = local_connection(&db_path).await;
        ensure_content_schema(&conn).await.unwrap();

        let total_rows = MAX_ROWS_PER_PULL_PASS + 3;
        for local_seq in 1..=total_rows {
            let capture_json = serde_json::to_string(&AppliedCaptureItem {
                ts_unix_ms: local_seq,
                app_bundle_id: "com.apple.Safari".to_string(),
                window_title: "Peer Window".to_string(),
                url: None,
                text: format!("peer captured text {local_seq}"),
                hash: local_seq,
                source: Some("ax".to_string()),
            })
            .unwrap();
            seed_content_row(
                &conn,
                &group_key,
                "device-a",
                local_seq,
                ROW_TYPE_CAPTURE_ITEM,
                &capture_json,
            )
            .await;
        }

        let store_b = Store::open_in_memory().unwrap();
        pull_content(&conn, &group_key, &store_b, "device-b")
            .await
            .unwrap();

        // First pass stops at the cap, not the full backlog -- proves one call can never
        // block on an arbitrarily large first-time peer history.
        assert_eq!(
            store_b.get_sync_cursor("device-a").unwrap(),
            MAX_ROWS_PER_PULL_PASS
        );
        assert_eq!(
            store_b
                .search_keyword(&format!("peer captured text {total_rows}"), 10)
                .unwrap()
                .len(),
            0
        );

        pull_content(&conn, &group_key, &store_b, "device-b")
            .await
            .unwrap();

        // Second call resumes from the persisted cursor and finishes the backlog.
        assert_eq!(store_b.get_sync_cursor("device-a").unwrap(), total_rows);
        assert_eq!(
            store_b
                .search_keyword(&format!("peer captured text {total_rows}"), 10)
                .unwrap()
                .len(),
            1
        );
    }

    /// The actual production bug: under the old design, a transcript segment whose
    /// parent meeting hasn't arrived yet pinned the peer's cursor at `local_seq - 1`
    /// forever. Combined with `MAX_ROWS_PER_PULL_PASS`, that meant every pass re-fetched
    /// the SAME bounded window starting right after the stuck row -- any real content
    /// pushed beyond that window (e.g. a device's more recent meetings) was never
    /// fetched again, not merely delayed. This proves the new design instead keeps
    /// draining the backlog past the stuck row, while still resolving it once its
    /// dependency finally arrives.
    #[tokio::test]
    async fn pull_content_drains_the_backlog_past_an_unresolved_dependency_instead_of_stalling_forever(
    ) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("shared.db");
        let group_key = GroupKey::generate();
        let conn = local_connection(&db_path).await;
        ensure_content_schema(&conn).await.unwrap();

        // A segment at local_seq=1 referencing a meeting (local_seq=999) that doesn't
        // exist yet -- stays unresolved across every pass until seeded at the end.
        let segment_payload = serde_json::to_string(&TranscriptSegmentPayload {
            meeting_local_seq: 999,
            source_meeting_id: None,
            segment: AppliedTranscriptSegment {
                start_cs: 0,
                speaker_label: "Alice".to_string(),
                text: "stuck segment".to_string(),
            },
        })
        .unwrap();
        seed_content_row(
            &conn,
            &group_key,
            "device-a",
            1,
            ROW_TYPE_TRANSCRIPT_SEGMENT,
            &segment_payload,
        )
        .await;

        // A backlog of ordinary rows behind the stuck segment, spanning more than one
        // pull pass (local_seq 2..=551).
        let backlog_end = 1 + MAX_ROWS_PER_PULL_PASS + 50;
        for local_seq in 2..=backlog_end {
            let capture_json = serde_json::to_string(&AppliedCaptureItem {
                ts_unix_ms: local_seq,
                app_bundle_id: "com.apple.Safari".to_string(),
                window_title: "Peer Window".to_string(),
                url: None,
                text: format!("backlog text {local_seq}"),
                hash: local_seq,
                source: Some("ax".to_string()),
            })
            .unwrap();
            seed_content_row(
                &conn,
                &group_key,
                "device-a",
                local_seq,
                ROW_TYPE_CAPTURE_ITEM,
                &capture_json,
            )
            .await;
        }

        let store_b = Store::open_in_memory().unwrap();
        pull_content(&conn, &group_key, &store_b, "device-b")
            .await
            .unwrap();
        pull_content(&conn, &group_key, &store_b, "device-b")
            .await
            .unwrap();

        // Two passes fully drain the backlog (500 + 51 rows) despite the still-unresolved
        // segment at the very front -- the old design would have re-fetched local_seq
        // 1..=500 on the second pass too, never reaching row 501 onward.
        assert_eq!(store_b.get_sync_cursor("device-a").unwrap(), backlog_end);
        assert_eq!(
            store_b
                .search_keyword(&format!("backlog text {backlog_end}"), 10)
                .unwrap()
                .len(),
            1,
            "the last row of the backlog, well past MAX_ROWS_PER_PULL_PASS, must have been applied"
        );
        assert_eq!(
            store_b.list_sync_pending_dependencies("device-a").unwrap(),
            vec![(1, ROW_TYPE_TRANSCRIPT_SEGMENT.to_string())],
            "the stuck segment is tracked for retry, not silently dropped"
        );

        // Now its dependency finally arrives.
        let meeting_json = serde_json::to_string(&sample_applied_meeting("Standup")).unwrap();
        seed_content_row(
            &conn,
            &group_key,
            "device-a",
            999,
            ROW_TYPE_MEETING,
            &meeting_json,
        )
        .await;

        pull_content(&conn, &group_key, &store_b, "device-b")
            .await
            .unwrap();

        assert_eq!(store_b.get_sync_cursor("device-a").unwrap(), 999);
        assert!(
            store_b.list_sync_pending_dependencies("device-a").unwrap().is_empty(),
            "the segment must resolve and clear from the pending set"
        );
        let results = store_b
            .search_transcript_keyword("stuck segment", 10)
            .unwrap();
        assert_eq!(results.len(), 1);
    }

    /// The time budget: a pass stops once it's spent, saves the cursor at the last row
    /// it handled, and resumes there. A zero budget still handles one row per peer per
    /// call, so one peer's backlog never starves another's pulls.
    #[tokio::test]
    async fn pull_content_stops_a_pass_at_its_time_budget_and_every_peer_progresses() {
        let dir = tempfile::tempdir().unwrap();
        let group_key = GroupKey::generate();
        let conn = local_connection(&dir.path().join("shared.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        for (device, word, rows) in [("device-a", "alpha", 3), ("device-c", "gamma", 2)] {
            for local_seq in 1..=rows {
                let capture_json = serde_json::to_string(&AppliedCaptureItem {
                    ts_unix_ms: local_seq,
                    app_bundle_id: "com.apple.Safari".to_string(),
                    window_title: "Peer Window".to_string(),
                    url: None,
                    text: format!("{word} row {local_seq}"),
                    hash: local_seq,
                    source: Some("ax".to_string()),
                })
                .unwrap();
                seed_content_row(
                    &conn,
                    &group_key,
                    device,
                    local_seq,
                    ROW_TYPE_CAPTURE_ITEM,
                    &capture_json,
                )
                .await;
            }
        }
        let store_b = Store::open_in_memory().unwrap();
        let found = |text: &str| store_b.search_keyword(text, 10).unwrap().len();
        let zero = std::time::Duration::ZERO;

        let more = pull_content_with_budget(&conn, &group_key, &store_b, "device-b", zero, &PullOptions::default(), false).await.unwrap();
        assert!(more, "a pass cut short says rows are waiting");
        assert_eq!(store_b.get_sync_cursor("device-a").unwrap(), 1, "the cursor stops at the last row handled");
        assert_eq!(store_b.get_sync_cursor("device-c").unwrap(), 1, "the other peer progresses in the same call");
        assert_eq!((found("alpha row 1"), found("alpha row 2")), (1, 0));

        for _ in 0..2 {
            pull_content_with_budget(&conn, &group_key, &store_b, "device-b", zero, &PullOptions::default(), false).await.unwrap();
        }
        assert_eq!(store_b.get_sync_cursor("device-a").unwrap(), 3);
        assert_eq!(store_b.get_sync_cursor("device-c").unwrap(), 2);
        let more = pull_content_with_budget(&conn, &group_key, &store_b, "device-b", zero, &PullOptions::default(), false).await.unwrap();
        assert!(!more, "a caught-up device has nothing waiting");
        for text in ["alpha row 2", "alpha row 3", "gamma row 1", "gamma row 2"] {
            assert_eq!(found(text), 1, "{text} applied exactly once");
        }
    }

    /// Seeds `count` transcript segments (local_seq 1..=count) whose meeting
    /// (meeting_local_seq 999) isn't on the relay, and pulls once so they all go pending.
    async fn seed_pending_segments(conn: &Connection, group_key: &GroupKey, store: &Store, count: i64) {
        for local_seq in 1..=count {
            let payload = serde_json::to_string(&TranscriptSegmentPayload {
                meeting_local_seq: 999,
                source_meeting_id: None,
                segment: AppliedTranscriptSegment {
                    start_cs: local_seq,
                    speaker_label: "Alice".to_string(),
                    text: format!("pending segment {local_seq}"),
                },
            })
            .unwrap();
            seed_content_row(conn, group_key, "device-a", local_seq, ROW_TYPE_TRANSCRIPT_SEGMENT, &payload).await;
        }
        pull_content(conn, group_key, store, "device-b").await.unwrap();
        assert_eq!(store.list_sync_pending_dependencies("device-a").unwrap().len() as i64, count);
    }

    fn take_row_queries() -> usize {
        ROW_QUERIES.with(|c| c.replace(0))
    }

    async fn seed_tagged_row(conn: &Connection, device_id: &str, local_seq: i64, row_type: &str, tag: &str) {
        conn.execute(
            "INSERT INTO sync_row (device_id, local_seq, row_type, ciphertext, created_at_unix_ms, entity_tag) \
             VALUES (?1, ?2, ?3, 'x', 0, ?4)",
            params![device_id, local_seq, row_type, tag],
        )
        .await
        .unwrap();
    }

    fn completed_meeting(store: &Store, device_id: &str) -> i64 {
        let id = store
            .insert_meeting(&memory::NewMeeting {
                device_id: device_id.to_string(),
                start_unix_ms: 1000,
                end_unix_ms: None,
                title: "Standup".to_string(),
                attendees: vec![],
                trigger: memory::MeetingTrigger::Calendar,
                audio_path: None,
                status: memory::MeetingStatus::Recording,
            })
            .unwrap();
        store.complete_meeting(id, 2000, None).unwrap();
        id
    }

    async fn meeting_rows(conn: &Connection, device_id: &str) -> i64 {
        let mut rows = conn
            .query(
                "SELECT COUNT(*) FROM sync_row WHERE device_id = ?1 AND row_type = 'meeting'",
                params![device_id],
            )
            .await
            .unwrap();
        rows.next().await.unwrap().unwrap().get(0).unwrap()
    }

    #[tokio::test]
    async fn a_meeting_restamped_three_times_leaves_one_relay_row() {
        let dir = tempfile::tempdir().unwrap();
        let conn = compacting_relay(&dir).await;
        let key = GroupKey::generate();
        let store = Store::open_in_memory().unwrap();

        let id = completed_meeting(&store, "device-a");
        push_content(&conn, &key, &store, "device-a").await.unwrap();
        store.set_meeting_title(id, "Renamed").unwrap();
        push_content(&conn, &key, &store, "device-a").await.unwrap();
        store
            .update_meeting_summary(id, "tl;dr", &[], &[], &[], "anthropic", "claude-haiku-4-5")
            .unwrap();
        push_content(&conn, &key, &store, "device-a").await.unwrap();

        assert_eq!(single_i64(&conn, "SELECT COUNT(*) FROM sync_row WHERE device_id = 'device-a' AND row_type = 'meeting'").await, 1);
        assert_eq!(single_i64(&conn, "SELECT local_seq FROM sync_row WHERE device_id = 'device-a' AND row_type = 'meeting'").await,
                   store.get_sync_push_cursor().unwrap());
    }

    #[tokio::test]
    async fn a_restamped_calendar_event_keeps_one_row_and_captures_are_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let conn = compacting_relay(&dir).await;
        let key = GroupKey::generate();
        let store = Store::open_in_memory().unwrap();

        store.insert_capture_item(&sample_new_capture_item("first capture")).unwrap();
        // A distinct hash, or the per-app dedup drops it.
        store
            .insert_capture_item(&memory::NewCaptureItem { hash: 10, ..sample_new_capture_item("second capture") })
            .unwrap();
        let mut event = memory::NewCalendarEventItem {
            device_id: "dev-a".to_string(),
            source: "google_calendar".to_string(),
            external_id: "evt-1".to_string(),
            ts_unix_ms: 5000,
            title: "Planning".to_string(),
            actors: vec![],
        };
        store.upsert_calendar_event_items(std::slice::from_ref(&event)).unwrap();
        push_content(&conn, &key, &store, "dev-a").await.unwrap();
        event.title = "Planning (moved)".to_string();
        store.upsert_calendar_event_items(std::slice::from_ref(&event)).unwrap();
        push_content(&conn, &key, &store, "dev-a").await.unwrap();

        let tagged = "SELECT COUNT(*) FROM sync_row WHERE row_type = 'items' AND entity_tag IS NOT NULL";
        let untagged = "SELECT COUNT(*) FROM sync_row WHERE row_type = 'items' AND entity_tag IS NULL";
        assert_eq!(single_i64(&conn, tagged).await, 1);
        assert_eq!(single_i64(&conn, untagged).await, 2);
    }

    #[tokio::test]
    async fn a_capture_only_push_issues_no_delete() {
        let dir = tempfile::tempdir().unwrap();
        let conn = compacting_relay(&dir).await;
        let store = Store::open_in_memory().unwrap();
        store.insert_capture_item(&sample_new_capture_item("only a capture")).unwrap();
        take_compaction_deletes();

        push_content(&conn, &GroupKey::generate(), &store, "dev-a").await.unwrap();

        assert_eq!(single_i64(&conn, "SELECT COUNT(*) FROM sync_row WHERE row_type = 'items'").await, 1);
        assert_eq!(take_compaction_deletes(), 0);
    }

    #[tokio::test]
    async fn an_old_schema_relay_still_takes_untagged_pushes() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("relay.db")).await;
        conn.execute(OLD_SYNC_ROW_DDL, ()).await.unwrap();
        // The account-switch case: this process saw the column on another relay, so it
        // still tries the tagged INSERT and must fall back.
        ENTITY_TAG_COLUMN_SEEN.set();
        let key = GroupKey::generate();
        let store = Store::open_in_memory().unwrap();
        take_compaction_deletes();

        let id = completed_meeting(&store, "device-a");
        push_content(&conn, &key, &store, "device-a").await.unwrap();
        store.set_meeting_title(id, "Renamed").unwrap();
        push_content(&conn, &key, &store, "device-a").await.unwrap();

        assert_eq!(meeting_rows(&conn, "device-a").await, 2);
        assert_eq!(take_compaction_deletes(), 0);
    }

    /// One failed tagged INSERT (a relay without the column, after an account switch), then
    /// the rest of the call pushes untagged: later chunks do not pay a doomed INSERT.
    #[tokio::test]
    async fn after_a_missing_column_fallback_the_rest_of_the_push_is_untagged() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("relay.db")).await;
        conn.execute(OLD_SYNC_ROW_DDL, ()).await.unwrap();
        ENTITY_TAG_COLUMN_SEEN.set();
        let store = Store::open_in_memory().unwrap();
        let events: Vec<memory::NewCalendarEventItem> = (0..120)
            .map(|i| memory::NewCalendarEventItem {
                device_id: "dev-a".to_string(),
                source: "google_calendar".to_string(),
                external_id: format!("evt-{i}"),
                ts_unix_ms: 5000 + i,
                title: format!("Event {i}"),
                actors: vec![],
            })
            .collect();
        store.upsert_calendar_event_items(&events).unwrap();
        FAILED_TAGGED_INSERTS.with(|c| c.set(0));

        push_content(&conn, &GroupKey::generate(), &store, "dev-a").await.unwrap();

        assert_eq!(single_i64(&conn, "SELECT COUNT(*) FROM sync_row WHERE row_type = 'items'").await, 120);
        assert_eq!(FAILED_TAGGED_INSERTS.with(|c| c.get()), 1, "three chunks, one doomed INSERT");
    }

    #[tokio::test]
    async fn a_failing_compaction_delete_never_fails_the_push() {
        let dir = tempfile::tempdir().unwrap();
        let conn = compacting_relay(&dir).await;
        conn.execute("CREATE TRIGGER no_delete BEFORE DELETE ON sync_row BEGIN SELECT RAISE(ABORT, 'nope'); END", ())
            .await
            .unwrap();
        let key = GroupKey::generate();
        let store = Store::open_in_memory().unwrap();

        let id = completed_meeting(&store, "device-a");
        push_content(&conn, &key, &store, "device-a").await.unwrap();
        store.set_meeting_title(id, "Renamed").unwrap();
        push_content(&conn, &key, &store, "device-a").await.unwrap();

        assert_eq!(meeting_rows(&conn, "device-a").await, 2);
        assert_eq!(
            single_i64(&conn, "SELECT MAX(local_seq) FROM sync_row WHERE device_id = 'device-a' AND row_type = 'meeting'").await,
            store.get_sync_push_cursor().unwrap()
        );
    }

    #[tokio::test]
    async fn compaction_never_touches_another_devices_rows() {
        let dir = tempfile::tempdir().unwrap();
        let conn = compacting_relay(&dir).await;
        let key = GroupKey::generate();
        let store = Store::open_in_memory().unwrap();

        let id = completed_meeting(&store, "device-a");
        let forged = tag_for(&key, "device-a", &store.sync_entity_epoch().unwrap(), &format!("meeting:{id}"));
        // Two versions, so device-b's older row would go if the DELETE lost its device scope.
        seed_tagged_row(&conn, "device-b", 1, "meeting", &forged).await;
        seed_tagged_row(&conn, "device-b", 2, "meeting", &forged).await;
        // Without a marker, device-b would close the items cutover gate.
        seed_peer_capability_marker(&conn, "device-b").await;
        push_content(&conn, &key, &store, "device-a").await.unwrap();
        store.set_meeting_title(id, "Renamed").unwrap();
        push_content(&conn, &key, &store, "device-a").await.unwrap();

        assert_eq!(meeting_rows(&conn, "device-a").await, 1, "device-a's own old version is compacted");
        assert_eq!(meeting_rows(&conn, "device-b").await, 2, "device-b's rows survive");
        assert_eq!(
            single_i64(&conn, "SELECT COUNT(DISTINCT entity_tag) FROM sync_row WHERE row_type = 'meeting'").await,
            1,
            "the forged tag really collides with device-a's"
        );
    }

    #[tokio::test]
    async fn a_different_group_key_never_deletes_the_other_keys_rows() {
        let dir = tempfile::tempdir().unwrap();
        let conn = compacting_relay(&dir).await;
        let store = Store::open_in_memory().unwrap();

        let id = completed_meeting(&store, "device-a");
        push_content(&conn, &GroupKey::generate(), &store, "device-a").await.unwrap();
        store.set_meeting_title(id, "Renamed").unwrap();
        push_content(&conn, &GroupKey::generate(), &store, "device-a").await.unwrap();

        assert_eq!(meeting_rows(&conn, "device-a").await, 2);
    }

    #[tokio::test]
    async fn compact_entity_tags_keeps_the_newest_version_of_every_tag() {
        let dir = tempfile::tempdir().unwrap();
        let conn = compacting_relay(&dir).await;
        // Tag "a": three versions; tag "b": a single version; one untagged row.
        for seq in [1, 4, 7] {
            seed_tagged_row(&conn, "device-a", seq, "meeting", "a").await;
        }
        seed_tagged_row(&conn, "device-a", 2, "meeting", "b").await;
        seed_content_row(&conn, &GroupKey::from_bytes([7; 32]), "device-a", 3, "items", "{}").await;

        let tags = ["a".to_string(), "b".to_string(), "unseen".to_string()];
        assert_eq!(compact_entity_tags(&conn, "device-a", &tags).await, 2);
        assert_eq!(compact_entity_tags(&conn, "device-a", &tags).await, 0, "idempotent");
        assert_eq!(compact_entity_tags(&conn, "device-a", &[]).await, 0);

        let mut rows = conn
            .query("SELECT local_seq FROM sync_row ORDER BY local_seq", ())
            .await
            .unwrap();
        let mut left = Vec::new();
        while let Some(row) = rows.next().await.unwrap() {
            left.push(row.get::<i64>(0).unwrap());
        }
        assert_eq!(left, [2, 3, 7]);
    }

    #[test]
    fn two_databases_under_one_device_id_tag_the_same_meeting_differently() {
        let key = GroupKey::generate();
        let (old_db, new_db) = (Store::open_in_memory().unwrap(), Store::open_in_memory().unwrap());
        let tag = |store: &Store| tag_for(&key, "device-a", &store.sync_entity_epoch().unwrap(), "meeting:1");
        assert_ne!(tag(&old_db), tag(&new_db));
        assert_eq!(tag(&old_db), tag(&old_db));
    }

    /// The wipe case: the Keychain keeps `device_id`, the new local database restarts
    /// meeting ids, so its meeting 1 is a different meeting from the relay's meeting 1.
    #[tokio::test]
    async fn a_push_from_a_new_database_never_deletes_the_old_databases_rows() {
        let dir = tempfile::tempdir().unwrap();
        let conn = compacting_relay(&dir).await;
        let key = GroupKey::generate();

        let old_db = Store::open_in_memory().unwrap();
        let old_id = completed_meeting(&old_db, "device-a");
        for title in ["b", "c", "d", "e"] {
            old_db.set_meeting_title(old_id, title).unwrap();
        }
        push_content(&conn, &key, &old_db, "device-a").await.unwrap();
        let old_seq = single_i64(&conn, "SELECT local_seq FROM sync_row WHERE row_type = 'meeting'").await;

        let new_db = Store::open_in_memory().unwrap();
        let new_id = completed_meeting(&new_db, "device-a");
        assert_eq!(new_id, old_id, "the wipe reuses the meeting id");
        push_content(&conn, &key, &new_db, "device-a").await.unwrap();

        assert_eq!(meeting_rows(&conn, "device-a").await, 2, "neither database's meeting is deleted");
        assert!(new_db.get_sync_push_cursor().unwrap() < old_seq, "the new row sits below the old MAX");
    }

    #[test]
    fn missing_entity_tag_column_is_recognized() {
        assert!(is_missing_entity_tag_column("table sync_row has no column named entity_tag"));
        assert!(!is_missing_entity_tag_column("database is locked"));
        // Mentions the column but is not the missing-column error: must not trigger the retry.
        assert!(!is_missing_entity_tag_column("CHECK constraint failed: entity_tag"));
    }

    /// A relay as ops leaves it: the column plus the out-of-band index.
    pub(crate) async fn compacting_relay(dir: &tempfile::TempDir) -> Connection {
        let conn = local_connection(&dir.path().join("relay.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        create_ops_index(&conn).await;
        conn
    }

    /// The index exactly as scripts/sync-row-entity-tag.sql creates it (plan Task 8).
    async fn create_ops_index(conn: &Connection) {
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_sync_row_entity ON sync_row(device_id, entity_tag, local_seq) \
             WHERE entity_tag IS NOT NULL",
            (),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn no_compaction_without_the_index_until_ops_creates_it() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("relay.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        COMPACTION_INDEX_SEEN.reset();
        let key = GroupKey::generate();
        let store = Store::open_in_memory().unwrap();
        take_compaction_deletes();

        let id = completed_meeting(&store, "device-a");
        push_content(&conn, &key, &store, "device-a").await.unwrap();
        store.set_meeting_title(id, "Renamed").unwrap();
        push_content(&conn, &key, &store, "device-a").await.unwrap();
        assert_eq!(take_compaction_deletes(), 0, "no index, no DELETE");
        assert_eq!(meeting_rows(&conn, "device-a").await, 2);
        assert_eq!(
            single_i64(&conn, "SELECT COUNT(*) FROM sync_row WHERE row_type = 'meeting' AND entity_tag IS NOT NULL").await,
            2,
            "rows are tagged even without the index"
        );

        create_ops_index(&conn).await;
        store.set_meeting_title(id, "Renamed again").unwrap();
        push_content(&conn, &key, &store, "device-a").await.unwrap();
        assert_eq!(take_compaction_deletes(), 1, "the next round compacts, no restart");
        assert_eq!(meeting_rows(&conn, "device-a").await, 1);
    }

    #[tokio::test]
    async fn a_known_missing_column_pushes_untagged_without_trying() {
        let dir = tempfile::tempdir().unwrap();
        let conn = compacting_relay(&dir).await;
        // As if this round's ensure_content_schema could not confirm the column.
        ENTITY_TAG_COLUMN_SEEN.reset();
        let store = Store::open_in_memory().unwrap();
        take_compaction_deletes();

        completed_meeting(&store, "device-a");
        push_content(&conn, &GroupKey::generate(), &store, "device-a").await.unwrap();

        assert_eq!(meeting_rows(&conn, "device-a").await, 1);
        assert_eq!(
            single_i64(&conn, "SELECT COUNT(*) FROM sync_row WHERE entity_tag IS NOT NULL").await,
            0,
            "no tagged INSERT was sent"
        );
        assert_eq!(take_compaction_deletes(), 0);
    }

    /// Three tagged chunks against a relay whose DELETE never answers in time: the round
    /// spends one 5 s budget on compaction in total for the pass (not 5 s per chunk), skips the rest, and
    /// still inserts every chunk and saves the cursor. The clock is paused, so the
    /// injected 60 s delay costs no real time.
    #[tokio::test(start_paused = true)]
    async fn a_slow_compaction_spends_one_budget_per_pass() {
        let dir = tempfile::tempdir().unwrap();
        let conn = compacting_relay(&dir).await;
        let store = Store::open_in_memory().unwrap();
        let events: Vec<memory::NewCalendarEventItem> = (0..120)
            .map(|i| memory::NewCalendarEventItem {
                device_id: "dev-a".to_string(),
                source: "google_calendar".to_string(),
                external_id: format!("evt-{i}"),
                ts_unix_ms: 5000 + i,
                title: format!("Event {i}"),
                actors: vec![],
            })
            .collect();
        store.upsert_calendar_event_items(&events).unwrap();
        take_compaction_deletes();
        INJECTED_DELETE_DELAY.with(|d| d.set(Some(std::time::Duration::from_secs(60))));

        let started = tokio::time::Instant::now();
        push_content(&conn, &GroupKey::generate(), &store, "dev-a").await.unwrap();
        let spent = started.elapsed();

        assert!(spent <= COMPACTION_TIMEOUT + std::time::Duration::from_secs(1), "{spent:?}");
        assert_eq!(take_compaction_deletes(), 1, "after the timeout, the round stops compacting");
        assert_eq!(
            single_i64(&conn, "SELECT COUNT(*) FROM sync_row WHERE row_type = 'items' AND entity_tag IS NOT NULL").await,
            120,
            "every chunk is still inserted, tagged"
        );
        assert_eq!(
            single_i64(&conn, "SELECT MAX(local_seq) FROM sync_row WHERE device_id = 'dev-a'").await,
            store.get_sync_push_cursor().unwrap(),
            "the cursor reaches the last chunk"
        );
        assert_eq!(COMPACTION_TIMEOUT, std::time::Duration::from_secs(5));
    }

    #[tokio::test]
    async fn the_compaction_delete_uses_the_ops_index_for_both_lookups() {
        let dir = tempfile::tempdir().unwrap();
        let conn = compacting_relay(&dir).await;
        let sql = compaction_delete_sql(3);
        let args = vec![libsql::Value::Text("x".into()); 4];
        let mut rows = conn.query(&format!("EXPLAIN QUERY PLAN {sql}"), args).await.unwrap();
        let mut plan = Vec::new();
        while let Some(row) = rows.next().await.unwrap() {
            plan.push(row.get::<String>(3).unwrap());
        }
        // A bare "USING COVERING INDEX idx_sync_row_entity" with no key terms walks the whole index.
        let seek = "INDEX idx_sync_row_entity (device_id=? AND entity_tag=?)";
        let seeks = plan.iter().filter(|step| step.contains(seek)).count();
        assert_eq!(seeks, 2, "the outer DELETE and the MAX subquery both seek: {plan:?}");
        assert!(!plan.iter().any(|step| step.contains("SCAN")), "{plan:?}");
    }

    /// The 2026-10 iPhone stall: one point lookup per pending entry (and another for its
    /// missing meeting) ran past SYNC_TIMEOUT every round. The retry pass must batch.
    #[tokio::test]
    async fn three_hundred_pending_entries_are_retried_in_a_few_batched_queries() {
        let dir = tempfile::tempdir().unwrap();
        let group_key = GroupKey::generate();
        let conn = local_connection(&dir.path().join("shared.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        let store_b = Store::open_in_memory().unwrap();
        seed_pending_segments(&conn, &group_key, &store_b, 300).await;

        take_row_queries();
        pull_content(&conn, &group_key, &store_b, "device-b").await.unwrap();
        let queries = take_row_queries();
        assert!(queries <= 5, "300 pending entries took {queries} sync_row queries");
        assert_eq!(store_b.list_sync_pending_dependencies("device-a").unwrap().len(), 300, "still waiting on the meeting");

        let meeting_json = serde_json::to_string(&sample_applied_meeting("Standup")).unwrap();
        seed_content_row(&conn, &group_key, "device-a", 999, ROW_TYPE_MEETING, &meeting_json).await;
        pull_content(&conn, &group_key, &store_b, "device-b").await.unwrap();
        let queries = take_row_queries();
        assert!(queries <= 5, "resolving 300 pending entries took {queries} sync_row queries");
        assert!(store_b.list_sync_pending_dependencies("device-a").unwrap().is_empty());
        assert_eq!(store_b.search_transcript_keyword("pending segment", 500).unwrap().len(), 300);
    }

    /// A spent budget stops the pending pass after one chunk, but never before the forward
    /// pull has saved its cursor. The rest resolve on later rounds.
    #[tokio::test]
    async fn many_pending_entries_never_hold_back_the_forward_cursor() {
        let dir = tempfile::tempdir().unwrap();
        let group_key = GroupKey::generate();
        let conn = local_connection(&dir.path().join("shared.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        let store_b = Store::open_in_memory().unwrap();
        seed_pending_segments(&conn, &group_key, &store_b, 450).await;
        // The meeting arrives in this round's forward fetch, plus a new capture after it.
        let meeting_json = serde_json::to_string(&sample_applied_meeting("Standup")).unwrap();
        seed_content_row(&conn, &group_key, "device-a", 999, ROW_TYPE_MEETING, &meeting_json).await;
        let capture_json = serde_json::to_string(&AppliedCaptureItem {
            ts_unix_ms: 1000,
            app_bundle_id: "com.apple.Safari".to_string(),
            window_title: "Peer Window".to_string(),
            url: None,
            text: "fresh forward row".to_string(),
            hash: 1000,
            source: Some("ax".to_string()),
        })
        .unwrap();
        seed_content_row(&conn, &group_key, "device-a", 1000, ROW_TYPE_CAPTURE_ITEM, &capture_json).await;

        let zero = std::time::Duration::ZERO;
        let pending = |s: &Store| s.list_sync_pending_dependencies("device-a").unwrap().len();
        // A zero budget still handles one forward row per round; two rounds reach 1000.
        for _ in 0..2 {
            pull_content_with_budget(&conn, &group_key, &store_b, "device-b", zero, &PullOptions::default(), false).await.unwrap();
        }
        assert_eq!(store_b.get_sync_cursor("device-a").unwrap(), 1000, "the forward cursor advances");
        assert_eq!(store_b.search_keyword("fresh forward row", 10).unwrap().len(), 1);
        assert_eq!(pending(&store_b), 450 - 2 * PENDING_LOOKUP_CHUNK, "each round retries one chunk");
        pull_content_with_budget(&conn, &group_key, &store_b, "device-b", zero, &PullOptions::default(), false).await.unwrap();
        assert_eq!(pending(&store_b), 0, "a later round drains the rest");
        assert_eq!(store_b.search_transcript_keyword("pending segment", 500).unwrap().len(), 450);
    }

    #[tokio::test]
    async fn pull_content_never_re_applies_an_already_applied_row() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("shared.db");
        let group_key = GroupKey::generate();
        let conn = local_connection(&db_path).await;
        ensure_content_schema(&conn).await.unwrap();

        let meeting_json = serde_json::to_string(&sample_applied_meeting("Once Only")).unwrap();
        seed_content_row(
            &conn,
            &group_key,
            "device-a",
            1,
            ROW_TYPE_MEETING,
            &meeting_json,
        )
        .await;

        let store_b = Store::open_in_memory().unwrap();
        pull_content(&conn, &group_key, &store_b, "device-b")
            .await
            .unwrap();
        // Reset the cursor to force a re-fetch of the same row on the next call —
        // simulates the "cursor capped below a blocked row" scenario without needing
        // a real blocker present.
        store_b.set_sync_cursor("device-a", 0).unwrap();
        pull_content(&conn, &group_key, &store_b, "device-b")
            .await
            .unwrap();

        assert_eq!(
            store_b.recent_meetings(10).unwrap().len(),
            1,
            "re-fetching an already-applied row must not create a duplicate"
        );
    }

    #[tokio::test]
    async fn a_transcript_segment_pulled_before_its_meeting_resolves_once_the_meeting_arrives() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("shared.db");
        let group_key = GroupKey::generate();
        let conn = local_connection(&db_path).await;
        ensure_content_schema(&conn).await.unwrap();

        // The segment (local_seq=2) arrives referencing meeting_local_seq=1, but the
        // meeting itself (local_seq=1) has NOT been seeded yet.
        let segment_payload = serde_json::to_string(&TranscriptSegmentPayload {
            meeting_local_seq: 1,
            source_meeting_id: None,
            segment: AppliedTranscriptSegment {
                start_cs: 0,
                speaker_label: "Alice".to_string(),
                text: "hello from the meeting".to_string(),
            },
        })
        .unwrap();
        seed_content_row(
            &conn,
            &group_key,
            "device-a",
            2,
            ROW_TYPE_TRANSCRIPT_SEGMENT,
            &segment_payload,
        )
        .await;

        let store_b = Store::open_in_memory().unwrap();
        pull_content(&conn, &group_key, &store_b, "device-b")
            .await
            .unwrap();

        assert!(
            store_b
                .resolve_sync_row_map("device-a", 2, ROW_TYPE_TRANSCRIPT_SEGMENT)
                .unwrap()
                .is_none(),
            "a segment pulled before its meeting must stay unresolved"
        );
        assert!(store_b
            .search_transcript_keyword("hello from the meeting", 10)
            .unwrap()
            .is_empty());
        assert_eq!(
            store_b.get_sync_cursor("device-a").unwrap(),
            2,
            "the cursor advances past an unresolved dependency -- it's retried independently via sync_pending_dependency, not by holding the cursor back"
        );
        assert_eq!(
            store_b.list_sync_pending_dependencies("device-a").unwrap(),
            vec![(2, ROW_TYPE_TRANSCRIPT_SEGMENT.to_string())],
            "the unresolved segment must be tracked for retry"
        );

        // Now the meeting arrives.
        let meeting_json = serde_json::to_string(&sample_applied_meeting("Standup")).unwrap();
        seed_content_row(
            &conn,
            &group_key,
            "device-a",
            1,
            ROW_TYPE_MEETING,
            &meeting_json,
        )
        .await;

        pull_content(&conn, &group_key, &store_b, "device-b")
            .await
            .unwrap();

        let local_meeting_id = store_b
            .resolve_sync_row_map("device-a", 1, ROW_TYPE_MEETING)
            .unwrap()
            .expect("meeting should now resolve");
        assert!(
            store_b
                .resolve_sync_row_map("device-a", 2, ROW_TYPE_TRANSCRIPT_SEGMENT)
                .unwrap()
                .is_some(),
            "segment should now resolve"
        );

        let results = store_b
            .search_transcript_keyword("hello from the meeting", 10)
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].meeting_id, local_meeting_id,
            "the segment must attach to B's OWN local meeting id, not A's original one"
        );
        assert_eq!(store_b.get_sync_cursor("device-a").unwrap(), 2);
        assert!(
            store_b.list_sync_pending_dependencies("device-a").unwrap().is_empty(),
            "resolving the segment must clear its pending-dependency entry"
        );
    }

    #[tokio::test]
    async fn pull_content_skips_a_row_that_fails_to_decrypt_without_aborting_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("shared.db");
        let group_key = GroupKey::generate();
        let wrong_key = GroupKey::generate();
        let conn = local_connection(&db_path).await;
        ensure_content_schema(&conn).await.unwrap();

        // A row encrypted under the WRONG key (or by an attacker without any key at
        // all) must be skipped, not abort the whole pull.
        let bad_json = serde_json::to_string(&sample_applied_meeting("Forged")).unwrap();
        seed_content_row(
            &conn,
            &wrong_key,
            "device-a",
            1,
            ROW_TYPE_MEETING,
            &bad_json,
        )
        .await;
        let good_json = serde_json::to_string(&sample_applied_meeting("Real")).unwrap();
        seed_content_row(
            &conn,
            &group_key,
            "device-a",
            2,
            ROW_TYPE_MEETING,
            &good_json,
        )
        .await;

        let store_b = Store::open_in_memory().unwrap();
        pull_content(&conn, &group_key, &store_b, "device-b")
            .await
            .unwrap();

        let meetings = store_b.recent_meetings(10).unwrap();
        assert_eq!(meetings.len(), 1);
        assert_eq!(meetings[0].title, "Real");
    }

    #[tokio::test]
    async fn pull_content_applies_three_lifecycle_updates_as_one_meeting_not_three() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("shared.db");
        let group_key = GroupKey::generate();
        let conn = local_connection(&db_path).await;
        ensure_content_schema(&conn).await.unwrap();

        let store_a = Store::open_in_memory().unwrap();
        let meeting_id = store_a
            .insert_meeting(&memory::NewMeeting {
                device_id: "device-a".to_string(),
                start_unix_ms: 1000,
                end_unix_ms: None,
                title: "Genesys".to_string(),
                attendees: vec![],
                trigger: memory::MeetingTrigger::Calendar,
                audio_path: None,
                status: memory::MeetingStatus::Recording,
            })
            .unwrap();

        // Lifecycle point 1: completion.
        store_a.complete_meeting(meeting_id, 2000, None).unwrap();
        push_content(&conn, &group_key, &store_a, "device-a")
            .await
            .unwrap();

        let store_b = Store::open_in_memory().unwrap();
        pull_content(&conn, &group_key, &store_b, "device-b")
            .await
            .unwrap();
        assert_eq!(
            store_b.recent_meetings(10).unwrap().len(),
            1,
            "after completion, exactly 1 meeting"
        );

        // Lifecycle point 2: transcript + notes arrive, re-stamping local_seq.
        store_a
            .update_meeting_transcript_and_notes(
                meeting_id,
                "/path/transcript.txt",
                Some("some notes"),
            )
            .unwrap();
        push_content(&conn, &group_key, &store_a, "device-a")
            .await
            .unwrap();
        pull_content(&conn, &group_key, &store_b, "device-b")
            .await
            .unwrap();
        let meetings = store_b.recent_meetings(10).unwrap();
        assert_eq!(
            meetings.len(),
            1,
            "after transcript+notes, still exactly 1 meeting — not 2"
        );
        assert_eq!(meetings[0].notes.as_deref(), Some("some notes"));

        // Lifecycle point 3: summary arrives, re-stamping local_seq again.
        store_a
            .update_meeting_summary(
                meeting_id,
                "tl;dr",
                &["decided X".to_string()],
                &[],
                &[],
                "anthropic",
                "claude-haiku-4-5",
            )
            .unwrap();
        push_content(&conn, &group_key, &store_a, "device-a")
            .await
            .unwrap();
        pull_content(&conn, &group_key, &store_b, "device-b")
            .await
            .unwrap();
        let meetings = store_b.recent_meetings(10).unwrap();
        assert_eq!(
            meetings.len(),
            1,
            "after summary, still exactly 1 meeting — not 3 (the original bug)"
        );
        assert_eq!(meetings[0].summary_tldr.as_deref(), Some("tl;dr"));
        assert_eq!(
            meetings[0].notes.as_deref(),
            Some("some notes"),
            "the upsert must be a full-row update — earlier fields survive"
        );
    }

    #[tokio::test]
    async fn pull_content_applies_a_legacy_meeting_row_without_source_meeting_id() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("shared.db");
        let group_key = GroupKey::generate();
        let conn = local_connection(&db_path).await;
        ensure_content_schema(&conn).await.unwrap();

        // A pre-fix build's wire payload: just AppliedMeeting's own fields, no
        // source_meeting_id key at all — MeetingPayload's #[serde(default)] must
        // tolerate this and fall back to a plain insert.
        let legacy_json = serde_json::to_string(&sample_applied_meeting("Legacy Row")).unwrap();
        seed_content_row(
            &conn,
            &group_key,
            "device-a",
            1,
            ROW_TYPE_MEETING,
            &legacy_json,
        )
        .await;

        let store_b = Store::open_in_memory().unwrap();
        pull_content(&conn, &group_key, &store_b, "device-b")
            .await
            .unwrap();

        let meetings = store_b.recent_meetings(10).unwrap();
        assert_eq!(meetings.len(), 1);
        assert_eq!(meetings[0].title, "Legacy Row");
    }

    #[tokio::test]
    async fn transcript_segment_resolves_via_source_meeting_id_even_with_a_stale_meeting_local_seq()
    {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("shared.db");
        let group_key = GroupKey::generate();
        let conn = local_connection(&db_path).await;
        ensure_content_schema(&conn).await.unwrap();

        // Meeting pushed once at local_seq=1 (source_meeting_id=100).
        let meeting_payload = MeetingPayload {
            source_meeting_id: Some(100),
            meeting: sample_applied_meeting("Standup"),
        };
        seed_content_row(
            &conn,
            &group_key,
            "device-a",
            1,
            ROW_TYPE_MEETING,
            &serde_json::to_string(&meeting_payload).unwrap(),
        )
        .await;

        let store_b = Store::open_in_memory().unwrap();
        pull_content(&conn, &group_key, &store_b, "device-b")
            .await
            .unwrap();
        let local_meeting_id = store_b
            .resolve_sync_meeting_map("device-a", 100)
            .unwrap()
            .unwrap();

        // Meeting re-pushed at local_seq=20 (same source_meeting_id=100) — simulates a
        // later lifecycle re-stamp landing before the segment below.
        seed_content_row(
            &conn,
            &group_key,
            "device-a",
            20,
            ROW_TYPE_MEETING,
            &serde_json::to_string(&meeting_payload).unwrap(),
        )
        .await;

        // The segment's own baked-in `meeting_local_seq` is deliberately stale (5 — a
        // snapshot value that was never independently pushed as its own row), but
        // `source_meeting_id` still resolves it correctly.
        let segment_payload = TranscriptSegmentPayload {
            meeting_local_seq: 5,
            source_meeting_id: Some(100),
            segment: AppliedTranscriptSegment {
                start_cs: 0,
                speaker_label: "Alice".to_string(),
                text: "hello again".to_string(),
            },
        };
        seed_content_row(
            &conn,
            &group_key,
            "device-a",
            21,
            ROW_TYPE_TRANSCRIPT_SEGMENT,
            &serde_json::to_string(&segment_payload).unwrap(),
        )
        .await;

        pull_content(&conn, &group_key, &store_b, "device-b")
            .await
            .unwrap();

        let results = store_b
            .search_transcript_keyword("hello again", 10)
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].meeting_id, local_meeting_id,
            "must resolve to the SAME local meeting id despite the stale meeting_local_seq"
        );
    }

    // ---- Slice 3: items-format cutover (capability marker, push gate, pull routing) ----

    #[tokio::test]
    async fn push_content_always_announces_the_capability_marker() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("shared.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        let store = Store::open_in_memory().unwrap();
        let group_key = GroupKey::generate();

        push_content(&conn, &group_key, &store, "dev-a")
            .await
            .unwrap();

        assert_eq!(marker_count(&conn, "dev-a").await, 1);
    }

    #[tokio::test]
    async fn push_content_marker_is_idempotent_across_multiple_rounds() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("shared.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        let store = Store::open_in_memory().unwrap();
        let group_key = GroupKey::generate();

        push_content(&conn, &group_key, &store, "dev-a")
            .await
            .unwrap();
        push_content(&conn, &group_key, &store, "dev-a")
            .await
            .unwrap();

        assert_eq!(
            marker_count(&conn, "dev-a").await,
            1,
            "must never insert a second marker row"
        );
    }

    #[tokio::test]
    async fn push_content_stays_on_old_format_until_peer_announces_capability() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("shared.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        let store = Store::open_in_memory().unwrap();
        let group_key = GroupKey::generate();

        // Evidence of a peer device that has pushed old-format content but no
        // items_capability marker -- simulates a peer still on pre-Slice-3 code.
        let peer_json = serde_json::to_string(&AppliedCaptureItem {
            ts_unix_ms: 500,
            app_bundle_id: "com.peer".to_string(),
            window_title: "W".to_string(),
            url: None,
            text: "peer text".to_string(),
            hash: 1,
            source: None,
        })
        .unwrap();
        seed_content_row(
            &conn,
            &group_key,
            "dev-peer",
            1,
            ROW_TYPE_CAPTURE_ITEM,
            &peer_json,
        )
        .await;

        store
            .insert_capture_item(&sample_new_capture_item("local text"))
            .unwrap();

        push_content(&conn, &group_key, &store, "dev-a")
            .await
            .unwrap();

        assert_eq!(
            latest_pushed_row_type(&conn, "dev-a").await.as_deref(),
            Some(ROW_TYPE_CAPTURE_ITEM),
            "must not switch to items-format until the peer's marker is observed"
        );
        // insert_capture_item dual-writes an items row sharing the same local_seq, so
        // the assertion above alone would still pass if BOTH formats were pushed. This
        // is what "must not switch" actually means, and what catches a loosened filter
        // in push_content's pre-cutover branch.
        let mut rows = conn
            .query(
                "SELECT COUNT(*) FROM sync_row WHERE device_id = 'dev-a' AND row_type = ?1",
                params![ROW_TYPE_ITEMS],
            )
            .await
            .unwrap();
        let items_count: i64 = rows.next().await.unwrap().unwrap().get(0).unwrap();
        assert_eq!(
            items_count, 0,
            "no items-format row may be pushed before the cutover"
        );
    }

    #[tokio::test]
    async fn push_content_switches_to_items_format_once_peer_capability_is_observed() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("shared.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        let store = Store::open_in_memory().unwrap();
        let group_key = GroupKey::generate();

        seed_peer_capability_marker(&conn, "dev-peer").await;
        store
            .insert_capture_item(&sample_new_capture_item("local text"))
            .unwrap();

        push_content(&conn, &group_key, &store, "dev-a")
            .await
            .unwrap();

        assert_eq!(
            latest_pushed_row_type(&conn, "dev-a").await.as_deref(),
            Some(ROW_TYPE_ITEMS)
        );
    }

    #[tokio::test]
    async fn pull_content_applies_a_pushed_capture_kind_item_row() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("shared.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        let group_key = GroupKey::generate();

        let pusher_store = Store::open_in_memory().unwrap();
        seed_peer_capability_marker(&conn, "dev-b").await;
        pusher_store
            .insert_capture_item(&sample_new_capture_item("pushed capture text"))
            .unwrap();
        push_content(&conn, &group_key, &pusher_store, "dev-a")
            .await
            .unwrap();

        let puller_store = Store::open_in_memory().unwrap();
        pull_content(&conn, &group_key, &puller_store, "dev-b")
            .await
            .unwrap();

        let results = puller_store
            .search_keyword("pushed capture text", 10)
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].device_id, "dev-a");
    }

    #[tokio::test]
    async fn pull_content_resolves_transcript_kind_item_thread_id_against_a_synced_meeting() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("shared.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        let group_key = GroupKey::generate();

        let pusher_store = Store::open_in_memory().unwrap();
        seed_peer_capability_marker(&conn, "dev-b").await;

        let meeting_id = pusher_store
            .insert_meeting(&memory::NewMeeting {
                device_id: "dev-a".to_string(),
                start_unix_ms: 1000,
                end_unix_ms: None,
                title: "Standup".to_string(),
                attendees: vec![],
                trigger: memory::MeetingTrigger::Calendar,
                audio_path: None,
                status: memory::MeetingStatus::Recording,
            })
            .unwrap();
        pusher_store
            .insert_transcript_segments(&[memory::NewTranscriptSegment {
                meeting_id,
                start_cs: 0,
                speaker_label: "Me".to_string(),
                text: "hello from a synced meeting".to_string(),
            }])
            .unwrap();
        pusher_store
            .complete_meeting(meeting_id, 2000, None)
            .unwrap();

        push_content(&conn, &group_key, &pusher_store, "dev-a")
            .await
            .unwrap();

        let puller_store = Store::open_in_memory().unwrap();
        pull_content(&conn, &group_key, &puller_store, "dev-b")
            .await
            .unwrap();

        // search_transcript_keyword INNER JOINs meeting ON meeting.id = items.thread_id,
        // so a NULL thread_id would make this row invisible -- len()==1 is itself the
        // proof that apply_synced_item was never called with a NULL thread_id.
        let local_meeting_id = puller_store.recent_meetings(10).unwrap()[0].id;
        let results = puller_store
            .search_transcript_keyword("hello from a synced meeting", 10)
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].meeting_id, local_meeting_id);
    }

    #[tokio::test]
    async fn pull_content_defers_a_transcript_kind_item_until_its_meeting_arrives() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("shared.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        let group_key = GroupKey::generate();

        // A transcript-kind items row whose parent meeting has NOT been seeded yet.
        // Its local_seq is BELOW the meeting's, which is the only ordering that can
        // occur in practice: push_content pushes a meeting and its transcript items in
        // the same round, and complete_meeting re-stamps the meeting's local_seq after
        // every segment was inserted -- so a meeting always lands at a higher local_seq
        // than its own segments. That matters because an items row carries no
        // meeting_local_seq, so it can only cap the cursor at its OWN local_seq - 1.
        let payload = ItemPayload {
            source_meeting_id: Some(100),
            meeting_local_seq: None,
            item: memory::AppliedItem {
                kind: "transcript".to_string(),
                source: None,
                ts_unix_ms: 1000,
                actors: Some("[\"Me\"]".to_string()),
                title: None,
                body_text: "orphan candidate".to_string(),
                refs: None,
                flags: None,
                hash: None,
                external_id: None,
                app_bundle_id: None,
                url: None,
            },
        };
        seed_content_row(
            &conn,
            &group_key,
            "dev-a",
            1,
            ROW_TYPE_ITEMS,
            &serde_json::to_string(&payload).unwrap(),
        )
        .await;

        let store_b = Store::open_in_memory().unwrap();
        pull_content(&conn, &group_key, &store_b, "dev-b")
            .await
            .unwrap();

        assert!(
            store_b.resolve_sync_row_map("dev-a", 1, ROW_TYPE_ITEMS).unwrap().is_none(),
            "a transcript-kind items row pulled before its meeting must stay unapplied, never inserted with a NULL thread_id"
        );
        assert!(store_b
            .search_transcript_keyword("orphan candidate", 10)
            .unwrap()
            .is_empty());
        assert_eq!(
            store_b.get_sync_cursor("dev-a").unwrap(),
            1,
            "the cursor advances past an unresolved dependency -- it's retried independently via sync_pending_dependency"
        );
        assert_eq!(
            store_b.list_sync_pending_dependencies("dev-a").unwrap(),
            vec![(1, ROW_TYPE_ITEMS.to_string())],
            "the unresolved transcript-kind item must be tracked for retry"
        );

        // Now the meeting arrives, under the same source_meeting_id.
        let meeting_payload = MeetingPayload {
            source_meeting_id: Some(100),
            meeting: sample_applied_meeting("Standup"),
        };
        seed_content_row(
            &conn,
            &group_key,
            "dev-a",
            2,
            ROW_TYPE_MEETING,
            &serde_json::to_string(&meeting_payload).unwrap(),
        )
        .await;

        pull_content(&conn, &group_key, &store_b, "dev-b")
            .await
            .unwrap();

        let local_meeting_id = store_b
            .resolve_sync_meeting_map("dev-a", 100)
            .unwrap()
            .unwrap();
        let results = store_b
            .search_transcript_keyword("orphan candidate", 10)
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].meeting_id, local_meeting_id);
        assert_eq!(store_b.get_sync_cursor("dev-a").unwrap(), 2);
    }

    #[tokio::test]
    async fn pull_content_never_re_applies_an_already_applied_item_row() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("shared.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        let group_key = GroupKey::generate();

        let pusher_store = Store::open_in_memory().unwrap();
        seed_peer_capability_marker(&conn, "dev-b").await;
        pusher_store
            .insert_capture_item(&sample_new_capture_item("exactly once"))
            .unwrap();
        push_content(&conn, &group_key, &pusher_store, "dev-a")
            .await
            .unwrap();

        let store_b = Store::open_in_memory().unwrap();
        pull_content(&conn, &group_key, &store_b, "dev-b")
            .await
            .unwrap();
        // Force a re-fetch of the same row, as a cursor rollback from a blocked row would.
        store_b.set_sync_cursor("dev-a", 0).unwrap();
        pull_content(&conn, &group_key, &store_b, "dev-b")
            .await
            .unwrap();

        assert_eq!(
            store_b.search_keyword("exactly once", 10).unwrap().len(),
            1,
            "re-fetching an already-applied items row must not double-insert"
        );
    }

    /// The stall the first round of this task shipped: a transcript-kind items row whose
    /// parent meeting was applied from a LEGACY envelope (no `source_meeting_id`, so
    /// `apply_parent_row` never recorded a `sync_meeting_map` entry) has nothing for
    /// `resolve_sync_meeting_map` to hit. Without the `meeting_local_seq` fallback it
    /// pins the peer's cursor at `item_seq - 1` forever, freezing the entire pull stream
    /// from that peer. Reachable via `backfill_transcript_segments_batch`, which mints
    /// transcript items for an already-completed, already-pushed meeting without
    /// re-stamping `meeting.local_seq` -- so the items land ABOVE the meeting's stamp
    /// and the meeting is not re-pushed alongside them. Modelled here exactly.
    #[tokio::test]
    async fn a_transcript_kind_item_resolves_via_meeting_local_seq_when_the_meeting_was_applied_legacy(
    ) {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("shared.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        let group_key = GroupKey::generate();

        let pusher_store = Store::open_in_memory().unwrap();
        seed_peer_capability_marker(&conn, "dev-b").await;

        let meeting_id = pusher_store
            .insert_meeting(&memory::NewMeeting {
                device_id: "dev-a".to_string(),
                start_unix_ms: 1000,
                end_unix_ms: None,
                title: "Backfilled".to_string(),
                attendees: vec![],
                trigger: memory::MeetingTrigger::Calendar,
                audio_path: None,
                status: memory::MeetingStatus::Recording,
            })
            .unwrap();
        // Completed and pushed FIRST -- then the transcript arrives, exactly as
        // backfill_transcript_segments_batch does it. No meeting local_seq re-stamp
        // follows, so the item's local_seq ends up above the meeting's.
        pusher_store
            .complete_meeting(meeting_id, 2000, None)
            .unwrap();
        pusher_store
            .insert_transcript_segments(&[memory::NewTranscriptSegment {
                meeting_id,
                start_cs: 0,
                speaker_label: "Me".to_string(),
                text: "backfilled transcript line".to_string(),
            }])
            .unwrap();

        push_content(&conn, &group_key, &pusher_store, "dev-a")
            .await
            .unwrap();

        let meeting_seq = single_i64(
            &conn,
            "SELECT local_seq FROM sync_row WHERE device_id = 'dev-a' AND row_type = 'meeting'",
        )
        .await;
        let item_seq = single_i64(
            &conn,
            "SELECT local_seq FROM sync_row WHERE device_id = 'dev-a' AND row_type = 'items'",
        )
        .await;
        assert!(
            item_seq > meeting_seq,
            "backfill puts the transcript item above the meeting's stamp"
        );

        // Rewrite the pushed meeting envelope as a pre-source_meeting_id one, so the
        // puller applies it the legacy way and records NO sync_meeting_map entry.
        conn.execute(
            "DELETE FROM sync_row WHERE device_id = 'dev-a' AND row_type = 'meeting'",
            params![],
        )
        .await
        .unwrap();
        let legacy_json = serde_json::to_string(&sample_applied_meeting("Backfilled")).unwrap();
        seed_content_row(
            &conn,
            &group_key,
            "dev-a",
            meeting_seq,
            ROW_TYPE_MEETING,
            &legacy_json,
        )
        .await;

        let store_b = Store::open_in_memory().unwrap();
        pull_content(&conn, &group_key, &store_b, "dev-b")
            .await
            .unwrap();

        assert!(
            store_b.resolve_sync_meeting_map("dev-a", meeting_id).unwrap().is_none(),
            "precondition: a legacy meeting envelope leaves sync_meeting_map empty, so source_meeting_id cannot resolve"
        );
        let local_meeting_id = store_b
            .resolve_sync_row_map("dev-a", meeting_seq, ROW_TYPE_MEETING)
            .unwrap()
            .unwrap();
        let results = store_b
            .search_transcript_keyword("backfilled transcript line", 10)
            .unwrap();
        assert_eq!(
            results.len(),
            1,
            "the item must resolve via the meeting_local_seq fallback, not stall"
        );
        assert_eq!(results[0].meeting_id, local_meeting_id);
        assert_eq!(
            store_b.get_sync_cursor("dev-a").unwrap(),
            item_seq,
            "the cursor must advance -- a stall here freezes every later row from this peer"
        );
    }

    #[tokio::test]
    async fn a_deleted_peer_meeting_returns_when_the_peer_syncs_an_update() {
        let dir = tempfile::tempdir().unwrap();
        let conn = compacting_relay(&dir).await;
        let key = GroupKey::generate();
        let store_a = Store::open_in_memory().unwrap();
        let store_b = Store::open_in_memory().unwrap();

        let id = completed_meeting(&store_a, "device-a");
        store_a
            .insert_transcript_segments(&[memory::NewTranscriptSegment {
                meeting_id: id,
                start_cs: 0,
                speaker_label: "Me".to_string(),
                text: "selfrepair transcript line".to_string(),
            }])
            .unwrap();
        push_content(&conn, &key, &store_a, "device-a").await.unwrap();
        pull_content(&conn, &key, &store_b, "device-b").await.unwrap();
        let old_local = store_b.recent_meetings(10).unwrap()[0].id;
        assert_eq!(store_b.search_transcript_keyword("selfrepair", 10).unwrap().len(), 1);

        store_b.delete_meeting(old_local).unwrap();
        assert!(store_b.recent_meetings(10).unwrap().is_empty());

        store_a
            .update_meeting_transcript_and_notes(id, "/path/transcript.txt", Some("later notes"))
            .unwrap();
        push_content(&conn, &key, &store_a, "device-a").await.unwrap();
        pull_content(&conn, &key, &store_b, "device-b").await.unwrap();

        let meetings = store_b.recent_meetings(10).unwrap();
        assert_eq!(meetings.len(), 1, "the deleted meeting comes back");
        assert_eq!(meetings[0].notes.as_deref(), Some("later notes"));
        let new_local = meetings[0].id;
        assert_ne!(new_local, old_local);
        assert_eq!(
            store_b.resolve_sync_meeting_map("device-a", id).unwrap(),
            Some(new_local),
            "the map is repointed"
        );

        // Its transcript lines re-apply when pulled again (a cursor rewind).
        store_b.set_sync_cursor("device-a", 0).unwrap();
        pull_content(&conn, &key, &store_b, "device-b").await.unwrap();
        let hits = store_b.search_transcript_keyword("selfrepair", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].meeting_id, new_local);
        assert_eq!(store_b.recent_meetings(10).unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_legacy_meeting_applied_twice_without_its_stamp_stays_one_row() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("shared.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        let key = GroupKey::generate();
        let meeting = sample_applied_meeting("Legacy");
        let legacy_json = serde_json::to_string(&meeting).unwrap();
        seed_content_row(&conn, &key, "device-a", 1, ROW_TYPE_MEETING, &legacy_json).await;
        // Already applied earlier, but its sync_row_map stamp is gone.
        let store_b = Store::open_in_memory().unwrap();
        store_b.apply_synced_meeting("device-a", 99, None, &meeting).unwrap();
        pull_content(&conn, &key, &store_b, "device-b").await.unwrap();
        store_b.set_sync_cursor("device-a", 0).unwrap();
        pull_content(&conn, &key, &store_b, "device-b").await.unwrap();
        assert_eq!(store_b.recent_meetings(10).unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_lagging_puller_applies_the_newest_meeting_after_compaction() {
        let dir = tempfile::tempdir().unwrap();
        let conn = compacting_relay(&dir).await;
        let key = GroupKey::generate();
        let store_a = Store::open_in_memory().unwrap();
        let store_b = Store::open_in_memory().unwrap();

        let id = completed_meeting(&store_a, "device-a");
        push_content(&conn, &key, &store_a, "device-a").await.unwrap();
        pull_content(&conn, &key, &store_b, "device-b").await.unwrap();
        store_a
            .update_meeting_transcript_and_notes(id, "/path/transcript.txt", Some("some notes"))
            .unwrap();
        push_content(&conn, &key, &store_a, "device-a").await.unwrap();
        store_a
            .update_meeting_summary(id, "tl;dr", &[], &[], &[], "anthropic", "claude-haiku-4-5")
            .unwrap();
        push_content(&conn, &key, &store_a, "device-a").await.unwrap();
        assert_eq!(meeting_rows(&conn, "device-a").await, 1, "precondition: the older versions were compacted");

        pull_content(&conn, &key, &store_b, "device-b").await.unwrap();

        let meetings = store_b.recent_meetings(10).unwrap();
        assert_eq!(meetings.len(), 1);
        assert_eq!(meetings[0].notes.as_deref(), Some("some notes"));
        assert_eq!(meetings[0].summary_tldr.as_deref(), Some("tl;dr"));
        assert_eq!(store_b.get_sync_cursor("device-a").unwrap(), store_a.get_sync_push_cursor().unwrap());
    }

    #[tokio::test]
    async fn a_lagging_puller_gets_the_newest_calendar_event_title() {
        let dir = tempfile::tempdir().unwrap();
        let conn = compacting_relay(&dir).await;
        let key = GroupKey::generate();
        let store_a = Store::open_in_memory().unwrap();
        let store_b = Store::open_in_memory().unwrap();
        let mut event = memory::NewCalendarEventItem {
            device_id: "device-a".to_string(),
            source: "google_calendar".to_string(),
            external_id: "evt-1".to_string(),
            ts_unix_ms: 5000,
            title: "v1".to_string(),
            actors: vec![],
        };

        store_a.upsert_calendar_event_items(std::slice::from_ref(&event)).unwrap();
        push_content(&conn, &key, &store_a, "device-a").await.unwrap();
        pull_content(&conn, &key, &store_b, "device-b").await.unwrap();
        for title in ["v2", "v3"] {
            event.title = title.to_string();
            store_a.upsert_calendar_event_items(std::slice::from_ref(&event)).unwrap();
            push_content(&conn, &key, &store_a, "device-a").await.unwrap();
        }
        assert_eq!(
            single_i64(&conn, "SELECT COUNT(*) FROM sync_row WHERE device_id = 'device-a' AND row_type = 'items'").await,
            1,
            "precondition: v1 and v2 were compacted"
        );
        pull_content(&conn, &key, &store_b, "device-b").await.unwrap();

        let events = store_b.upcoming_calendar_events(0, i64::MAX, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].title, "v3");
    }

    #[tokio::test]
    async fn a_transcript_item_whose_meeting_version_was_compacted_still_resolves() {
        let dir = tempfile::tempdir().unwrap();
        let conn = compacting_relay(&dir).await;
        let key = GroupKey::generate();
        let store_a = Store::open_in_memory().unwrap();
        seed_peer_capability_marker(&conn, "dev-b").await;

        let meeting_id = completed_meeting(&store_a, "dev-a");
        store_a
            .insert_transcript_segments(&[memory::NewTranscriptSegment {
                meeting_id,
                start_cs: 0,
                speaker_label: "Me".to_string(),
                text: "backfilled transcript line".to_string(),
            }])
            .unwrap();
        push_content(&conn, &key, &store_a, "dev-a").await.unwrap();
        let o = single_i64(&conn, "SELECT local_seq FROM sync_row WHERE device_id = 'dev-a' AND row_type = 'meeting'").await;
        store_a
            .update_meeting_summary(meeting_id, "tl;dr", &[], &[], &[], "anthropic", "claude-haiku-4-5")
            .unwrap();
        push_content(&conn, &key, &store_a, "dev-a").await.unwrap();
        assert_eq!(
            single_i64(&conn, &format!("SELECT COUNT(*) FROM sync_row WHERE device_id = 'dev-a' AND local_seq = {o}")).await,
            0,
            "precondition: the meeting version the item points at was compacted"
        );
        assert_eq!(
            single_i64(&conn, "SELECT COUNT(*) FROM sync_row WHERE device_id = 'dev-a' AND row_type = 'items'").await,
            1,
            "precondition: the transcript item is still on the relay"
        );

        let store_b = Store::open_in_memory().unwrap();
        pull_content(&conn, &key, &store_b, "dev-b").await.unwrap();
        pull_content(&conn, &key, &store_b, "dev-b").await.unwrap();

        let n = single_i64(&conn, "SELECT local_seq FROM sync_row WHERE device_id = 'dev-a' AND row_type = 'meeting'").await;
        let local_meeting = store_b.resolve_sync_row_map("dev-a", n, ROW_TYPE_MEETING).unwrap().unwrap();
        let hits = store_b.search_transcript_keyword("backfilled transcript line", 10).unwrap();
        assert_eq!((hits.len(), hits[0].meeting_id), (1, local_meeting));
        assert!(store_b.list_sync_pending_dependencies("dev-a").unwrap().is_empty());
        assert_eq!(store_b.get_sync_cursor("dev-a").unwrap(), n);
    }

    /// The meeting fallback: a NULL-tagged legacy meeting row is never compacted, even
    /// when a later tagged version of the same meeting lands above it.
    #[tokio::test]
    async fn a_legacy_untagged_meeting_row_survives_a_tagged_push() {
        let dir = tempfile::tempdir().unwrap();
        let conn = compacting_relay(&dir).await;
        let key = GroupKey::generate();
        let store_a = Store::open_in_memory().unwrap();
        seed_peer_capability_marker(&conn, "dev-b").await;

        let meeting_id = completed_meeting(&store_a, "dev-a");
        store_a
            .insert_transcript_segments(&[memory::NewTranscriptSegment {
                meeting_id,
                start_cs: 0,
                speaker_label: "Me".to_string(),
                text: "backfilled transcript line".to_string(),
            }])
            .unwrap();
        push_content(&conn, &key, &store_a, "dev-a").await.unwrap();
        let meeting_seq = single_i64(&conn, "SELECT local_seq FROM sync_row WHERE device_id = 'dev-a' AND row_type = 'meeting'").await;
        conn.execute("DELETE FROM sync_row WHERE device_id = 'dev-a' AND row_type = 'meeting'", params![])
            .await
            .unwrap();
        let legacy_json = serde_json::to_string(&sample_applied_meeting("Standup")).unwrap();
        seed_content_row(&conn, &key, "dev-a", meeting_seq, ROW_TYPE_MEETING, &legacy_json).await;

        store_a.set_meeting_title(meeting_id, "Renamed").unwrap();
        push_content(&conn, &key, &store_a, "dev-a").await.unwrap();
        assert_eq!(
            single_i64(&conn, "SELECT COUNT(*) FROM sync_row WHERE device_id = 'dev-a' AND row_type = 'meeting' AND entity_tag IS NOT NULL").await,
            1,
            "precondition: the new version is tagged"
        );
        assert_eq!(
            single_i64(&conn, &format!("SELECT COUNT(*) FROM sync_row WHERE device_id = 'dev-a' AND local_seq = {meeting_seq}")).await,
            1,
            "the untagged legacy row must survive the tagged push"
        );

        let store_c = Store::open_in_memory().unwrap();
        pull_content(&conn, &key, &store_c, "dev-c").await.unwrap();
        pull_content(&conn, &key, &store_c, "dev-c").await.unwrap();

        // Pull applies parents first, so the item resolves through the tagged version's
        // source_meeting_id. The legacy envelope also yields its own copy on C, as it does
        // with no compaction at all: it carries no source_meeting_id to merge on.
        let local_meeting = store_c.resolve_sync_meeting_map("dev-a", meeting_id).unwrap().unwrap();
        let hits = store_c.search_transcript_keyword("backfilled transcript line", 10).unwrap();
        assert_eq!((hits.len(), hits[0].meeting_id), (1, local_meeting));
        assert!(store_c.list_sync_pending_dependencies("dev-a").unwrap().is_empty());
    }

    const DAY_MS: i64 = 24 * 60 * 60 * 1000;

    fn applied_item(kind: &str, ts_unix_ms: i64, body_text: &str) -> memory::AppliedItem {
        memory::AppliedItem {
            kind: kind.to_string(),
            source: Some("ax".to_string()),
            ts_unix_ms,
            actors: None,
            title: Some("W".to_string()),
            body_text: body_text.to_string(),
            refs: None,
            flags: None,
            hash: Some(ts_unix_ms),
            external_id: None,
            app_bundle_id: Some("com.a".to_string()),
            url: None,
        }
    }

    /// Seeds, from `dev-a`: a legacy `capture_item` and an `items` capture-kind row
    /// each 30 days old (seqs 1, 3) and each 1 day old (seqs 2, 4).
    async fn seed_old_and_new_captures(conn: &Connection, group_key: &GroupKey) {
        let now = crate::now_unix_ms();
        for (seq, age_days, text) in [(1, 30, "legacy old capture"), (2, 1, "legacy new capture")] {
            let json = serde_json::to_string(&AppliedCaptureItem {
                ts_unix_ms: now - age_days * DAY_MS,
                app_bundle_id: "com.a".to_string(),
                window_title: "W".to_string(),
                url: None,
                text: text.to_string(),
                hash: seq,
                source: Some("ax".to_string()),
            })
            .unwrap();
            seed_content_row(conn, group_key, "dev-a", seq, ROW_TYPE_CAPTURE_ITEM, &json).await;
        }
        for (seq, age_days, text) in [(3, 30, "items old capture"), (4, 1, "items new capture")] {
            let json = serde_json::to_string(&ItemPayload {
                source_meeting_id: None,
                meeting_local_seq: None,
                item: applied_item("capture", now - age_days * DAY_MS, text),
            })
            .unwrap();
            seed_content_row(conn, group_key, "dev-a", seq, ROW_TYPE_ITEMS, &json).await;
        }
    }

    fn applied(store: &Store, local_seq: i64, row_type: &str) -> bool {
        store.resolve_sync_row_map("dev-a", local_seq, row_type).unwrap().is_some()
    }

    #[tokio::test]
    async fn capture_items_older_than_the_window_are_skipped_newer_applied() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("shared.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        let group_key = GroupKey::generate();
        seed_old_and_new_captures(&conn, &group_key).await;

        let store = Store::open_in_memory().unwrap();
        let options = PullOptions { capture_window_ms: Some(7 * DAY_MS) };
        pull_content_with_options(&conn, &group_key, &store, "dev-b", &options).await.unwrap();

        assert!(!applied(&store, 1, ROW_TYPE_CAPTURE_ITEM), "legacy capture older than the window");
        assert!(applied(&store, 2, ROW_TYPE_CAPTURE_ITEM), "legacy capture inside the window");
        assert!(!applied(&store, 3, ROW_TYPE_ITEMS), "items capture older than the window");
        assert!(applied(&store, 4, ROW_TYPE_ITEMS), "items capture inside the window");
        assert!(store.search_keyword("legacy old capture", 10).unwrap().is_empty());
        assert!(store.search_keyword("items old capture", 10).unwrap().is_empty());
        assert_eq!(store.search_keyword("legacy new capture", 10).unwrap().len(), 1);
        assert_eq!(store.search_keyword("items new capture", 10).unwrap().len(), 1);
        assert_eq!(
            store.get_sync_cursor("dev-a").unwrap(),
            4,
            "skipped rows still advance the cursor, so they are not re-read every round"
        );
    }

    #[tokio::test]
    async fn cursor_reaches_a_trailing_out_of_window_capture_and_ts_zero_counts_as_old() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("shared.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        let group_key = GroupKey::generate();
        let now = crate::now_unix_ms();
        let legacy = |ts: i64, text: &str, hash: i64| {
            serde_json::to_string(&AppliedCaptureItem {
                ts_unix_ms: ts,
                app_bundle_id: "com.a".to_string(),
                window_title: "W".to_string(),
                url: None,
                text: text.to_string(),
                hash,
                source: Some("ax".to_string()),
            })
            .unwrap()
        };
        let items = |ts: i64, text: &str| {
            serde_json::to_string(&ItemPayload {
                source_meeting_id: None,
                meeting_local_seq: None,
                item: applied_item("capture", ts, text),
            })
            .unwrap()
        };
        // seq 1 is the only applied row; every later row is skipped, and the HIGHEST
        // (seq 5) is a skipped one, so the cursor only reaches it if skips advance it.
        seed_content_row(&conn, &group_key, "dev-a", 1, ROW_TYPE_ITEMS, &items(now - DAY_MS, "new")).await;
        seed_content_row(&conn, &group_key, "dev-a", 2, ROW_TYPE_ITEMS, &items(now - 30 * DAY_MS, "old")).await;
        seed_content_row(&conn, &group_key, "dev-a", 3, ROW_TYPE_ITEMS, &items(0, "items ts zero")).await;
        seed_content_row(&conn, &group_key, "dev-a", 4, ROW_TYPE_CAPTURE_ITEM, &legacy(0, "legacy ts zero", 4)).await;
        seed_content_row(&conn, &group_key, "dev-a", 5, ROW_TYPE_CAPTURE_ITEM, &legacy(now - 30 * DAY_MS, "legacy old", 5)).await;

        let store = Store::open_in_memory().unwrap();
        let options = PullOptions { capture_window_ms: Some(7 * DAY_MS) };
        pull_content_with_options(&conn, &group_key, &store, "dev-b", &options).await.unwrap();

        assert!(applied(&store, 1, ROW_TYPE_ITEMS));
        for (seq, rt) in [(2, ROW_TYPE_ITEMS), (3, ROW_TYPE_ITEMS), (4, ROW_TYPE_CAPTURE_ITEM), (5, ROW_TYPE_CAPTURE_ITEM)] {
            assert!(!applied(&store, seq, rt), "seq {seq} is outside the window");
        }
        assert_eq!(store.get_sync_cursor("dev-a").unwrap(), 5);
    }

    #[tokio::test]
    async fn cursor_reaches_a_trailing_out_of_window_items_format_capture() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("shared.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        let group_key = GroupKey::generate();
        let now = crate::now_unix_ms();
        for (seq, ts, text) in [(1, now - DAY_MS, "new"), (2, now - 30 * DAY_MS, "old")] {
            let json = serde_json::to_string(&ItemPayload {
                source_meeting_id: None,
                meeting_local_seq: None,
                item: applied_item("capture", ts, text),
            })
            .unwrap();
            seed_content_row(&conn, &group_key, "dev-a", seq, ROW_TYPE_ITEMS, &json).await;
        }
        let store = Store::open_in_memory().unwrap();
        let options = PullOptions { capture_window_ms: Some(7 * DAY_MS) };
        pull_content_with_options(&conn, &group_key, &store, "dev-b", &options).await.unwrap();
        assert!(applied(&store, 1, ROW_TYPE_ITEMS));
        assert!(!applied(&store, 2, ROW_TYPE_ITEMS));
        assert_eq!(store.get_sync_cursor("dev-a").unwrap(), 2);
    }

    #[tokio::test]
    async fn meetings_transcript_segments_and_non_capture_items_are_never_windowed() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("shared.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        let group_key = GroupKey::generate();
        let old = crate::now_unix_ms() - 90 * DAY_MS;

        let mut meeting = sample_applied_meeting("Old Standup");
        meeting.start_unix_ms = old;
        meeting.end_unix_ms = Some(old + 1000);
        let meeting_json = serde_json::to_string(&MeetingPayload {
            source_meeting_id: Some(100),
            meeting,
        })
        .unwrap();
        seed_content_row(&conn, &group_key, "dev-a", 1, ROW_TYPE_MEETING, &meeting_json).await;
        let segment_json = serde_json::to_string(&TranscriptSegmentPayload {
            meeting_local_seq: 1,
            source_meeting_id: Some(100),
            segment: memory::AppliedTranscriptSegment {
                start_cs: 0,
                speaker_label: "Me".to_string(),
                text: "old segment line".to_string(),
            },
        })
        .unwrap();
        seed_content_row(&conn, &group_key, "dev-a", 2, ROW_TYPE_TRANSCRIPT_SEGMENT, &segment_json).await;
        let transcript_json = serde_json::to_string(&ItemPayload {
            source_meeting_id: Some(100),
            meeting_local_seq: Some(1),
            item: applied_item("transcript", old, "old transcript item"),
        })
        .unwrap();
        seed_content_row(&conn, &group_key, "dev-a", 3, ROW_TYPE_ITEMS, &transcript_json).await;
        let calendar_json = serde_json::to_string(&ItemPayload {
            source_meeting_id: None,
            meeting_local_seq: None,
            item: applied_item("calendar_event", old, "old calendar event"),
        })
        .unwrap();
        seed_content_row(&conn, &group_key, "dev-a", 4, ROW_TYPE_ITEMS, &calendar_json).await;

        let store = Store::open_in_memory().unwrap();
        let options = PullOptions { capture_window_ms: Some(7 * DAY_MS) };
        pull_content_with_options(&conn, &group_key, &store, "dev-b", &options).await.unwrap();

        assert!(applied(&store, 1, ROW_TYPE_MEETING), "meeting");
        assert!(applied(&store, 2, ROW_TYPE_TRANSCRIPT_SEGMENT), "legacy transcript segment");
        assert!(applied(&store, 3, ROW_TYPE_ITEMS), "transcript-kind item");
        assert!(applied(&store, 4, ROW_TYPE_ITEMS), "calendar_event-kind item");
    }

    #[tokio::test]
    async fn no_window_applies_everything_as_today() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("shared.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        let group_key = GroupKey::generate();
        seed_old_and_new_captures(&conn, &group_key).await;

        let via_default = Store::open_in_memory().unwrap();
        pull_content(&conn, &group_key, &via_default, "dev-b").await.unwrap();
        let via_none = Store::open_in_memory().unwrap();
        pull_content_with_options(&conn, &group_key, &via_none, "dev-b", &PullOptions::default())
            .await
            .unwrap();

        for store in [&via_default, &via_none] {
            assert!(applied(store, 1, ROW_TYPE_CAPTURE_ITEM));
            assert!(applied(store, 2, ROW_TYPE_CAPTURE_ITEM));
            assert!(applied(store, 3, ROW_TYPE_ITEMS));
            assert!(applied(store, 4, ROW_TYPE_ITEMS));
            assert_eq!(store.get_sync_cursor("dev-a").unwrap(), 4);
        }
    }

    #[test]
    fn a_calendar_item_is_identified_by_source_and_external_id() {
        let mut item = applied_item("calendar_event", 1, "standup");
        item.source = Some("google_calendar".to_string());
        item.external_id = Some("evt-1".to_string());
        let row = SyncableRow::Item { local_seq: 1, source_meeting_id: None, meeting_local_seq: None, item };
        assert_eq!(entity_identity(&row), Some("items:calendar_event:google_calendar:evt-1".to_string()));
    }

    #[test]
    fn a_calendar_item_missing_source_or_external_id_is_untagged() {
        for (source, external_id) in [(None, Some("evt-1")), (Some("google_calendar"), None), (None, None)] {
            let mut item = applied_item("calendar_event", 1, "standup");
            item.source = source.map(str::to_string);
            item.external_id = external_id.map(str::to_string);
            let row = SyncableRow::Item { local_seq: 1, source_meeting_id: None, meeting_local_seq: None, item };
            assert_eq!(entity_identity(&row), None, "{source:?} {external_id:?}");
        }
    }

    #[test]
    fn captures_transcripts_and_legacy_rows_are_untagged() {
        for kind in ["capture", "transcript"] {
            let mut item = applied_item(kind, 1, "x");
            item.external_id = Some("e".to_string());
            let row = SyncableRow::Item { local_seq: 1, source_meeting_id: Some(7), meeting_local_seq: Some(2), item };
            assert_eq!(entity_identity(&row), None, "{kind}");
        }
        let capture = SyncableRow::CaptureItem {
            local_seq: 1,
            item: AppliedCaptureItem {
                ts_unix_ms: 1,
                app_bundle_id: "com.a".to_string(),
                window_title: "W".to_string(),
                url: None,
                text: "t".to_string(),
                hash: 1,
                source: Some("ax".to_string()),
            },
        };
        assert_eq!(entity_identity(&capture), None);
        let segment = SyncableRow::TranscriptSegment {
            local_seq: 2,
            meeting_local_seq: 1,
            source_meeting_id: 7,
            segment: AppliedTranscriptSegment { start_cs: 0, speaker_label: "A".to_string(), text: "hi".to_string() },
        };
        assert_eq!(entity_identity(&segment), None);
    }

    #[test]
    fn only_the_five_outbox_types_are_tagged() {
        let op = |row_type: &str, aggregate_id: &str| memory::ClaimedOutboxOperation {
            operation_id: "op".to_string(),
            device_id: "device-a".to_string(),
            local_seq: 1,
            row_type: row_type.to_string(),
            aggregate_type: row_type.to_string(),
            aggregate_id: aggregate_id.to_string(),
            operation_kind: memory::OutboxOperationKind::Upsert,
            payload_json: "{}".to_string(),
            connector: memory::OUTBOX_CONNECTOR.to_string(),
            attempts: 0,
        };
        for (row_type, id, expected) in [
            (ROW_TYPE_MEETING, "42", "meeting:42"),
            (ROW_TYPE_ROUTINE_RUN, "7", "routine_run:7"),
            (ROW_TYPE_MEETING_PREP, "evt-1", "meeting_prep:evt-1"),
            (ROW_TYPE_SKIN, "user-1", "skin:user-1"),
            (ROW_TYPE_SKIN_IMAGE_CHUNK, "abc:0", "skin_image_chunk:abc:0"),
        ] {
            assert_eq!(outbox_entity_identity(&op(row_type, id)), Some(expected.to_string()));
        }
        for row_type in [ROW_TYPE_ITEMS, ROW_TYPE_PROFILE_SUGGESTION] {
            assert_eq!(outbox_entity_identity(&op(row_type, "9")), None, "{row_type}");
        }
    }
}

/// Skin sync over a local relay database (docs/superpowers/plans/2026-10-22-skins-windows-and-sync.md,
/// Task 13): rows go out through the outbox and come in through `pull_content`.
#[cfg(test)]
mod skin_tests {
    use super::*;
    use memory::skin_sync::{sha256_hex, SkinImageChunk, SkinSyncPayload, SKIN_CHUNK_BYTES};

    async fn relay() -> (tempfile::TempDir, Connection, GroupKey) {
        let dir = tempfile::tempdir().unwrap();
        let db = libsql::Builder::new_local(dir.path().join("relay.db")).build().await.unwrap();
        let conn = db.connect().unwrap();
        ensure_content_schema(&conn).await.unwrap();
        (dir, conn, GroupKey::generate())
    }

    fn device(id: &str) -> Store {
        let store = Store::open_in_memory().unwrap();
        store.set_own_device_id(id);
        store
    }

    fn skin(id: &str, at: i64, images: Vec<(String, String)>) -> SkinSyncPayload {
        SkinSyncPayload {
            skin_id: id.into(),
            name: "Stadium".into(),
            manifest: serde_json::json!({ "id": id, "name": "Stadium", "palettes": [format!("{id}-p1")] }),
            palettes: vec![(format!("{id}-p1"), serde_json::json!({ "id": format!("{id}-p1"), "name": "Navy" }))],
            images,
            updated_at_unix_ms: at,
            device_id: "device-a".into(),
            deleted: false,
        }
    }

    /// Saves a skin with one image on device A, as `core/skins` will: the chunks, then the row.
    fn save_on_a(a: &Store, bytes: &[u8]) -> SkinSyncPayload {
        let sha = sha256_hex(bytes);
        a.enqueue_skin_chunks(&sha, bytes).unwrap();
        let payload = skin("user-stadium-1", 100, vec![("lights.png".into(), sha)]);
        a.enqueue_skin_row(&payload).unwrap();
        payload
    }

    async fn rows_of(conn: &Connection, row_type: &str) -> i64 {
        let mut rows = conn
            .query("SELECT COUNT(*) FROM sync_row WHERE row_type = ?1", params![row_type])
            .await
            .unwrap();
        rows.next().await.unwrap().unwrap().get(0).unwrap()
    }

    #[tokio::test]
    async fn skin_row_roundtrips() {
        let (_dir, conn, key) = relay().await;
        let (a, b) = (device("device-a"), device("device-b"));
        let bytes: Vec<u8> = (0..SKIN_CHUNK_BYTES + 77).map(|i| (i % 253) as u8).collect();
        let payload = save_on_a(&a, &bytes);
        crate::outbox::dispatch_outbox(&conn, &key, &a, "device-a").await.unwrap();
        pull_content(&conn, &key, &b, "device-b").await.unwrap();
        assert_eq!(b.synced_skins_ready().unwrap(), vec![payload.clone()]);
        assert_eq!(b.synced_image(&payload.images[0].1).unwrap().as_deref(), Some(&bytes[..]));
        // Pulled again: nothing new, and B writes nothing to the relay.
        pull_content(&conn, &key, &b, "device-b").await.unwrap();
        assert_eq!(b.synced_skins_ready().unwrap().len(), 1);
        let mut rows = conn.query("SELECT COUNT(*) FROM sync_row WHERE device_id = 'device-b'", ()).await.unwrap();
        assert_eq!(rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(), 0);
    }

    /// The puller's LWW rule, across devices (plan ruling 7): compaction keeps each device's
    /// last-enqueued version, and one device only ever enqueues its current state. B pulls
    /// device-a before device-c, so the older version arrives last and must still lose.
    #[tokio::test]
    async fn older_skin_loses_to_newer() {
        let (_dir, conn, key) = relay().await;
        let (a, b, c) = (device("device-a"), device("device-b"), device("device-c"));
        a.enqueue_skin_row(&skin("user-x-1", 200, vec![])).unwrap();
        c.enqueue_skin_row(&SkinSyncPayload { device_id: "device-c".into(), ..skin("user-x-1", 100, vec![]) }).unwrap();
        crate::outbox::dispatch_outbox(&conn, &key, &a, "device-a").await.unwrap();
        crate::outbox::dispatch_outbox(&conn, &key, &c, "device-c").await.unwrap();
        pull_content(&conn, &key, &b, "device-b").await.unwrap();
        let ready = b.synced_skins_ready().unwrap();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].updated_at_unix_ms, 200);
    }

    /// Spec D5: a delete is the newest version, so compaction keeps it and drops what came before.
    #[tokio::test]
    async fn a_skin_tombstone_survives_its_older_versions() {
        let dir = tempfile::tempdir().unwrap();
        let conn = super::tests::compacting_relay(&dir).await;
        let a = device("device-a");
        let saved = save_on_a(&a, &[1, 2, 3]);
        a.enqueue_skin_row(&SkinSyncPayload { deleted: true, updated_at_unix_ms: 200, ..saved }).unwrap();

        crate::outbox::dispatch_outbox(&conn, &GroupKey::generate(), &a, "device-a").await.unwrap();

        assert_eq!(rows_of(&conn, ROW_TYPE_SKIN).await, 1);
        let newest = a.list_outbox_operations().unwrap().into_iter().filter(|r| r.1 == ROW_TYPE_SKIN).map(|r| r.0).max();
        let mut rows = conn.query("SELECT local_seq FROM sync_row WHERE row_type = 'skin'", ()).await.unwrap();
        assert_eq!(rows.next().await.unwrap().unwrap().get::<i64>(0).ok(), newest);
    }

    #[tokio::test]
    async fn image_sent_once_for_two_skins() {
        let (_dir, conn, key) = relay().await;
        let a = device("device-a");
        let bytes = vec![9u8; 1000];
        let sha = sha256_hex(&bytes);
        for id in ["user-a-1", "user-b-1"] {
            a.enqueue_skin_chunks(&sha, &bytes).unwrap();
            a.enqueue_skin_row(&skin(id, 1, vec![("same.png".into(), sha.clone())])).unwrap();
        }
        crate::outbox::dispatch_outbox(&conn, &key, &a, "device-a").await.unwrap();
        assert_eq!(rows_of(&conn, ROW_TYPE_SKIN_IMAGE_CHUNK).await, 1);
        assert_eq!(rows_of(&conn, ROW_TYPE_SKIN).await, 2);
        let b = device("device-b");
        pull_content(&conn, &key, &b, "device-b").await.unwrap();
        assert_eq!(b.synced_skins_ready().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn partial_chunks_hold_skin() {
        let (_dir, conn, key) = relay().await;
        let a = device("device-a");
        let bytes: Vec<u8> = (0..SKIN_CHUNK_BYTES * 2 + 10).map(|i| (i % 241) as u8).collect();
        let payload = save_on_a(&a, &bytes);
        crate::outbox::dispatch_outbox(&conn, &key, &a, "device-a").await.unwrap();
        // The relay is missing the middle chunk (a pass cut short, say): the skin arrives but stays held.
        conn.execute(
            "DELETE FROM sync_row WHERE row_type = 'skin_image_chunk' AND local_seq = (SELECT MIN(local_seq) + 1 FROM sync_row WHERE row_type = 'skin_image_chunk')",
            (),
        )
        .await
        .unwrap();
        let b = device("device-b");
        pull_content(&conn, &key, &b, "device-b").await.unwrap();
        assert!(b.synced_skins_ready().unwrap().is_empty());
        assert_eq!(b.synced_image(&payload.images[0].1).unwrap(), None);
        // The missing piece comes: the image is whole, so the skin is ready.
        let middle = SkinImageChunk {
            sha256: payload.images[0].1.clone(),
            index: 1,
            count: 3,
            data: STANDARD.encode(&bytes[SKIN_CHUNK_BYTES..SKIN_CHUNK_BYTES * 2]),
        };
        b.apply_synced_skin_chunk(&middle).unwrap();
        assert_eq!(b.synced_skins_ready().unwrap(), vec![payload]);
    }

    #[tokio::test]
    async fn chunk_bytes_capped_per_pass() {
        let (_dir, conn, key) = relay().await;
        let a = device("device-a");
        // 30 full chunks, about 10 MB on the relay: more than two passes' worth each way.
        let bytes: Vec<u8> = (0..SKIN_CHUNK_BYTES * 30).map(|i| (i * 7 % 256) as u8).collect();
        let payload = save_on_a(&a, &bytes);
        let t = crate::now_unix_ms() + 1;
        let first = crate::outbox::dispatch_outbox_at(&conn, &key, &a, "device-a", t).await.unwrap();
        let sent = rows_of(&conn, ROW_TYPE_SKIN_IMAGE_CHUNK).await;
        assert!(sent > 0 && sent < 30, "{sent} chunks in one pass");
        assert!(first.deferred > 0);
        let mut at = t;
        while rows_of(&conn, ROW_TYPE_SKIN_IMAGE_CHUNK).await < 30 {
            at += 2;
            crate::outbox::dispatch_outbox_at(&conn, &key, &a, "device-a", at).await.unwrap();
            assert!(at - t < 40, "the chunks never all went");
        }
        assert_eq!(rows_of(&conn, ROW_TYPE_SKIN).await, 1);
        // Pulling is capped the same way: the first pass stops partway, and the skin waits for the rest.
        let b = device("device-b");
        pull_content(&conn, &key, &b, "device-b").await.unwrap();
        let after_one = b.get_sync_cursor("device-a").unwrap();
        assert!(b.synced_skins_ready().unwrap().is_empty());
        let mut passes = 1;
        while b.synced_skins_ready().unwrap().is_empty() {
            pull_content(&conn, &key, &b, "device-b").await.unwrap();
            passes += 1;
            assert!(passes < 10, "the image never completed");
        }
        assert!(passes >= 3, "{passes} passes");
        assert!(after_one < b.get_sync_cursor("device-a").unwrap());
        assert_eq!(b.synced_image(&payload.images[0].1).unwrap().map(|b| b.len()), Some(bytes.len()));
    }

    #[tokio::test]
    async fn capability_marker_triggers_full_repush() {
        let (_dir, conn, key) = relay().await;
        let (a, b) = (device("device-a"), device("device-b"));
        push_content(&conn, &key, &a, "device-a").await.unwrap();
        pull_content(&conn, &key, &b, "device-b").await.unwrap();
        assert!(b.take_skin_repush_request().unwrap(), "a newly seen peer asks for every skin");
        pull_content(&conn, &key, &b, "device-b").await.unwrap();
        assert!(!b.take_skin_repush_request().unwrap(), "once per peer");
        // A peer with rows but no skin marker (a build from before skins synced) asks for nothing.
        let (_dir2, conn2, key2) = relay().await;
        let aad = content_row_aad("device-old", 0, ROW_TYPE_ITEMS_CAPABILITY);
        let ciphertext = sync::encrypt_payload(&key2, b"1", &aad);
        conn2
            .execute(
                "INSERT INTO sync_row (device_id, local_seq, row_type, ciphertext, created_at_unix_ms) VALUES ('device-old', 0, ?1, ?2, 0)",
                params![ROW_TYPE_ITEMS_CAPABILITY, STANDARD.encode(&ciphertext).as_str()],
            )
            .await
            .unwrap();
        let c = device("device-c");
        pull_content(&conn2, &key2, &c, "device-c").await.unwrap();
        assert!(!c.take_skin_repush_request().unwrap());
    }

    #[tokio::test]
    async fn phone_pull_skips_skin_rows() {
        let (_dir, conn, key) = relay().await;
        let a = device("device-a");
        push_content(&conn, &key, &a, "device-a").await.unwrap();
        save_on_a(&a, &[1, 2, 3]);
        crate::outbox::dispatch_outbox(&conn, &key, &a, "device-a").await.unwrap();
        let phone = device("phone");
        crate::pull_only_with_connection(&conn, &key, &phone, "phone", &PullOptions::default())
            .await
            .unwrap();
        assert!(phone.synced_skins_ready().unwrap().is_empty());
        assert!(!phone.take_skin_repush_request().unwrap(), "the phone ignores the skin marker too");
        // A full peer gets them from the same relay.
        let b = device("device-b");
        pull_content(&conn, &key, &b, "device-b").await.unwrap();
        assert_eq!(b.synced_skins_ready().unwrap().len(), 1);
    }

    /// The peer updated after pulling past A's skin with a build that skipped skin rows: its cursor is
    /// beyond the first copies. Its marker asks A to re-push, and A resends the chunks too.
    #[tokio::test]
    async fn a_repush_reaches_a_peer_that_skipped_the_first_copy() {
        let (_dir, conn, key) = relay().await;
        let (a, b) = (device("device-a"), device("device-b"));
        let payload = save_on_a(&a, &[7, 7, 7]);
        crate::outbox::dispatch_outbox(&conn, &key, &a, "device-a").await.unwrap();
        // B, on its old build, pulled past everything A had sent.
        let mut rows = conn.query("SELECT MAX(local_seq) FROM sync_row WHERE device_id = 'device-a'", ()).await.unwrap();
        let sent_through: i64 = rows.next().await.unwrap().unwrap().get(0).unwrap();
        b.set_sync_cursor("device-a", sent_through).unwrap();
        // B updates: it pushes its marker, and A sees it.
        push_content(&conn, &key, &b, "device-b").await.unwrap();
        pull_content(&conn, &key, &a, "device-a").await.unwrap();
        let sha = payload.images[0].1.clone();
        assert!(!a.enqueue_skin_chunks(&sha, &[7, 7, 7]).unwrap(), "sent once already");
        assert!(a.take_skin_repush_request().unwrap());
        // A enqueues every skin again, as core/skins' enqueue_all will: the chunks, then the row.
        assert!(a.enqueue_skin_chunks(&sha, &[7, 7, 7]).unwrap(), "resent for the repush");
        a.enqueue_skin_row(&payload).unwrap();
        crate::outbox::dispatch_outbox(&conn, &key, &a, "device-a").await.unwrap();
        pull_content(&conn, &key, &b, "device-b").await.unwrap();
        assert_eq!(b.synced_skins_ready().unwrap(), vec![payload]);
    }

    /// A large image waits behind its 4 MB budget, but rows enqueued after it still go in the same pass.
    #[tokio::test]
    async fn a_large_image_does_not_hold_up_later_rows() {
        let (_dir, conn, key) = relay().await;
        let a = device("device-a");
        let bytes: Vec<u8> = (0..SKIN_CHUNK_BYTES * 150).map(|i| (i * 13 % 256) as u8).collect();
        save_on_a(&a, &bytes);
        a.enqueue_skin_row(&skin("user-later-1", 5, vec![])).unwrap();
        crate::outbox::dispatch_outbox(&conn, &key, &a, "device-a").await.unwrap();
        let mut rows = conn
            .query("SELECT COUNT(*) FROM sync_row WHERE row_type = 'skin' AND local_seq = (SELECT MAX(local_seq) FROM sync_row)", ())
            .await
            .unwrap();
        assert_eq!(rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(), 1, "the later row went out");
        assert!(rows_of(&conn, ROW_TYPE_SKIN_IMAGE_CHUNK).await < 150);
    }

    #[test]
    fn the_skin_row_types_match_the_phone_query() {
        let sql = "AND row_type NOT IN ('skin', 'skin_image_chunk')";
        assert_eq!(sql, format!("AND row_type NOT IN ('{ROW_TYPE_SKIN}', '{ROW_TYPE_SKIN_IMAGE_CHUNK}')"));
    }

    /// A skin row this build can't read (a newer shape, or a corrupt one) is skipped, and the peer's
    /// later rows still apply, as a build from before skins skips the `skin` rows it has no arm for.
    #[tokio::test]
    async fn old_build_ignores_skin_rows() {
        let (_dir, conn, key) = relay().await;
        let a = device("device-a");
        for (seq, row_type, json) in [
            (1_i64, ROW_TYPE_SKIN, r#"{"skinId":"from-a-newer-build"}"#),
            (2, ROW_TYPE_SKIN_IMAGE_CHUNK, "not json"),
            (3, "skin_v2", "{}"),
        ] {
            let aad = content_row_aad("device-a", seq, row_type);
            let ciphertext = sync::encrypt_payload(&key, json.as_bytes(), &aad);
            conn.execute(
                "INSERT INTO sync_row (device_id, local_seq, row_type, ciphertext, created_at_unix_ms) VALUES (?1, ?2, ?3, ?4, 0)",
                params!["device-a", seq, row_type, STANDARD.encode(&ciphertext).as_str()],
            )
            .await
            .unwrap();
        }
        // Use up A's sequence numbers 1 to 3, which the rows above already hold on the relay (these
        // three are dropped there as duplicates), so its real rows land after them.
        for _ in 0..3 {
            a.enqueue_skin_row(&skin("user-filler-1", 1, vec![])).unwrap();
        }
        save_on_a(&a, &[4, 5, 6]);
        crate::outbox::dispatch_outbox(&conn, &key, &a, "device-a").await.unwrap();
        let b = device("device-b");
        pull_content(&conn, &key, &b, "device-b").await.unwrap();
        let ready: Vec<String> = b.synced_skins_ready().unwrap().into_iter().map(|p| p.skin_id).collect();
        assert!(ready.contains(&"user-stadium-1".to_string()), "{ready:?}");
    }
}

/// Turso bills rows read, so one query that scans `sync_row` costs real money on every round
/// of every device. Each relay query in a sync round has to seek by primary key or index.
#[cfg(test)]
mod relay_query_tests {
    use super::*;

    async fn relay() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let db = libsql::Builder::new_local(dir.path().join("relay.db")).build().await.unwrap();
        let conn = db.connect().unwrap();
        ensure_content_schema(&conn).await.unwrap();
        ensure_entity_tag_column(&conn).await;
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_sync_row_entity ON sync_row(device_id, entity_tag, local_seq) WHERE entity_tag IS NOT NULL",
        )
        .await
        .unwrap();
        (dir, conn)
    }

    async fn assert_seeks(conn: &Connection, sql: &str) {
        let mut rows = conn.query(&format!("EXPLAIN QUERY PLAN {sql}"), ()).await.unwrap();
        let mut steps = 0;
        while let Some(row) = rows.next().await.unwrap() {
            let detail: String = row.get(3).unwrap();
            assert!(!detail.starts_with("SCAN"), "{detail}\nin: {sql}");
            steps += 1;
        }
        assert!(steps > 0, "no plan for: {sql}");
    }

    #[tokio::test]
    async fn no_relay_query_scans_the_table() {
        let (_dir, conn) = relay().await;
        for sql in [NEXT_PEER_SQL, PEER_MARKER_SQL, FETCH_ROWS_SQL, FETCH_ROWS_NO_SKINS_SQL] {
            assert_seeks(&conn, sql).await;
        }
        assert_seeks(&conn, &compaction_delete_sql(3)).await;
        assert_seeks(&conn, "SELECT local_seq, row_type, ciphertext FROM sync_row WHERE device_id = ?1 AND local_seq IN (?2, ?3)").await;
    }

    /// The test itself has to be able to fail.
    #[tokio::test]
    #[should_panic(expected = "SCAN")]
    async fn the_old_full_scan_would_be_caught() {
        let (_dir, conn) = relay().await;
        assert_seeks(&conn, "SELECT row_type, COUNT(*) FROM sync_row GROUP BY 1").await;
    }
}
