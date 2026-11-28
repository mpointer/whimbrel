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
pub(crate) const ROW_TYPE_PROFILE_SUGGESTION_STATUS: &str = "profile_suggestion_status";
pub(crate) const ROW_TYPE_LIVE_ALERT: &str = "live_alert";
pub(crate) const ROW_TYPE_PROVIDER_SPEND: &str = memory::OUTBOX_ROW_TYPE_PROVIDER_SPEND;
/// Speaker names (docs/superpowers/specs/2026-11-05-speaker-names-design.md): people and who each
/// meeting's voices were assigned to.
pub(crate) const ROW_TYPE_PERSON: &str = memory::OUTBOX_ROW_TYPE_PERSON;
pub(crate) const ROW_TYPE_MEETING_SPEAKER: &str = memory::OUTBOX_ROW_TYPE_MEETING_SPEAKER;
pub(crate) const ROW_TYPE_ITEMS_CAPABILITY: &str = "items_capability";
/// Deletion sync (docs/superpowers/specs/2026-11-05-deletion-sync-design.md): "Forget meeting"'s tombstone.
pub(crate) const ROW_TYPE_MEETING_FORGOTTEN: &str = memory::OUTBOX_ROW_TYPE_MEETING_FORGOTTEN;
/// Skin sync (docs/superpowers/plans/2026-10-22-skins-windows-and-sync.md, Task 13): a user skin's
/// version, its images in chunks, and the marker a device pushes once it applies them.
pub(crate) const ROW_TYPE_SKIN: &str = memory::OUTBOX_ROW_TYPE_SKIN;
pub(crate) const ROW_TYPE_SKIN_IMAGE_CHUNK: &str = memory::OUTBOX_ROW_TYPE_SKIN_IMAGE_CHUNK;
pub(crate) const ROW_TYPE_SKINS_CAPABILITY: &str = "skins_capability";
/// Topics (docs/superpowers/plans/2026-11-07-topics-engine.md, Task 7): seed topics and Jev
/// verdicts, sent only once every peer announces `topics_capability` (older builds drop
/// unknown row types, so a late upgrader would miss them for good).
pub(crate) const ROW_TYPE_TOPIC_SEED: &str = memory::OUTBOX_ROW_TYPE_TOPIC_SEED;
pub(crate) const ROW_TYPE_TOPIC_VERDICT: &str = memory::OUTBOX_ROW_TYPE_TOPIC_VERDICT;
pub(crate) const ROW_TYPE_TOPICS_CAPABILITY: &str = "topics_capability";
/// The topic page (docs/superpowers/plans/2026-11-08-topic-page.md, Task 8): steps, step reads,
/// role edits and "Names I go by". Piece-1 builds announce `topics_capability` yet drop these
/// row types, so they wait for every peer to announce `topic_page_capability`.
pub(crate) const ROW_TYPE_TOPIC_STEP: &str = memory::OUTBOX_ROW_TYPE_TOPIC_STEP;
pub(crate) const ROW_TYPE_TOPIC_EXTRACTION: &str = memory::OUTBOX_ROW_TYPE_TOPIC_EXTRACTION;
pub(crate) const ROW_TYPE_TOPIC_ROLE: &str = memory::OUTBOX_ROW_TYPE_TOPIC_ROLE;
pub(crate) const ROW_TYPE_TOPIC_NAMES: &str = memory::OUTBOX_ROW_TYPE_TOPIC_NAMES;
pub(crate) const ROW_TYPE_TOPIC_PAGE_CAPABILITY: &str = "topic_page_capability";
/// The phone-sized topic page snapshot (one row per topic): builds that announce
/// `topic_page_capability` still drop this row type, so it waits for `topic_snapshot_capability`.
pub(crate) const ROW_TYPE_TOPIC_PAGE_SNAPSHOT: &str = memory::OUTBOX_ROW_TYPE_TOPIC_PAGE_SNAPSHOT;
pub(crate) const ROW_TYPE_TOPIC_SNAPSHOT_CAPABILITY: &str = "topic_snapshot_capability";
/// Client controls (docs/superpowers/specs/2026-11-12-client-controls-design.md 4.3, 13.3): a client
/// item's tombstone and a seed topic's purge. Both are sent at once; an older build skips them and
/// reads them again on its first pull once updated (`catch_up_tombstones`). Devices on the relay
/// without `client_controls_capability` (desktops, and phones from the build that announces it) are
/// the receipt's "N devices still need to update".
pub(crate) const ROW_TYPE_CLIENT_ITEM_FORGOTTEN: &str =
    memory::OUTBOX_ROW_TYPE_CLIENT_ITEM_FORGOTTEN;
pub(crate) const ROW_TYPE_TOPIC_SEED_PURGED: &str = memory::OUTBOX_ROW_TYPE_TOPIC_SEED_PURGED;
pub(crate) const ROW_TYPE_CLIENT_CONTROLS_CAPABILITY: &str = "client_controls_capability";
/// Written at `local_seq` 0 under a retired device's id by a desktop whose user chose "Remove this device":
/// every device then leaves it out of the devices-to-update count. Read by that count only; the marker gates
/// (`all_peers_have_marker`) still count the device, and nothing is ever deleted for it.
pub(crate) const ROW_TYPE_DEVICE_RETIRED: &str = "device_retired";
/// The `kind_tag` backfill's advisory claim (`kind_backfill`): written at `local_seq` 0 under the claiming
/// device's own id, its `created_at_unix_ms` refreshed every step. Like every marker it is never pulled (pulls
/// read `local_seq > cursor`, cursor from 0) and no query reads it by `row_type` alone.
pub(crate) const ROW_TYPE_KIND_BACKFILL_CLAIM: &str = "kind_backfill_claim";
/// The `slice_tag` backfill's advisory claim (`slice_backfill`, the cloud runner only): the same kind of row as
/// [`ROW_TYPE_KIND_BACKFILL_CLAIM`], of its own type so the two passes never hold each other.
pub(crate) const ROW_TYPE_SLICE_BACKFILL_CLAIM: &str = "slice_backfill_claim";

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
    /// The recording database's `sync_entity_epoch` (deletion sync); absent from older builds' rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_epoch: Option<String>,
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
            kind_tag TEXT,
            slice_tag TEXT,
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
    if !KIND_TAG_COLUMN_SEEN.get() && ensure_kind_tag_column(conn).await {
        KIND_TAG_COLUMN_SEEN.set();
    }
    if !SLICE_TAG_COLUMN_SEEN.get() && ensure_slice_tag_column(conn).await {
        SLICE_TAG_COLUMN_SEEN.set();
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
    static KIND_COLUMN_SEEN_CELL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static SLICE_COLUMN_SEEN_CELL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Set once this process has confirmed `sync_row.entity_tag`. `push_content` runs after
/// `ensure_content_schema`, so while it is false the column is known missing.
#[cfg(not(test))]
pub(crate) static ENTITY_TAG_COLUMN_SEEN: Latch = Latch(AtomicBool::new(false));
#[cfg(test)]
pub(crate) static ENTITY_TAG_COLUMN_SEEN: Latch = Latch(&COLUMN_SEEN_CELL);

/// Set once this process has confirmed `sync_row.kind_tag`; independent of `entity_tag`.
#[cfg(not(test))]
pub(crate) static KIND_TAG_COLUMN_SEEN: Latch = Latch(AtomicBool::new(false));
#[cfg(test)]
pub(crate) static KIND_TAG_COLUMN_SEEN: Latch = Latch(&KIND_COLUMN_SEEN_CELL);

/// Set once this process has confirmed `sync_row.slice_tag`; independent of the other two tag columns.
#[cfg(not(test))]
pub(crate) static SLICE_TAG_COLUMN_SEEN: Latch = Latch(AtomicBool::new(false));
#[cfg(test)]
pub(crate) static SLICE_TAG_COLUMN_SEEN: Latch = Latch(&SLICE_COLUMN_SEEN_CELL);

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
    ensure_sync_row_column(conn, "entity_tag").await
}

/// The `kind_tag` twin of `ensure_entity_tag_column`: same race rules, same best effort.
pub(crate) async fn ensure_kind_tag_column(conn: &Connection) -> bool {
    ensure_sync_row_column(conn, "kind_tag").await
}

/// The `slice_tag` twin of `ensure_kind_tag_column`: same race rules, same best effort.
pub(crate) async fn ensure_slice_tag_column(conn: &Connection) -> bool {
    ensure_sync_row_column(conn, "slice_tag").await
}

#[cfg(test)]
async fn add_entity_tag_column(conn: &Connection) -> bool {
    add_sync_row_column(conn, "entity_tag").await
}

/// `column` is one of our own constant names, never user input.
pub(crate) async fn ensure_sync_row_column(conn: &Connection, column: &str) -> bool {
    if let Ok(mut rows) = conn.query("PRAGMA table_info(sync_row)", ()).await {
        while let Ok(Some(row)) = rows.next().await {
            if row.get::<String>(1).is_ok_and(|name| name == column) {
                return true;
            }
        }
    }
    add_sync_row_column(conn, column).await
}

async fn add_sync_row_column(conn: &Connection, column: &str) -> bool {
    match conn
        .execute(
            &format!("ALTER TABLE sync_row ADD COLUMN {column} TEXT"),
            (),
        )
        .await
    {
        Ok(_) => true,
        Err(e) if e.to_string().contains("duplicate column name") => true,
        Err(e) => {
            eprintln!("[sync] {column} column not added: {e}");
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

pub(crate) fn encode_row(
    row: &SyncableRow,
) -> Result<(i64, &'static str, String), SyncEngineError> {
    let (local_seq, row_type, json) = match row {
        SyncableRow::CaptureItem { local_seq, item } => (
            *local_seq,
            ROW_TYPE_CAPTURE_ITEM,
            serde_json::to_string(item),
        ),
        SyncableRow::Meeting {
            local_seq,
            source_meeting_id,
            origin_epoch,
            meeting,
        } => {
            let payload = MeetingPayload {
                source_meeting_id: Some(*source_meeting_id),
                origin_epoch: origin_epoch.clone(),
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
        SyncableRow::Meeting {
            source_meeting_id, ..
        } => Some(format!("{ROW_TYPE_MEETING}:{source_meeting_id}")),
        // `source` is a colon-free token, so a `:` inside `external_id` cannot collide.
        SyncableRow::Item { item, .. } if item.kind == "calendar_event" => {
            match (&item.source, &item.external_id) {
                (Some(source), Some(external_id)) => Some(format!(
                    "{ROW_TYPE_ITEMS}:calendar_event:{source}:{external_id}"
                )),
                _ => None,
            }
        }
        _ => None,
    }
}

/// The `kind_tag` for a pushed row: only `items` rows have a kind.
pub(crate) fn kind_tag_for(group_key: &GroupKey, row: &SyncableRow) -> Option<String> {
    match row {
        SyncableRow::Item { item, .. } => Some(sync::kind_tag(group_key, &item.kind)),
        _ => None,
    }
}

/// The backfill twin of `kind_tag_for`: an `items` row's decrypted payload, decoded exactly as the pull decodes it
/// (`ItemPayload`), tagged by its `item.kind`. `None` when the payload does not decode (the pull skips such a row).
pub(crate) fn item_payload_kind_tag(group_key: &GroupKey, plaintext: &[u8]) -> Option<String> {
    let payload = serde_json::from_slice::<ItemPayload>(plaintext).ok()?;
    Some(sync::kind_tag(group_key, &payload.item.kind))
}

/// The `slice_tag` of one item: its kind, source class and calendar month (`sync::slice_tag`).
fn item_slice_tag(
    group_key: &GroupKey,
    kind: &str,
    source: Option<&str>,
    ts_unix_ms: i64,
) -> String {
    sync::slice_tag(
        group_key,
        kind,
        sync::source_class(kind, source),
        sync::month_bucket(ts_unix_ms),
    )
}

/// The `slice_tag` for a pushed row: only `items` rows have one.
pub(crate) fn slice_tag_for(group_key: &GroupKey, row: &SyncableRow) -> Option<String> {
    match row {
        SyncableRow::Item { item, .. } => Some(item_slice_tag(
            group_key,
            &item.kind,
            item.source.as_deref(),
            item.ts_unix_ms,
        )),
        _ => None,
    }
}

/// The backfill twin of `slice_tag_for`: an `items` row's decrypted payload, decoded exactly as the pull decodes it.
/// `None` when the payload does not decode.
pub(crate) fn item_payload_slice_tag(group_key: &GroupKey, plaintext: &[u8]) -> Option<String> {
    let payload = serde_json::from_slice::<ItemPayload>(plaintext).ok()?;
    let item = &payload.item;
    Some(item_slice_tag(
        group_key,
        &item.kind,
        item.source.as_deref(),
        item.ts_unix_ms,
    ))
}

/// The outbox twin of `slice_tag_for`: an `items` op's payload is the item. An op without `ts_unix_ms` is left
/// untagged rather than guessed.
pub(crate) fn outbox_slice_tag(
    group_key: &GroupKey,
    op: &memory::ClaimedOutboxOperation,
) -> Option<String> {
    if op.row_type != ROW_TYPE_ITEMS {
        return None;
    }
    let value = serde_json::from_str::<serde_json::Value>(&op.payload_json).ok()?;
    Some(item_slice_tag(
        group_key,
        value.get("kind")?.as_str()?,
        value.get("source").and_then(|v| v.as_str()),
        value.get("ts_unix_ms")?.as_i64()?,
    ))
}

/// The outbox twin of `kind_tag_for`: an `items` op's payload is the item, so it names its kind.
pub(crate) fn outbox_kind_tag(
    group_key: &GroupKey,
    op: &memory::ClaimedOutboxOperation,
) -> Option<String> {
    if op.row_type != ROW_TYPE_ITEMS {
        return None;
    }
    let value = serde_json::from_str::<serde_json::Value>(&op.payload_json).ok()?;
    Some(sync::kind_tag(group_key, value.get("kind")?.as_str()?))
}

/// The outbox twin of `entity_identity`: each tagged type already enqueues its
/// stable id as `aggregate_id`.
pub(crate) fn outbox_entity_identity(op: &memory::ClaimedOutboxOperation) -> Option<String> {
    match op.row_type.as_str() {
        ROW_TYPE_MEETING
        | ROW_TYPE_ROUTINE_RUN
        | ROW_TYPE_MEETING_PREP
        | ROW_TYPE_SKIN
        | ROW_TYPE_SKIN_IMAGE_CHUNK
        | ROW_TYPE_PERSON
        | ROW_TYPE_MEETING_SPEAKER
        | ROW_TYPE_PROVIDER_SPEND
        | ROW_TYPE_TOPIC_SEED
        | ROW_TYPE_TOPIC_VERDICT
        | ROW_TYPE_TOPIC_STEP
        | ROW_TYPE_TOPIC_EXTRACTION
        | ROW_TYPE_TOPIC_ROLE
        | ROW_TYPE_TOPIC_NAMES
        | ROW_TYPE_TOPIC_PAGE_SNAPSHOT => Some(format!("{}:{}", op.row_type, op.aggregate_id)),
        // Client deletes (`client_item_forgotten`, `topic_seed_purged`) stay untagged, never compacted, like
        // `meeting_forgotten`: a seed row sent after its purge (a lease from a timed-out pass) must not
        // compact the purge away.
        _ => None,
    }
}

/// The `entity_tag` for an identity from `entity_identity` or `outbox_entity_identity`. Both
/// push paths make tags only through here, with `Store::sync_entity_epoch`.
/// The epoch keeps a wiped database's reused ids (same Keychain `device_id`, restarted
/// `local_seq` and meeting ids) from matching an earlier database's rows on the relay.
pub(crate) fn tag_for(
    group_key: &GroupKey,
    device_id: &str,
    epoch: &str,
    identity: &str,
) -> String {
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
    push_marker(conn, group_key, device_id, ROW_TYPE_SKINS_CAPABILITY).await?;
    // Topic rows wait in the outbox until every peer has said this (`dispatch_outbox`).
    push_marker(conn, group_key, device_id, ROW_TYPE_TOPICS_CAPABILITY).await?;
    // Topic page rows wait the same way for this one.
    push_marker(conn, group_key, device_id, ROW_TYPE_TOPIC_PAGE_CAPABILITY).await?;
    // Snapshot rows wait the same way for this one.
    push_marker(
        conn,
        group_key,
        device_id,
        ROW_TYPE_TOPIC_SNAPSHOT_CAPABILITY,
    )
    .await?;
    // Client-delete tombstones and seed purges wait for this one.
    push_marker(
        conn,
        group_key,
        device_id,
        ROW_TYPE_CLIENT_CONTROLS_CAPABILITY,
    )
    .await
}

/// The markers a pull-only device (the phone) announces: every row type the shared pull applies for it. Not
/// skins: the phone leaves them out at the query, and the marker would make peers re-push every skin. Once
/// the phone has any row on the relay, desktops count it as a peer in `all_peers_have_marker`, so it must
/// announce every gating marker it honours, or desktops would hold their topic rows for good. A new
/// `all_peers_have_marker` gate for a row type the phone applies must add its marker here in the same
/// release, or every paired phone (and any retired one, for good) holds those rows back.
pub(crate) const PULL_ONLY_MARKERS: [&str; 5] = [
    ROW_TYPE_ITEMS_CAPABILITY,
    ROW_TYPE_TOPICS_CAPABILITY,
    ROW_TYPE_TOPIC_PAGE_CAPABILITY,
    ROW_TYPE_TOPIC_SNAPSHOT_CAPABILITY,
    ROW_TYPE_CLIENT_CONTROLS_CAPABILITY,
];

/// The pull-only round's only write: [`PULL_ONLY_MARKERS`] at `local_seq` 0, in one statement, so a peer
/// never sees the phone with some of them and not others. Idempotent. Best effort: a failure is logged and
/// the pull goes on (the next round tries again).
pub(crate) async fn announce_pull_only_markers(
    conn: &Connection,
    group_key: &GroupKey,
    device_id: &str,
) {
    if device_id.is_empty() {
        return;
    }
    let now = now_unix_ms();
    let ciphertexts: Vec<String> = PULL_ONLY_MARKERS
        .iter()
        .map(|m| {
            STANDARD.encode(sync::encrypt_payload(
                group_key,
                b"1",
                &content_row_aad(device_id, 0, m),
            ))
        })
        .collect();
    let values = vec!["(?, 0, ?, ?, ?)"; PULL_ONLY_MARKERS.len()].join(", ");
    let sql = format!(
        "INSERT INTO sync_row (device_id, local_seq, row_type, ciphertext, created_at_unix_ms) VALUES {values} \
         ON CONFLICT (device_id, local_seq, row_type) DO NOTHING"
    );
    let mut args: Vec<libsql::Value> = Vec::new();
    for (marker, ciphertext) in PULL_ONLY_MARKERS.iter().zip(&ciphertexts) {
        args.push(device_id.into());
        args.push((*marker).into());
        args.push(ciphertext.as_str().into());
        args.push(now.into());
    }
    if let Err(e) = conn.execute(&sql, args).await {
        eprintln!("[sync] pull-only markers not announced: {e}");
    }
}

/// Stores which other devices have not announced `client_controls_capability`, and how many (the "update
/// needed" line), leaving out devices removed with "Remove this device". On a desktop the outbox pass does
/// this; the phone runs none, so its pull-only round does. Best effort: a failed read or write keeps the last
/// count and never fails the round.
pub(crate) async fn record_pending_tombstone_devices(
    conn: &Connection,
    store: &Store,
    device_id: &str,
) {
    match pending_update_device_ids(conn, device_id).await {
        Ok(ids) => {
            if let Err(e) = store.set_pending_tombstone_device_ids(&ids) {
                eprintln!("[sync] devices-to-update count not stored: {e}");
            }
        }
        Err(e) => eprintln!("[sync] devices-to-update count not read: {e}"),
    }
}

/// [`record_pending_tombstone_devices`] for a desktop's outbox pass, which also sends every client's settings
/// again when a device that still needed to update no longer does: its older build applied the seed records
/// but dropped their control fields, and nothing else would send them again (Nov 19 2026: a PC updated from
/// 0.9.72 never learned an exclusion set on the Mac before its update). Runs before the pass claims rows, so
/// the records go out in the same pass. Best effort, like the count.
pub(crate) async fn record_pending_devices_and_resend_controls(
    conn: &Connection,
    store: &Store,
    device_id: &str,
) {
    let before = store.pending_tombstone_device_ids().ok().flatten();
    match pending_update_device_ids(conn, device_id).await {
        Ok(ids) => {
            if before.is_some_and(|before| before.iter().any(|d| !ids.contains(d))) {
                if let Err(e) = store.requeue_client_control_seeds() {
                    eprintln!("[sync] client settings not sent again to an updated device: {e}");
                }
            }
            if let Err(e) = store.set_pending_tombstone_device_ids(&ids) {
                eprintln!("[sync] devices-to-update count not stored: {e}");
            }
        }
        Err(e) => eprintln!("[sync] devices-to-update count not read: {e}"),
    }
}

/// The OTHER devices in `sync_row` that have not announced `client_controls_capability` and were not removed
/// ([`ROW_TYPE_DEVICE_RETIRED`]).
pub(crate) async fn pending_update_device_ids(
    conn: &Connection,
    own_device_id: &str,
) -> Result<Vec<String>, SyncEngineError> {
    let mut out = Vec::new();
    for peer in distinct_peer_device_ids(conn, own_device_id).await? {
        if peer_has_marker(conn, &peer, ROW_TYPE_CLIENT_CONTROLS_CAPABILITY).await?
            || peer_has_marker(conn, &peer, ROW_TYPE_DEVICE_RETIRED).await?
        {
            continue;
        }
        out.push(peer);
    }
    Ok(out)
}

pub(crate) async fn peer_has_marker(
    conn: &Connection,
    peer: &str,
    marker: &str,
) -> Result<bool, SyncEngineError> {
    if let Some(answer) = round_peers::has_marker(conn, peer, marker).await? {
        return Ok(answer);
    }
    let mut rows = conn
        .query(PEER_MARKER_SQL, params![peer, marker])
        .await
        .map_err(|e| SyncEngineError::Query(e.to_string()))?;
    Ok(rows
        .next()
        .await
        .map_err(|e| SyncEngineError::Query(e.to_string()))?
        .is_some())
}

/// Publishes the removals queued by "Remove this device" ([`memory::Store::queue_retired_device`]): a
/// [`ROW_TYPE_DEVICE_RETIRED`] marker at `local_seq` 0 under each removed device's id. Desktop rounds only (a
/// phone's pull-only round never calls this, so a phone cannot remove a device). A device that has announced
/// the client marker since, or has no rows on the relay any more, is only taken off the queue: a marker would
/// make a device with no rows a peer of every gate. Additive and idempotent; it deletes nothing. Best effort:
/// what fails stays queued for the next round.
pub(crate) async fn publish_retired_devices(
    conn: &Connection,
    group_key: &GroupKey,
    store: &Store,
    own_device_id: &str,
) {
    let queue = match store.retired_device_queue() {
        Ok(q) => q,
        Err(e) => {
            eprintln!("[sync] removed devices not read: {e}");
            return;
        }
    };
    let mut done = Vec::new();
    for id in queue {
        if id == own_device_id {
            done.push(id);
            continue;
        }
        let publish = async {
            if peer_has_marker(conn, &id, ROW_TYPE_CLIENT_CONTROLS_CAPABILITY).await?
                || !peer_has_rows(conn, &id).await?
            {
                return Ok::<(), SyncEngineError>(());
            }
            push_marker(conn, group_key, &id, ROW_TYPE_DEVICE_RETIRED).await?;
            // This round's later gates (the outbox's update count) see the new marker.
            round_peers::forget_markers();
            Ok(())
        };
        match publish.await {
            Ok(()) => done.push(id),
            Err(e) => eprintln!("[sync] a removed device was not published: {e}"),
        }
    }
    if !done.is_empty() {
        if let Err(e) = store.clear_retired_devices(&done) {
            eprintln!("[sync] removed devices not cleared: {e}");
        }
    }
}

async fn peer_has_rows(conn: &Connection, peer: &str) -> Result<bool, SyncEngineError> {
    let mut rows = conn
        .query(
            "SELECT 1 FROM sync_row WHERE device_id = ?1 LIMIT 1",
            params![peer],
        )
        .await
        .map_err(|e| SyncEngineError::Query(e.to_string()))?;
    Ok(rows
        .next()
        .await
        .map_err(|e| SyncEngineError::Query(e.to_string()))?
        .is_some())
}

/// One capability marker at `local_seq` 0, idempotent (see `push_items_capability_marker`).
pub(crate) async fn push_marker(
    conn: &Connection,
    group_key: &GroupKey,
    device_id: &str,
    marker: &str,
) -> Result<(), SyncEngineError> {
    write_marker(conn, group_key, device_id, marker, false).await
}

/// [`push_marker`], but an existing marker's `created_at_unix_ms` is moved to now: a claim that says "still alive"
/// ([`ROW_TYPE_KIND_BACKFILL_CLAIM`]).
pub(crate) async fn refresh_marker(
    conn: &Connection,
    group_key: &GroupKey,
    device_id: &str,
    marker: &str,
) -> Result<(), SyncEngineError> {
    write_marker(conn, group_key, device_id, marker, true).await
}

async fn write_marker(
    conn: &Connection,
    group_key: &GroupKey,
    device_id: &str,
    marker: &str,
    refresh: bool,
) -> Result<(), SyncEngineError> {
    let aad = content_row_aad(device_id, 0, marker);
    let ciphertext = sync::encrypt_payload(group_key, b"1", &aad);
    let on_conflict = if refresh {
        "DO UPDATE SET created_at_unix_ms = excluded.created_at_unix_ms"
    } else {
        "DO NOTHING"
    };
    conn.execute(
        &format!(
            "INSERT INTO sync_row (device_id, local_seq, row_type, ciphertext, created_at_unix_ms) \
             VALUES (?1, 0, ?2, ?3, ?4) \
             ON CONFLICT (device_id, local_seq, row_type) {on_conflict}"
        ),
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
    all_peers_have_marker(conn, own_device_id, ROW_TYPE_ITEMS_CAPABILITY).await
}

/// True once every OTHER device in `sync_row` has pushed `marker` (vacuously true alone).
pub(crate) async fn all_peers_have_marker(
    conn: &Connection,
    own_device_id: &str,
    marker: &str,
) -> Result<bool, SyncEngineError> {
    for peer in distinct_peer_device_ids(conn, own_device_id).await? {
        if !peer_has_marker(conn, &peer, marker).await? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// The OTHER devices in `sync_row` that have not announced `marker` (removed devices too: a marker gate counts
/// every device with rows).
pub(crate) async fn peers_without_marker(
    conn: &Connection,
    own_device_id: &str,
    marker: &str,
) -> Result<Vec<String>, SyncEngineError> {
    let mut out = Vec::new();
    for peer in distinct_peer_device_ids(conn, own_device_id).await? {
        if !peer_has_marker(conn, &peer, marker).await? {
            out.push(peer);
        }
    }
    Ok(out)
}

/// The smallest `device_id` after `?1`, by primary-key seek. Turso bills rows read, and the
/// old `SELECT DISTINCT device_id ... WHERE device_id != ?` read every row of `sync_row`
/// several times per sync round; this reads one row per device.
const NEXT_PEER_SQL: &str =
    "SELECT device_id FROM sync_row WHERE device_id > ?1 ORDER BY device_id LIMIT 1";

/// One desktop round's view of the peers and their `local_seq` 0 markers. The gates in push, pull and the
/// outbox each walked the peers and looked up markers one query at a time: about 45 sequential relay
/// round trips a round, which alone outlasted `SYNC_TIMEOUT` once the relay answered in ~0.5 s (Nov 18
/// 2026). Inside a round (`begin`..`end`, set by `sync_content`) the peers are walked once and every
/// peer's markers are read in one primary-key query. Outside a round nothing is cached, so tests and
/// other callers read the relay exactly as before. Thread-local: a round runs on the thread that
/// `block_on`s it. A marker a peer adds during the round is seen next round.
pub(crate) mod round_peers {
    use super::*;
    use std::cell::RefCell;
    use std::collections::{HashMap, HashSet};

    #[derive(Default)]
    struct Snapshot {
        own: String,
        peers: Option<Vec<String>>,
        markers: Option<HashMap<String, HashSet<String>>>,
    }

    thread_local! {
        static ROUND: RefCell<Option<Snapshot>> = const { RefCell::new(None) };
    }

    /// Starts caching for this thread's round.
    pub(crate) fn begin(own_device_id: &str) {
        ROUND.with(|r| {
            *r.borrow_mut() = Some(Snapshot {
                own: own_device_id.to_string(),
                ..Default::default()
            })
        });
    }

    /// Ends the round: nothing is cached after this.
    pub(crate) fn end() {
        ROUND.with(|r| *r.borrow_mut() = None);
    }

    /// Drops the markers read so far (this round just published one, e.g. a removed device's marker).
    pub(crate) fn forget_markers() {
        ROUND.with(|r| {
            if let Some(s) = r.borrow_mut().as_mut() {
                s.markers = None;
            }
        });
    }

    pub(super) fn peers(own_device_id: &str) -> Option<Vec<String>> {
        ROUND.with(|r| {
            r.borrow()
                .as_ref()
                .filter(|s| s.own == own_device_id)
                .and_then(|s| s.peers.clone())
        })
    }

    pub(super) fn set_peers(own_device_id: &str, ids: &[String]) {
        ROUND.with(|r| {
            if let Some(s) = r.borrow_mut().as_mut().filter(|s| s.own == own_device_id) {
                s.peers = Some(ids.to_vec());
            }
        });
    }

    /// `Some(answer)` from the round's snapshot (read on first use), `None` outside a round or for a device
    /// the snapshot does not cover.
    pub(super) async fn has_marker(
        conn: &Connection,
        peer: &str,
        marker: &str,
    ) -> Result<Option<bool>, SyncEngineError> {
        let Some((own, loaded)) = ROUND.with(|r| {
            r.borrow()
                .as_ref()
                .map(|s| (s.own.clone(), s.markers.is_some()))
        }) else {
            return Ok(None);
        };
        if !loaded {
            let peers = distinct_peer_device_ids(conn, &own).await?;
            let markers = read_markers(conn, &peers).await?;
            ROUND.with(|r| {
                if let Some(s) = r.borrow_mut().as_mut() {
                    s.markers = Some(markers);
                }
            });
        }
        Ok(ROUND.with(|r| {
            r.borrow()
                .as_ref()
                .and_then(|s| s.markers.as_ref())
                .and_then(|m| m.get(peer))
                .map(|set| set.contains(marker))
        }))
    }

    /// Every `local_seq` 0 row type of `peers`, in one query (a primary-key seek per device).
    async fn read_markers(
        conn: &Connection,
        peers: &[String],
    ) -> Result<HashMap<String, HashSet<String>>, SyncEngineError> {
        let mut out: HashMap<String, HashSet<String>> =
            peers.iter().map(|p| (p.clone(), HashSet::new())).collect();
        if peers.is_empty() {
            return Ok(out);
        }
        let placeholders: Vec<String> = (1..=peers.len()).map(|i| format!("?{i}")).collect();
        let sql = format!(
            "SELECT device_id, row_type FROM sync_row WHERE local_seq = 0 AND device_id IN ({})",
            placeholders.join(", ")
        );
        let args: Vec<libsql::Value> = peers
            .iter()
            .map(|p| libsql::Value::Text(p.clone()))
            .collect();
        let mut rows = conn
            .query(&sql, libsql::params::Params::Positional(args))
            .await
            .map_err(|e| SyncEngineError::Query(e.to_string()))?;
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| SyncEngineError::Query(e.to_string()))?
        {
            let device: String = row
                .get(0)
                .map_err(|e| SyncEngineError::Query(e.to_string()))?;
            let row_type: String = row
                .get(1)
                .map_err(|e| SyncEngineError::Query(e.to_string()))?;
            out.entry(device).or_default().insert(row_type);
        }
        Ok(out)
    }
}

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

/// Rows per multi-row INSERT: one network round trip each, not one per row. Up to
/// eight parameters per row keeps a chunk (400) under SQLite's 999-variable floor. It also
/// caps a chunk's distinct tags at 50, the most `compact_entity_tags` takes.
const PUSH_CHUNK_ROWS: usize = 50;

/// One encoded row to insert: `(local_seq, row_type, ciphertext, entity_tag, kind_tag, slice_tag)`.
pub(crate) type PushRow<'a> = (
    i64,
    &'a str,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// One multi-row INSERT of a chunk of encoded rows. `with_entity` / `with_kind` / `with_slice` add those
/// columns (NULL for untagged rows); each is independent, as a relay may have any, all or none.
async fn insert_push_chunk(
    conn: &Connection,
    device_id: &str,
    now: i64,
    rows: &[PushRow<'_>],
    with_entity: bool,
    with_kind: bool,
    with_slice: bool,
) -> Result<u64, libsql::Error> {
    let width = 5 + usize::from(with_entity) + usize::from(with_kind) + usize::from(with_slice);
    let mut placeholders = Vec::with_capacity(rows.len());
    let mut values: Vec<Value> = Vec::with_capacity(rows.len() * width);
    for (i, (local_seq, row_type, ciphertext, entity, kind, slice)) in rows.iter().enumerate() {
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
        if with_entity {
            values.push(entity.clone().map_or(Value::Null, Value::Text));
        }
        if with_kind {
            values.push(kind.clone().map_or(Value::Null, Value::Text));
        }
        if with_slice {
            values.push(slice.clone().map_or(Value::Null, Value::Text));
        }
    }
    let mut columns =
        String::from("device_id, local_seq, row_type, ciphertext, created_at_unix_ms");
    if with_entity {
        columns.push_str(", entity_tag");
    }
    if with_kind {
        columns.push_str(", kind_tag");
    }
    if with_slice {
        columns.push_str(", slice_tag");
    }
    let sql = format!(
        "INSERT INTO sync_row ({columns}) VALUES {} \
         ON CONFLICT (device_id, local_seq, row_type) DO NOTHING",
        placeholders.join(", ")
    );
    conn.execute(&sql, Params::Positional(values)).await
}

/// `insert_push_chunk` that survives a relay missing any tag column: each missing column
/// is dropped from `with_entity` / `with_kind` / `with_slice` (so the caller stops asking for it) and the
/// INSERT retried, ending untagged at worst. Any other error is returned.
pub(crate) async fn insert_with_tag_fallback(
    conn: &Connection,
    device_id: &str,
    now: i64,
    rows: &[PushRow<'_>],
    with_entity: &mut bool,
    with_kind: &mut bool,
    with_slice: &mut bool,
) -> Result<u64, libsql::Error> {
    loop {
        match insert_push_chunk(
            conn,
            device_id,
            now,
            rows,
            *with_entity,
            *with_kind,
            *with_slice,
        )
        .await
        {
            Err(e) if *with_entity && is_missing_entity_tag_column(&e.to_string()) => {
                #[cfg(test)]
                FAILED_TAGGED_INSERTS.with(|c| c.set(c.get() + 1));
                *with_entity = false;
            }
            Err(e) if *with_kind && is_missing_kind_tag_column(&e.to_string()) => {
                #[cfg(test)]
                FAILED_TAGGED_INSERTS.with(|c| c.set(c.get() + 1));
                *with_kind = false;
            }
            Err(e) if *with_slice && is_missing_slice_tag_column(&e.to_string()) => {
                #[cfg(test)]
                FAILED_TAGGED_INSERTS.with(|c| c.set(c.get() + 1));
                *with_slice = false;
            }
            other => return other,
        }
    }
}

fn row_local_seq(row: &SyncableRow) -> i64 {
    match row {
        SyncableRow::CaptureItem { local_seq, .. }
        | SyncableRow::Meeting { local_seq, .. }
        | SyncableRow::TranscriptSegment { local_seq, .. }
        | SyncableRow::Item { local_seq, .. } => *local_seq,
    }
}

/// The own meeting a pushed row belongs to: the meeting itself, or a transcript line of it.
fn row_meeting_id(row: &SyncableRow) -> Option<i64> {
    match row {
        SyncableRow::Meeting {
            source_meeting_id, ..
        }
        | SyncableRow::TranscriptSegment {
            source_meeting_id, ..
        } => Some(*source_meeting_id),
        SyncableRow::Item {
            source_meeting_id, ..
        } => *source_meeting_id,
        SyncableRow::CaptureItem { .. } => None,
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
    // Topic rows written before topic sync (or before this device had an id) are queued once,
    // the first round every peer can apply them. Before the pull, so rows pulled in that
    // round are not sent back.
    if !store
        .topic_sync_backfilled()
        .map_err(|e| SyncEngineError::Query(e.to_string()))?
        && all_peers_have_marker(conn, device_id, ROW_TYPE_TOPICS_CAPABILITY).await?
    {
        store
            .enqueue_topic_sync_backfill()
            .map_err(|e| SyncEngineError::Query(e.to_string()))?;
    }
    if !store
        .topic_page_sync_backfilled()
        .map_err(|e| SyncEngineError::Query(e.to_string()))?
        && all_peers_have_marker(conn, device_id, ROW_TYPE_TOPIC_PAGE_CAPABILITY).await?
    {
        store
            .enqueue_topic_page_sync_backfill()
            .map_err(|e| SyncEngineError::Query(e.to_string()))?;
    }

    let last_pushed = store
        .get_sync_push_cursor()
        .map_err(|e| SyncEngineError::Query(e.to_string()))?;
    let cutover_ready = all_peers_items_capable(conn, device_id).await?;

    // The epoch is created on first use, which writes: do that before the read snapshot below,
    // which must not write. An unreadable epoch never fails a round (the rows go untagged).
    let _ = store.sync_entity_epoch();
    // One snapshot for every read of unpushed rows. The cursor is the highest `local_seq` sent,
    // so a row that commits after a higher one was read (the app's other connection writes while
    // this one syncs) would be skipped for good. Rows commit in `local_seq` order (a `local_seq`
    // is taken and its row written in one transaction), so a single snapshot always holds a
    // prefix of them. No network call happens inside it.
    let rows = store
        .with_read_snapshot(|s| {
            // `meeting` has no `items` representation, so it is pushed in its own format on
            // both sides of the cutover.
            let mut rows = s.unpushed_meeting_rows(device_id, last_pushed)?;
            if cutover_ready {
                rows.extend(s.unpushed_item_rows(device_id, last_pushed)?);
            } else {
                // Pre-cutover: push capture_item/transcript_segment in the old format instead
                // of items. unpushed_rows() also re-selects meeting rows, so drop those here
                // to avoid double-pushing what unpushed_meeting_rows already added above.
                let legacy_rows = s.unpushed_rows(device_id, last_pushed)?;
                rows.extend(
                    legacy_rows
                        .into_iter()
                        .filter(|r| !matches!(r, SyncableRow::Meeting { .. })),
                );
            }
            Ok(rows)
        })
        .map_err(|e| SyncEngineError::Query(e.to_string()))?;
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
    // Same for `kind_tag`, which is independent of `entity_tag`: it needs no epoch.
    let mut kind_on = KIND_TAG_COLUMN_SEEN.get();
    let mut slice_on = SLICE_TAG_COLUMN_SEEN.get();
    let mut compaction = CompactionBudget::default();
    for chunk in rows.chunks(PUSH_CHUNK_ROWS) {
        let now = now_unix_ms();
        let mut encoded = Vec::with_capacity(chunk.len());
        let mut tags: Vec<String> = Vec::new();
        let mut chunk_highest = highest;
        let mut meeting_rows: Vec<(i64, i64, &str)> = Vec::new();
        for row in chunk {
            let (local_seq, row_type, payload_json) = encode_row(row)?;
            if let Some(meeting_id) = row_meeting_id(row) {
                meeting_rows.push((meeting_id, local_seq, row_type));
            }
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
            let kind = kind_on.then(|| kind_tag_for(group_key, row)).flatten();
            let slice = slice_on.then(|| slice_tag_for(group_key, row)).flatten();
            encoded.push((
                local_seq,
                row_type,
                STANDARD.encode(&ciphertext),
                tag,
                kind,
                slice,
            ));
            chunk_highest = chunk_highest.max(local_seq);
        }
        let mut with_entity = !tags.is_empty();
        let mut with_kind = kind_on && encoded.iter().any(|r| r.4.is_some());
        let mut with_slice = slice_on && encoded.iter().any(|r| r.5.is_some());
        let (asked_entity, asked_kind, asked_slice) = (with_entity, with_kind, with_slice);
        let inserted = insert_with_tag_fallback(
            conn,
            device_id,
            now,
            &encoded,
            &mut with_entity,
            &mut with_kind,
            &mut with_slice,
        )
        .await;
        if asked_entity && !with_entity {
            // A relay that predates `entity_tag`: compact nothing, and stop tagging for the
            // rest of this call (as the outbox does).
            tags.clear();
            epoch = None;
        }
        if asked_kind && !with_kind {
            kind_on = false;
        }
        if asked_slice && !with_slice {
            slice_on = false;
        }
        inserted.map_err(|e| SyncEngineError::Query(e.to_string()))?;
        // So forgetting the meeting can delete these rows (deletion sync).
        store
            .record_pushed_meeting_rows(&meeting_rows)
            .map_err(|e| SyncEngineError::Query(e.to_string()))?;
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

pub(crate) async fn distinct_peer_device_ids(
    conn: &Connection,
    own_device_id: &str,
) -> Result<Vec<String>, SyncEngineError> {
    if let Some(ids) = round_peers::peers(own_device_id) {
        return Ok(ids);
    }
    let ids = hop_peer_device_ids(conn, own_device_id).await?;
    round_peers::set_peers(own_device_id, &ids);
    Ok(ids)
}

/// [`distinct_peer_device_ids`] read from the relay, without the round snapshot.
async fn hop_peer_device_ids(
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

/// This device's own `provider_spend` rows on the relay. A PK prefix seek, but it reads
/// every own row, so it runs once per database (see `restore_own_provider_spend`).
const OWN_PROVIDER_SPEND_SQL: &str =
    "SELECT local_seq, row_type, ciphertext FROM sync_row WHERE device_id = ?1 AND row_type = 'provider_spend'";

/// Takes back this device's own month spend from the relay, once per database, so a
/// reinstall (same Keychain device id, new database) doesn't start its months at zero
/// (spec anti-goal: never silently spend past a budget). The peer loop skips the own id.
async fn restore_own_provider_spend(
    conn: &Connection,
    group_key: &GroupKey,
    store: &Store,
    own_device_id: &str,
) -> Result<(), SyncEngineError> {
    if own_device_id.is_empty()
        || store
            .own_provider_spend_restored()
            .map_err(|e| SyncEngineError::Query(e.to_string()))?
    {
        return Ok(());
    }
    let rows = conn
        .query(OWN_PROVIDER_SPEND_SQL, params![own_device_id])
        .await
        .map_err(|e| SyncEngineError::Query(e.to_string()))?;
    for env in read_envelopes(rows).await? {
        let Some(plaintext) = decrypt_row(group_key, own_device_id, &env) else {
            continue;
        };
        let Ok(spend) =
            serde_json::from_slice::<memory::provider_budget::AppliedProviderSpend>(&plaintext)
        else {
            continue;
        };
        store
            .apply_synced_provider_spend(own_device_id, &spend)
            .map_err(|e| SyncEngineError::Query(e.to_string()))?;
    }
    store
        .mark_own_provider_spend_restored()
        .map_err(|e| SyncEngineError::Query(e.to_string()))
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

/// How much one pull round fetches and applies: rows per peer page, time per peer, and
/// (catch-up only) a cap on the whole round's forward passes.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Pace {
    rows: i64,
    peer_budget: std::time::Duration,
    round_budget: Option<std::time::Duration>,
}

/// Every desktop pull, and a pull-only peer that is caught up.
const NORMAL_PACE: Pace = Pace {
    rows: MAX_ROWS_PER_PULL_PASS,
    peer_budget: PULL_PEER_TIME_BUDGET,
    round_budget: None,
};

/// A pull-only peer (the phone) whose last round left rows behind (#572): a first sync
/// of ~3.5M seq per peer took ~2 h at 500 rows a round. Bigger pages and budgets, but
/// the round's forward passes stop starting new work after 20 s, and the pending pass
/// keeps its 3 s, so a round with several busy peers still ends inside `SYNC_TIMEOUT`
/// (30 s). A later peer then gets its one row this round and its full turn once the
/// earlier ones catch up. The page is still one primary-key seek, just a bigger LIMIT.
const CATCH_UP_PACE: Pace = Pace {
    rows: 2000,
    peer_budget: std::time::Duration::from_secs(10),
    round_budget: Some(std::time::Duration::from_secs(20)),
};

/// The pace for a pull-only round, from whether the previous round reported more to pull.
fn pull_only_pace(catching_up: bool) -> Pace {
    if catching_up {
        CATCH_UP_PACE
    } else {
        NORMAL_PACE
    }
}

#[cfg(test)]
fn budget_only(peer_budget: std::time::Duration) -> Pace {
    Pace {
        peer_budget,
        ..NORMAL_PACE
    }
}

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

// The SQL of every tag-filtered pull query ([`query_envelopes`]), so tests can assert which shape ran.
#[cfg(test)]
thread_local! {
    static PULL_SQL: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The pull SQL captured on this thread since the last call.
#[cfg(test)]
fn take_pull_sql() -> Vec<String> {
    PULL_SQL.with(|c| std::mem::take(&mut *c.borrow_mut()))
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
pub(crate) async fn compact_entity_tags(
    conn: &Connection,
    device_id: &str,
    tags: &[String],
) -> u64 {
    compact_within(conn, device_id, tags, COMPACTION_TIMEOUT)
        .await
        .unwrap_or(0)
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
    pub(crate) async fn compact(
        &mut self,
        conn: &Connection,
        device_id: &str,
        tags: &[String],
    ) -> u64 {
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
        let left = self
            .deadline
            .get_or_insert(now + COMPACTION_TIMEOUT)
            .saturating_duration_since(now);
        if left.is_zero() {
            return 0;
        }
        compact_within(conn, device_id, tags, left)
            .await
            .unwrap_or(0)
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

/// The `kind_tag` twin of `is_missing_entity_tag_column`.
pub(crate) fn is_missing_kind_tag_column(error: &str) -> bool {
    error.contains("no column named kind_tag")
}

/// The `slice_tag` twin of `is_missing_kind_tag_column`.
pub(crate) fn is_missing_slice_tag_column(error: &str) -> bool {
    error.contains("no column named slice_tag")
}

/// The `slice_tag` twin of `is_no_such_kind_tag_column` ("no such column: slice_tag").
pub(crate) fn is_no_such_slice_tag_column(error: &str) -> bool {
    error.contains("no such column: slice_tag")
}

/// True when a SELECT failed because the relay's `sync_row` predates `kind_tag`
/// ("no such column: kind_tag").
pub(crate) fn is_no_such_kind_tag_column(error: &str) -> bool {
    error.contains("no such column: kind_tag")
}

const FETCH_ROWS_NO_SKINS_SQL: &str =
    "SELECT local_seq, row_type, ciphertext FROM sync_row WHERE device_id = ?1 AND local_seq > ?2 AND row_type NOT IN ('skin', 'skin_image_chunk') ORDER BY local_seq ASC LIMIT ?3";
const FETCH_ROWS_SQL: &str =
    "SELECT local_seq, row_type, ciphertext FROM sync_row WHERE device_id = ?1 AND local_seq > ?2 ORDER BY local_seq ASC LIMIT ?3";

/// A gap page (#572): the peer's rows strictly between `?3` and `?2`, newest first. The primary
/// key walked backward, like the forward page walks it forward.
const FETCH_GAP_ROWS_NO_SKINS_SQL: &str =
    "SELECT local_seq, row_type, ciphertext FROM sync_row WHERE device_id = ?1 AND local_seq < ?2 AND local_seq > ?3 AND row_type NOT IN ('skin', 'skin_image_chunk') ORDER BY local_seq DESC LIMIT ?4";
const FETCH_GAP_ROWS_SQL: &str =
    "SELECT local_seq, row_type, ciphertext FROM sync_row WHERE device_id = ?1 AND local_seq < ?2 AND local_seq > ?3 ORDER BY local_seq DESC LIMIT ?4";
/// The peer's highest local_seq on the relay: one primary-key seek.
const PEER_TOP_SEQ_SQL: &str =
    "SELECT local_seq FROM sync_row WHERE device_id = ?1 ORDER BY local_seq DESC LIMIT 1";

/// A gap page with less budget than this left fetches one row, not a full page.
const GAP_FULL_PAGE_MIN_BUDGET: std::time::Duration = std::time::Duration::from_secs(1);

/// How many rows a gap page fetches with `budget` left: the pace's page scaled to the share of
/// a peer budget that is left, so a nearly spent budget doesn't download a full page it then
/// applies a row of. One row under [`GAP_FULL_PAGE_MIN_BUDGET`], and never more than a page.
fn gap_page_rows(pace: Pace, budget: std::time::Duration) -> i64 {
    if budget < GAP_FULL_PAGE_MIN_BUDGET || pace.peer_budget.is_zero() {
        return 1;
    }
    let share = (budget.as_secs_f64() / pace.peer_budget.as_secs_f64()).min(1.0);
    ((pace.rows as f64 * share).ceil() as i64).clamp(1, pace.rows.max(1))
}

/// Up to `limit` of the peer's rows with `gap_lo < local_seq < gap_hi`, highest first. With the runner's
/// kind tags ([`RowFilter::query_kind_tags`]) the range is `window_lo < local_seq < gap_hi` and other kinds'
/// tagged `items` rows are left out, and with its slice tags ([`RowFilter::query_slice_tags`]) also sliced rows
/// of a calendar source or month it does not ask for; on a relay without `slice_tag` it falls back to the kind
/// filter, and without `kind_tag` to the plain allowlist over the whole gap. Returns the rows and whether the
/// kind filter (and its window) applied.
#[allow(clippy::too_many_arguments)]
async fn fetch_gap_rows(
    conn: &Connection,
    peer_device_id: &str,
    gap_lo: i64,
    window_lo: i64,
    gap_hi: i64,
    limit: i64,
    skip_skins: bool,
    filter: &RowFilter,
) -> Result<(Vec<RowEnvelope>, bool), SyncEngineError> {
    // At most three tries: with slice and kind tags, with kind tags, then (below) the plain allowlist.
    while let (Some(types), Some(tags)) = (filter.row_types.as_deref(), filter.query_kind_tags()) {
        let slices = filter.query_slice_tags().unwrap_or_default();
        let mut values = vec![
            Value::Text(peer_device_id.to_string()),
            Value::Integer(gap_hi),
            Value::Integer(window_lo),
            Value::Integer(limit),
        ];
        values.extend(types.iter().map(|t| Value::Text(t.clone())));
        values.extend(tags.iter().chain(slices).map(|t| Value::Text(t.clone())));
        match query_envelopes(
            conn,
            &fetch_allowed_gap_rows_sql(types.len(), tags.len(), slices.len()),
            values,
        )
        .await
        {
            Ok(envelopes) => return Ok((envelopes, true)),
            Err(e) if filter.tag_column_missing(&e) => {}
            Err(e) => return Err(e),
        }
    }
    let rows = match filter.row_types.as_deref() {
        Some(types) => {
            let mut values = vec![
                Value::Text(peer_device_id.to_string()),
                Value::Integer(gap_hi),
                Value::Integer(gap_lo),
                Value::Integer(limit),
            ];
            values.extend(types.iter().map(|t| Value::Text(t.clone())));
            conn.query(
                &fetch_allowed_gap_rows_sql(types.len(), 0, 0),
                Params::Positional(values),
            )
            .await
        }
        None => {
            let sql = if skip_skins {
                FETCH_GAP_ROWS_NO_SKINS_SQL
            } else {
                FETCH_GAP_ROWS_SQL
            };
            conn.query(sql, params![peer_device_id, gap_hi, gap_lo, limit])
                .await
        }
    }
    .map_err(|e| SyncEngineError::Query(e.to_string()))?;
    Ok((read_envelopes(rows).await?, false))
}

/// One kind-filtered page: the query and its rows, so a missing `kind_tag` column is caught wherever
/// the relay reports it.
async fn query_envelopes(
    conn: &Connection,
    sql: &str,
    values: Vec<Value>,
) -> Result<Vec<RowEnvelope>, SyncEngineError> {
    #[cfg(test)]
    PULL_SQL.with(|c| c.borrow_mut().push(sql.to_string()));
    let rows = conn
        .query(sql, Params::Positional(values))
        .await
        .map_err(query_err)?;
    read_envelopes(rows).await
}

/// The peer's highest local_seq on the relay, `None` when it has no rows.
async fn peer_top_seq(
    conn: &Connection,
    peer_device_id: &str,
) -> Result<Option<i64>, SyncEngineError> {
    let mut rows = conn
        .query(PEER_TOP_SEQ_SQL, params![peer_device_id])
        .await
        .map_err(|e| SyncEngineError::Query(e.to_string()))?;
    match rows
        .next()
        .await
        .map_err(|e| SyncEngineError::Query(e.to_string()))?
    {
        Some(row) => Ok(Some(
            row.get(0)
                .map_err(|e| SyncEngineError::Query(e.to_string()))?,
        )),
        None => Ok(None),
    }
}

/// `skip_skins` leaves out skin rows at the query, so a puller that never applies them (the phone)
/// never downloads their images. An allowlist (`filter.row_types`) takes only those types instead. With
/// the runner's kind tags the range is `since < local_seq <= window_hi` and other kinds' tagged `items`
/// rows are left out, as are sliced rows of a slice it does not ask for; on a relay without `slice_tag` it
/// falls back to the kind filter, and without `kind_tag` to the plain allowlist, unbounded.
/// Returns the rows and whether the kind filter (and its window) applied.
async fn fetch_rows_since(
    conn: &Connection,
    peer_device_id: &str,
    since_local_seq: i64,
    window_hi: i64,
    limit: i64,
    skip_skins: bool,
    filter: &RowFilter,
) -> Result<(Vec<RowEnvelope>, bool), SyncEngineError> {
    // At most three tries: with slice and kind tags, with kind tags, then (below) the plain allowlist.
    while let (Some(types), Some(tags)) = (filter.row_types.as_deref(), filter.query_kind_tags()) {
        let slices = filter.query_slice_tags().unwrap_or_default();
        let mut values = vec![
            Value::Text(peer_device_id.to_string()),
            Value::Integer(since_local_seq),
            Value::Integer(limit),
        ];
        values.extend(types.iter().map(|t| Value::Text(t.clone())));
        values.push(Value::Integer(window_hi));
        values.extend(tags.iter().chain(slices).map(|t| Value::Text(t.clone())));
        match query_envelopes(
            conn,
            &fetch_allowed_rows_sql(types.len(), tags.len(), slices.len()),
            values,
        )
        .await
        {
            Ok(envelopes) => return Ok((envelopes, true)),
            Err(e) if filter.tag_column_missing(&e) => {}
            Err(e) => return Err(e),
        }
    }
    let rows = match filter.row_types.as_deref() {
        Some(types) => {
            let mut values = vec![
                Value::Text(peer_device_id.to_string()),
                Value::Integer(since_local_seq),
                Value::Integer(limit),
            ];
            values.extend(types.iter().map(|t| Value::Text(t.clone())));
            conn.query(
                &fetch_allowed_rows_sql(types.len(), 0, 0),
                Params::Positional(values),
            )
            .await
        }
        None => {
            let sql = if skip_skins {
                FETCH_ROWS_NO_SKINS_SQL
            } else {
                FETCH_ROWS_SQL
            };
            conn.query(sql, params![peer_device_id, since_local_seq, limit])
                .await
        }
    }
    .map_err(|e| SyncEngineError::Query(e.to_string()))?;
    Ok((read_envelopes(rows).await?, false))
}

/// `FETCH_ROWS_SQL` for an allowlist of `type_count` row types, `?4..`. With `tag_count` kind tags
/// (the runner), also `local_seq <= ?(4 + type_count)` and only `items` rows that are untagged or carry
/// one of the tags, `?(5 + type_count)..`, and with `slice_count` slice tags after them, `?(5 + type_count +
/// tag_count)..`, a sliced row only when its slice is one of them (see [`kind_tag_condition`]). Without kind
/// tags the SQL is exactly the plain allowlist's (slice tags are only ever asked for beside kind tags).
fn fetch_allowed_rows_sql(type_count: usize, tag_count: usize, slice_count: usize) -> String {
    let kind_filter = match tag_count {
        0 => String::new(),
        n => format!(
            " AND local_seq <= ?{} {}",
            4 + type_count,
            kind_tag_condition(5 + type_count, n, slice_count)
        ),
    };
    format!(
        "SELECT local_seq, row_type, ciphertext FROM sync_row WHERE device_id = ?1 AND local_seq > ?2 \
         AND row_type IN ({}){kind_filter} ORDER BY local_seq ASC LIMIT ?3",
        numbered_placeholders(4, type_count)
    )
}

/// `FETCH_GAP_ROWS_SQL` for an allowlist of `type_count` row types, `?5..`. With `tag_count` kind tags,
/// also the kind condition of [`fetch_allowed_rows_sql`], `?(5 + type_count)..`, and its `slice_count` slice
/// tags after them (the window is `?3`).
fn fetch_allowed_gap_rows_sql(type_count: usize, tag_count: usize, slice_count: usize) -> String {
    let kind_filter = match tag_count {
        0 => String::new(),
        n => format!(" {}", kind_tag_condition(5 + type_count, n, slice_count)),
    };
    format!(
        "SELECT local_seq, row_type, ciphertext FROM sync_row WHERE device_id = ?1 AND local_seq < ?2 \
         AND local_seq > ?3 AND row_type IN ({}){kind_filter} ORDER BY local_seq DESC LIMIT ?4",
        numbered_placeholders(5, type_count)
    )
}

// Makes every indexed pull sub-query fail, so tests can check that any such error falls back to the scan.
#[cfg(test)]
thread_local! {
    static INDEXED_QUERY_FAILS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// One indexed pull sub-query ([`fetch_rows_indexed`], [`fetch_gap_rows_indexed`]).
async fn query_indexed(
    conn: &Connection,
    sql: &str,
    values: Vec<Value>,
) -> Result<Vec<RowEnvelope>, SyncEngineError> {
    #[cfg(test)]
    if INDEXED_QUERY_FAILS.with(|c| c.get()) {
        return Err(SyncEngineError::Query(
            "injected indexed query failure".to_string(),
        ));
    }
    query_envelopes(conn, sql, values).await
}

/// An indexed sub-query failed for a reason other than a missing tag column (a missing or wrongly shaped index,
/// a timeout, a dropped connection): the rest of this round scans. Said once a round.
fn give_up_pull_indexes(filter: &RowFilter, error: &SyncEngineError) {
    if !filter.no_pull_indexes.swap(true, Ordering::Relaxed) {
        eprintln!("[sync] an indexed pull query failed ({error}): this round scans");
    }
}

/// One indexed sub-query of [`fetch_rows_indexed`]: `?1` device, `?2` since, `?3` window top, `?4` limit, and
/// `condition`'s own list from `?5`.
fn indexed_pull_sql(index: &str, condition: &str) -> String {
    format!(
        "SELECT local_seq, row_type, ciphertext FROM sync_row INDEXED BY {index} WHERE device_id = ?1 AND {condition} \
         AND local_seq > ?2 AND local_seq <= ?3 ORDER BY local_seq ASC LIMIT ?4"
    )
}

/// The runner's live page when the relay has both pull indexes ([`RowFilter::use_pull_indexes`]): the rows
/// [`fetch_rows_since`]'s slice-filtered query takes in `since < local_seq <= window_hi`, read by three queries
/// that each seek an index instead of scanning the primary key: allowed non-`items` types (`idx_sync_row_type`),
/// `items` of an asked slice, and `items` without a slice whose kind tag is NULL or asked (`idx_sync_row_slice`).
/// Each takes at most `limit` rows in `local_seq` order. A sub-query that came back full may have more above its
/// last row, so the merged page stops there (rows of the others above it are dropped, read again next page);
/// sorted and cut to `limit`. So a page shorter than `limit` holds every row of the window, as the scan's does,
/// and a full one ends where every sub-query has been read. `None` when the relay turns out to lack a tag column,
/// or when an indexed sub-query fails for any other reason (a missing or wrongly shaped index, a timeout, a dropped
/// connection): the caller falls back to the scan, and the rest of this round scans too. A genuine connection
/// error then surfaces from the scan.
///
/// Cost: Q1 and Q2 read several index ranges (one per type or slice) and sort them, so each page reads every
/// match left in the window: about `matches^2 / limit` over a whole window. Q3 reads one range in order but
/// filters `kind_tag` per row (it is not in the index), so it tracks the peer's unsliced `items` rows. The span
/// scan reads every position once, so the indexed window wins while matches stay under about
/// `sqrt(positions * limit)`: ~220k matches for a 98M-position range at `limit` 500.
async fn fetch_rows_indexed(
    conn: &Connection,
    peer_device_id: &str,
    since_local_seq: i64,
    window_hi: i64,
    limit: i64,
    filter: &RowFilter,
) -> Result<Option<Vec<RowEnvelope>>, SyncEngineError> {
    let (Some(types), Some(kinds), Some(slices)) = (
        filter.row_types.as_deref(),
        filter.query_kind_tags(),
        filter.query_slice_tags(),
    ) else {
        return Ok(None);
    };
    // Q1. `slices` is never empty (`month_bucket` clamps the cutoff's month to `MAX_MONTH_BUCKET`, so each kind has
    // at least one month), which the Q2/Q3 split relies on: a sliced row is read by Q2 alone.
    let others: Vec<&String> = types.iter().filter(|t| *t != ROW_TYPE_ITEMS).collect();
    let mut queries: Vec<(String, Vec<&String>)> = Vec::new();
    if !others.is_empty() {
        let condition = format!(
            "row_type != '{ROW_TYPE_ITEMS}' AND row_type IN ({})",
            numbered_placeholders(5, others.len())
        );
        queries.push((indexed_pull_sql("idx_sync_row_type", &condition), others));
    }
    if types.iter().any(|t| t == ROW_TYPE_ITEMS) {
        if !slices.is_empty() {
            let condition = format!(
                "row_type = '{ROW_TYPE_ITEMS}' AND slice_tag IN ({})",
                numbered_placeholders(5, slices.len())
            );
            queries.push((
                indexed_pull_sql("idx_sync_row_slice", &condition),
                slices.iter().collect(),
            ));
        }
        // `kinds` is never empty (kind tags exist only for a kind restriction); if it were, `IN ()` matches nothing
        // and Q3 takes only NULL-kind rows, as the scan's `kind_tag_condition` would.
        let condition = format!(
            "row_type = '{ROW_TYPE_ITEMS}' AND slice_tag IS NULL AND (kind_tag IS NULL OR kind_tag IN ({}))",
            numbered_placeholders(5, kinds.len())
        );
        queries.push((
            indexed_pull_sql("idx_sync_row_slice", &condition),
            kinds.iter().collect(),
        ));
    }
    let mut merged = Vec::new();
    // The lowest last row of a sub-query that came back full: nothing above it is known to be complete.
    let mut cap = i64::MAX;
    for (sql, list) in queries {
        let mut values = vec![
            Value::Text(peer_device_id.to_string()),
            Value::Integer(since_local_seq),
            Value::Integer(window_hi),
            Value::Integer(limit),
        ];
        values.extend(list.into_iter().map(|t| Value::Text(t.clone())));
        match query_indexed(conn, &sql, values).await {
            Ok(rows) => {
                if rows.len() as i64 >= limit {
                    if let Some(last) = rows.last() {
                        cap = cap.min(last.local_seq);
                    }
                }
                merged.extend(rows);
            }
            Err(e) if filter.tag_column_missing(&e) => return Ok(None),
            Err(e) => {
                give_up_pull_indexes(filter, &e);
                return Ok(None);
            }
        }
    }
    merged.retain(|e| e.local_seq <= cap);
    merged.sort_by(|a, b| (a.local_seq, &a.row_type).cmp(&(b.local_seq, &b.row_type)));
    merged.truncate(limit.max(0) as usize);
    Ok(Some(merged))
}

/// One indexed gap sub-query of [`fetch_gap_rows_indexed`]: `?1` device, `?2` the gap's top (exclusive), `?3` its
/// floor (exclusive), `?4` limit, and `condition`'s own list from `?5`.
fn indexed_gap_sql(index: &str, condition: &str) -> String {
    format!(
        "SELECT local_seq, row_type, ciphertext FROM sync_row INDEXED BY {index} WHERE device_id = ?1 AND {condition} \
         AND local_seq < ?2 AND local_seq > ?3 ORDER BY local_seq DESC LIMIT ?4"
    )
}

/// [`fetch_rows_indexed`] for a gap page, newest first: the rows [`fetch_gap_rows`]'s slice-filtered query takes in
/// `gap_lo < local_seq < gap_hi`, by the same three index-seeking queries, each `ORDER BY local_seq DESC LIMIT
/// limit`. A sub-query that came back full may have more below its last row, so the merged page stops there (rows
/// of the others below it are dropped, read by the next page); sorted descending and cut to `limit`. So a page
/// shorter than `limit` holds every row of the gap, and a full one ends where every sub-query has been read. `None`
/// as for [`fetch_rows_indexed`]: the caller scans, and the rest of this round does too.
async fn fetch_gap_rows_indexed(
    conn: &Connection,
    peer_device_id: &str,
    gap_lo: i64,
    gap_hi: i64,
    limit: i64,
    filter: &RowFilter,
) -> Result<Option<Vec<RowEnvelope>>, SyncEngineError> {
    let (Some(types), Some(kinds), Some(slices)) = (
        filter.row_types.as_deref(),
        filter.query_kind_tags(),
        filter.query_slice_tags(),
    ) else {
        return Ok(None);
    };
    // Q1. `slices` is never empty (`month_bucket` clamps the cutoff's month to `MAX_MONTH_BUCKET`, so each kind has
    // at least one month), which the Q2/Q3 split relies on: a sliced row is read by Q2 alone.
    let others: Vec<&String> = types.iter().filter(|t| *t != ROW_TYPE_ITEMS).collect();
    let mut queries: Vec<(String, Vec<&String>)> = Vec::new();
    if !others.is_empty() {
        let condition = format!(
            "row_type != '{ROW_TYPE_ITEMS}' AND row_type IN ({})",
            numbered_placeholders(5, others.len())
        );
        queries.push((indexed_gap_sql("idx_sync_row_type", &condition), others));
    }
    if types.iter().any(|t| t == ROW_TYPE_ITEMS) {
        if !slices.is_empty() {
            let condition = format!(
                "row_type = '{ROW_TYPE_ITEMS}' AND slice_tag IN ({})",
                numbered_placeholders(5, slices.len())
            );
            queries.push((
                indexed_gap_sql("idx_sync_row_slice", &condition),
                slices.iter().collect(),
            ));
        }
        // `kinds` is never empty, as in [`fetch_rows_indexed`].
        let condition = format!(
            "row_type = '{ROW_TYPE_ITEMS}' AND slice_tag IS NULL AND (kind_tag IS NULL OR kind_tag IN ({}))",
            numbered_placeholders(5, kinds.len())
        );
        queries.push((
            indexed_gap_sql("idx_sync_row_slice", &condition),
            kinds.iter().collect(),
        ));
    }
    let mut merged = Vec::new();
    // The highest last (lowest) row of a sub-query that came back full: nothing below it is known to be complete.
    let mut cap = i64::MIN;
    for (sql, list) in queries {
        let mut values = vec![
            Value::Text(peer_device_id.to_string()),
            Value::Integer(gap_hi),
            Value::Integer(gap_lo),
            Value::Integer(limit),
        ];
        values.extend(list.into_iter().map(|t| Value::Text(t.clone())));
        match query_indexed(conn, &sql, values).await {
            Ok(rows) => {
                if rows.len() as i64 >= limit {
                    if let Some(last) = rows.last() {
                        cap = cap.max(last.local_seq);
                    }
                }
                merged.extend(rows);
            }
            Err(e) if filter.tag_column_missing(&e) => return Ok(None),
            Err(e) => {
                give_up_pull_indexes(filter, &e);
                return Ok(None);
            }
        }
    }
    merged.retain(|e| e.local_seq >= cap);
    merged.sort_by(|a, b| (b.local_seq, &b.row_type).cmp(&(a.local_seq, &a.row_type)));
    merged.truncate(limit.max(0) as usize);
    Ok(Some(merged))
}

/// Every row but an `items` row tagged with a kind not asked for. An untagged row (old client, old row,
/// failed ALTER) is always taken and filtered after decrypting, as before tags. With `slice_count` slice tags
/// (`?(first + count)..`), an `items` row with a `slice_tag` is judged by it alone (taken only when it is one
/// of them), and one without by its `kind_tag` as before.
fn kind_tag_condition(first: usize, count: usize, slice_count: usize) -> String {
    let kinds = numbered_placeholders(first, count);
    match slice_count {
        0 => format!("AND (row_type != '{ROW_TYPE_ITEMS}' OR kind_tag IS NULL OR kind_tag IN ({kinds}))"),
        m => format!(
            "AND (row_type != '{ROW_TYPE_ITEMS}' OR (slice_tag IS NULL AND (kind_tag IS NULL OR kind_tag IN ({kinds}))) \
             OR slice_tag IN ({}))",
            numbered_placeholders(first + count, m)
        ),
    }
}

/// `?first, ?first+1, ...`, `count` of them.
fn numbered_placeholders(first: usize, count: usize) -> String {
    (first..first + count)
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(", ")
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
    let placeholders = (0..keys.len())
        .map(|i| format!("?{}", i + 2))
        .collect::<Vec<_>>()
        .join(", ");
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
    envelopes.retain(|env| {
        keys.iter()
            .any(|(seq, ty)| *seq == env.local_seq && *ty == env.row_type)
    });
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
    // A build from before the tombstone row types skipped them as unknown (tests simulate it).
    if old_build_puller() && TOMBSTONE_ROW_TYPES.contains(&row_type) {
        return Ok(());
    }
    let local_id = match row_type {
        ROW_TYPE_CAPTURE_ITEM => {
            let item: memory::AppliedCaptureItem =
                serde_json::from_slice(plaintext).map_err(|_| SyncEngineError::Crypto)?;
            let (kind, id) =
                memory::client_forget::pulled_capture_identity(peer_device_id, local_seq);
            if client_forgotten(store, kind, &id)? {
                return Ok(());
            }
            store
                .apply_synced_capture_item(peer_device_id, local_seq, &item)
                .map_err(|e| SyncEngineError::Query(e.to_string()))?
        }
        ROW_TYPE_MEETING => {
            let payload: MeetingPayload =
                serde_json::from_slice(plaintext).map_err(|_| SyncEngineError::Crypto)?;
            // A late version of a forgotten meeting is dropped. A legacy row without a source id can't be
            // named, so it applies as before.
            if let Some(source_id) = payload.source_meeting_id {
                if store
                    .is_meeting_forgotten(
                        peer_device_id,
                        payload.origin_epoch.as_deref(),
                        source_id,
                    )
                    .map_err(|e| SyncEngineError::Query(e.to_string()))?
                {
                    return Ok(());
                }
            }
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
            // A phone pulls newest first (#572), so an older snapshot can follow a newer one:
            // it never overwrites it. Its seq is still stamped against the meeting, so a legacy
            // line naming that snapshot by `meeting_local_seq` resolves.
            if let (Some(source_id), Some(existing)) =
                (payload.source_meeting_id, existing_local_id)
            {
                if store
                    .newer_sync_version_applied(
                        peer_device_id,
                        ROW_TYPE_MEETING,
                        source_id,
                        payload.origin_epoch.as_deref(),
                        local_seq,
                    )
                    .map_err(|e| SyncEngineError::Query(e.to_string()))?
                {
                    return store
                        .record_sync_row_map(peer_device_id, local_seq, row_type, existing)
                        .map_err(|e| SyncEngineError::Query(e.to_string()));
                }
            }
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
                    .record_sync_meeting_map_with_epoch(
                        peer_device_id,
                        source_id,
                        local_id,
                        payload.origin_epoch.as_deref(),
                    )
                    .map_err(|e| SyncEngineError::Query(e.to_string()))?;
                store
                    .record_sync_version(
                        peer_device_id,
                        ROW_TYPE_MEETING,
                        source_id,
                        payload.origin_epoch.as_deref(),
                        local_seq,
                    )
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
            // Added items stay on the computer that received them (bridge spec 8.2): a peer's
            // row of the kind (a bug, or a newer build) is never applied here.
            if payload.item.kind == memory::MCP_ITEM_KIND {
                return Ok(());
            }
            if forgotten_item(store, peer_device_id, local_seq, &payload.item)? {
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
            // replaces the row in place; a template this build does not know is ignored. An
            // older version pulled after a newer one (newest first, #572) is dropped.
            let epoch = origin_epoch_of(plaintext);
            if store
                .newer_sync_version_applied(
                    peer_device_id,
                    ROW_TYPE_ROUTINE_RUN,
                    run.source_run_id,
                    epoch.as_deref(),
                    local_seq,
                )
                .map_err(|e| SyncEngineError::Query(e.to_string()))?
            {
                return Ok(());
            }
            match store
                .apply_synced_routine_run(peer_device_id, &run)
                .map_err(|e| SyncEngineError::Query(e.to_string()))?
            {
                Some(id) => {
                    store
                        .record_sync_version(
                            peer_device_id,
                            ROW_TYPE_ROUTINE_RUN,
                            run.source_run_id,
                            epoch.as_deref(),
                            local_seq,
                        )
                        .map_err(|e| SyncEngineError::Query(e.to_string()))?;
                    id
                }
                None => return Ok(()),
            }
        }
        ROW_TYPE_MEETING_PREP => {
            // Skipped, not an error, when this build cannot decode it. It has no local row id, so
            // there is no `sync_row_map` entry to record: return before it.
            let Ok(row) = serde_json::from_slice::<memory::prep::MeetingPrepRow>(plaintext) else {
                return Ok(());
            };
            // The brief of a calendar event a client delete forgot.
            if store
                .is_meeting_prep_forgotten(&row.event_id)
                .map_err(|e| SyncEngineError::Query(e.to_string()))?
            {
                return Ok(());
            }
            store
                .apply_synced_meeting_prep(&row)
                .map_err(|e| SyncEngineError::Query(e.to_string()))?;
            return Ok(());
        }
        ROW_TYPE_SKIN => {
            // Kept in the store, not the skins folder: `core/skins` writes it once its images are whole.
            let payload = match serde_json::from_slice::<memory::skin_sync::SkinSyncPayload>(
                plaintext,
            ) {
                Ok(payload) => payload,
                Err(e) => {
                    tracing::warn!(peer = %peer_tag(peer_device_id), local_seq, bytes = plaintext.len(), error = %e, "pulled skin row unreadable");
                    return Ok(());
                }
            };
            let stored = store
                .apply_synced_skin(peer_device_id, &payload)
                .map_err(|e| SyncEngineError::Query(e.to_string()))?;
            tracing::info!(peer = %peer_tag(peer_device_id), local_seq, skin = %payload.skin_id, images = payload.images.len(), deleted = payload.deleted, stored, "pulled skin row");
            return Ok(());
        }
        ROW_TYPE_SKIN_IMAGE_CHUNK => {
            let chunk = match serde_json::from_slice::<memory::skin_sync::SkinImageChunk>(plaintext)
            {
                Ok(chunk) => chunk,
                Err(e) => {
                    tracing::warn!(peer = %peer_tag(peer_device_id), local_seq, error = %e, "pulled skin image chunk unreadable");
                    return Ok(());
                }
            };
            store
                .apply_synced_skin_chunk(&chunk)
                .map_err(|e| SyncEngineError::Query(e.to_string()))?;
            return Ok(());
        }
        ROW_TYPE_PROFILE_SUGGESTION => {
            let Ok(suggestion) =
                serde_json::from_slice::<memory::profile::AppliedProfileSuggestion>(plaintext)
            else {
                return Ok(());
            };
            store
                .apply_synced_profile_suggestion(peer_device_id, &suggestion)
                .map_err(|e| SyncEngineError::Query(e.to_string()))?;
            return Ok(());
        }
        ROW_TYPE_PROFILE_SUGGESTION_STATUS => {
            // Dismissed or added on another device: resolve the copy here (and any that arrives later).
            let Ok(status) = serde_json::from_slice::<
                memory::profile::AppliedProfileSuggestionStatus,
            >(plaintext) else {
                return Ok(());
            };
            store
                .apply_synced_profile_suggestion_status(&status)
                .map_err(|e| SyncEngineError::Query(e.to_string()))?;
            return Ok(());
        }
        ROW_TYPE_LIVE_ALERT => {
            // Upserts on (origin, source_id); its meeting is resolved (or back-filled later)
            // through `sync_meeting_map`, so there is no `sync_row_map` entry to record.
            let Ok(alert) =
                serde_json::from_slice::<memory::live_alert::AppliedLiveAlert>(plaintext)
            else {
                return Ok(());
            };
            // Named by its meeting's identity even while its `meeting_id` is still NULL here.
            let epoch = origin_epoch_of(plaintext);
            if forgotten_child(
                store,
                peer_device_id,
                epoch.as_deref(),
                Some(alert.source_meeting_id),
            )? {
                return Ok(());
            }
            // Re-pushed when a draft is added: an older copy pulled after it (newest first, #572)
            // would wipe the draft, so it is dropped.
            let newer_applied = store
                .newer_sync_version_applied(
                    peer_device_id,
                    ROW_TYPE_LIVE_ALERT,
                    alert.source_id,
                    epoch.as_deref(),
                    local_seq,
                )
                .map_err(|e| SyncEngineError::Query(e.to_string()))?;
            if newer_applied {
                return Ok(());
            }
            if store
                .apply_synced_live_alert(peer_device_id, &alert)
                .map_err(|e| SyncEngineError::Query(e.to_string()))?
                .is_some()
            {
                store
                    .record_sync_version(
                        peer_device_id,
                        ROW_TYPE_LIVE_ALERT,
                        alert.source_id,
                        epoch.as_deref(),
                        local_seq,
                    )
                    .map_err(|e| SyncEngineError::Query(e.to_string()))?;
            }
            return Ok(());
        }
        ROW_TYPE_PERSON => {
            // Last write wins on `updated_at_unix_ms`; no local row id to map.
            let Ok(person) =
                serde_json::from_slice::<memory::speaker_names::AppliedPerson>(plaintext)
            else {
                return Ok(());
            };
            store
                .apply_synced_person(peer_device_id, &person)
                .map_err(|e| SyncEngineError::Query(e.to_string()))?;
            return Ok(());
        }
        ROW_TYPE_MEETING_SPEAKER => {
            // Its meeting is resolved (or back-filled later) through `sync_meeting_map`.
            let Ok(speaker) =
                serde_json::from_slice::<memory::speaker_names::AppliedMeetingSpeaker>(plaintext)
            else {
                return Ok(());
            };
            // Else it would wait in `meeting_speaker_pending` for a meeting that never comes.
            let origin = speaker
                .origin_device_id
                .as_deref()
                .unwrap_or(peer_device_id);
            if forgotten_child(
                store,
                origin,
                origin_epoch_of(plaintext).as_deref(),
                Some(speaker.source_meeting_id),
            )? {
                return Ok(());
            }
            store
                .apply_synced_meeting_speaker(peer_device_id, &speaker)
                .map_err(|e| SyncEngineError::Query(e.to_string()))?;
            return Ok(());
        }
        ROW_TYPE_PROVIDER_SPEND => {
            // The highest cents per (peer, install, month, provider) wins; no local row id to map.
            let Ok(spend) =
                serde_json::from_slice::<memory::provider_budget::AppliedProviderSpend>(plaintext)
            else {
                return Ok(());
            };
            store
                .apply_synced_provider_spend(peer_device_id, &spend)
                .map_err(|e| SyncEngineError::Query(e.to_string()))?;
            return Ok(());
        }
        ROW_TYPE_TOPIC_SEED => {
            // Last write wins on `updated_at_unix_ms`; no local row id to map.
            let Ok(seed) =
                serde_json::from_slice::<memory::topics::seeds::TopicSeedPayload>(plaintext)
            else {
                return Ok(());
            };
            store
                .apply_synced_topic_seed(&seed)
                .map_err(|e| SyncEngineError::Query(e.to_string()))?;
            return Ok(());
        }
        ROW_TYPE_TOPIC_VERDICT => {
            // Last write wins on `updated_at_unix_ms`; no local row id to map.
            let Ok(verdict) =
                serde_json::from_slice::<memory::topics::judge::TopicVerdictPayload>(plaintext)
            else {
                return Ok(());
            };
            if forgotten_seed(store, verdict.topic_key.as_deref())?
                || forgotten_source(store, &verdict.source_key)?
            {
                return Ok(());
            }
            store
                .apply_synced_topic_verdict(&verdict)
                .map_err(|e| SyncEngineError::Query(e.to_string()))?;
            return Ok(());
        }
        ROW_TYPE_TOPIC_STEP => {
            // Last write wins on `updated_at_unix_ms`; `source_id` is looked up here by source key.
            let Ok(step) =
                serde_json::from_slice::<memory::topics::steps::TopicStepPayload>(plaintext)
            else {
                return Ok(());
            };
            if client_forgotten(
                store,
                memory::ClientItemKind::Step.as_str(),
                &memory::client_forget::topic_key_opaque_id(&step.key),
            )? || forgotten_seed(store, Some(&step.topic_key))?
                || forgotten_source(store, &step.source_key)?
            {
                return Ok(());
            }
            store
                .apply_synced_topic_step(&step)
                .map_err(|e| SyncEngineError::Query(e.to_string()))?;
            return Ok(());
        }
        ROW_TYPE_TOPIC_EXTRACTION => {
            let Ok(extraction) =
                serde_json::from_slice::<memory::topics::steps::TopicExtractionPayload>(plaintext)
            else {
                return Ok(());
            };
            if client_forgotten(
                store,
                memory::ClientItemKind::Extraction.as_str(),
                &memory::client_forget::topic_key_opaque_id(&extraction.source_key),
            )? || forgotten_seed(store, extraction.topic_key.as_deref())?
                || forgotten_source(store, &extraction.source_key)?
            {
                return Ok(());
            }
            store
                .apply_synced_topic_extraction(&extraction)
                .map_err(|e| SyncEngineError::Query(e.to_string()))?;
            return Ok(());
        }
        ROW_TYPE_TOPIC_ROLE => {
            let Ok(role) =
                serde_json::from_slice::<memory::topics::page::TopicRolePayload>(plaintext)
            else {
                return Ok(());
            };
            if forgotten_seed(store, Some(&role.topic_key))? {
                return Ok(());
            }
            store
                .apply_synced_topic_role(&role)
                .map_err(|e| SyncEngineError::Query(e.to_string()))?;
            return Ok(());
        }
        ROW_TYPE_TOPIC_NAMES => {
            let Ok(names) =
                serde_json::from_slice::<memory::topics::steps::TopicNamesPayload>(plaintext)
            else {
                return Ok(());
            };
            store
                .apply_synced_topic_names(&names)
                .map_err(|e| SyncEngineError::Query(e.to_string()))?;
            return Ok(());
        }
        ROW_TYPE_TOPIC_PAGE_SNAPSHOT => {
            // Last write wins on `built_at_unix_ms`; a bad or out-of-range build is dropped.
            let Ok(snap) =
                serde_json::from_slice::<memory::topics::snapshot::TopicPageSnapshot>(plaintext)
            else {
                return Ok(());
            };
            if forgotten_seed(store, Some(&snap.topic_key))? {
                return Ok(());
            }
            store
                .apply_synced_topic_page_snapshot(&snap, now_unix_ms())
                .map_err(|e| SyncEngineError::Query(e.to_string()))?;
            return Ok(());
        }
        ROW_TYPE_MEETING_FORGOTTEN => {
            // Deletes the meeting here and remembers it, so its late rows are dropped. Kept on the relay
            // forever (untagged, never compacted), so a device joining later learns it too. If it names this
            // device's own meeting, `delete_forgotten_meeting_rows` removes the server rows next.
            let Ok(forgotten) = serde_json::from_slice::<memory::MeetingForgotten>(plaintext)
            else {
                return Ok(());
            };
            if let Some(files) = store
                .apply_meeting_forgotten(&forgotten)
                .map_err(|e| SyncEngineError::Query(e.to_string()))?
            {
                push_forgotten_meeting_files(files);
            }
            return Ok(());
        }
        ROW_TYPE_CLIENT_ITEM_FORGOTTEN => {
            // A client delete: removes this device's copy and remembers its id, so a late copy is dropped.
            // Kept on the relay forever (untagged, never compacted), so a device joining later learns it too.
            // No client item owns a file on a peer (a meeting's files go through `meeting_forgotten`).
            let Ok(forgotten) = serde_json::from_slice::<memory::ClientItemForgotten>(plaintext)
            else {
                return Ok(());
            };
            store
                .apply_client_item_forgotten(&forgotten)
                .map_err(|e| SyncEngineError::Query(e.to_string()))?;
            return Ok(());
        }
        ROW_TYPE_TOPIC_SEED_PURGED => {
            // A deleted client's seed: hard-deleted here with its topic content, whatever version is here.
            let Ok(seed) =
                serde_json::from_slice::<memory::topics::seeds::TopicSeedPayload>(plaintext)
            else {
                return Ok(());
            };
            if seed.purged {
                store
                    .apply_synced_topic_seed(&seed)
                    .map_err(|e| SyncEngineError::Query(e.to_string()))?;
            }
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

/// A row of a peer's meeting that a tombstone forgot: dropped, never applied or left waiting. The row's
/// `origin_epoch` (newer senders) must match the tombstone's when both are known, so an earlier database's
/// forget never drops a re-created database's rows for a reused meeting id; a meeting still mapped here wins
/// either way. `None` (a row from before source ids) can't be named.
fn forgotten_child(
    store: &Store,
    origin_device_id: &str,
    origin_epoch: Option<&str>,
    source_meeting_id: Option<i64>,
) -> Result<bool, SyncEngineError> {
    let Some(source_id) = source_meeting_id else {
        return Ok(false);
    };
    // This device's own meetings have no map row, and an epoch-less match could name a meeting of an earlier
    // database; their child rows apply (or wait) against the meeting row itself.
    if store.own_device_id().as_deref() == Some(origin_device_id) {
        return Ok(false);
    }
    let q = |e: &dyn std::fmt::Display| SyncEngineError::Query(e.to_string());
    Ok(store
        .resolve_sync_meeting_map(origin_device_id, source_id)
        .map_err(|e| q(&e))?
        .is_none()
        && store
            .is_meeting_forgotten(origin_device_id, origin_epoch, source_id)
            .map_err(|e| q(&e))?)
}

/// Whether a client delete forgot `kind`/`id` (see `memory::client_forget`): a late copy is dropped.
fn client_forgotten(store: &Store, kind: &str, id: &str) -> Result<bool, SyncEngineError> {
    store
        .is_client_item_forgotten(kind, id)
        .map_err(|e| SyncEngineError::Query(e.to_string()))
}

/// A pulled `items` row of a client item a delete forgot (a capture, calendar event or email).
fn forgotten_item(
    store: &Store,
    origin_device_id: &str,
    local_seq: i64,
    item: &memory::AppliedItem,
) -> Result<bool, SyncEngineError> {
    match memory::client_forget::pulled_item_identity(origin_device_id, local_seq, item) {
        Some((kind, id)) => client_forgotten(store, kind, &id),
        None => Ok(false),
    }
}

/// A topic row of a seed topic a client delete purged.
fn forgotten_seed(store: &Store, topic_key: Option<&str>) -> Result<bool, SyncEngineError> {
    match topic_key {
        Some(key) => client_forgotten(store, memory::client_forget::FORGOTTEN_SEED_KIND, key),
        None => Ok(false),
    }
}

/// A topic row about a source a client delete forgot (an email, by its `email:{gmail id}` key).
fn forgotten_source(store: &Store, source_key: &str) -> Result<bool, SyncEngineError> {
    match source_key.strip_prefix("email:") {
        Some(id) => client_forgotten(store, memory::ClientItemKind::Email.as_str(), id),
        None => Ok(false),
    }
}

/// Files of meetings a pulled tombstone deleted here, for the app to remove (sync-engine knows no recordings
/// folder). Drained by [`take_forgotten_meeting_files`]; per thread under test, like the other counters.
#[cfg(not(test))]
static FORGOTTEN_MEETING_FILES: std::sync::Mutex<Vec<memory::DeletedMeetingFiles>> =
    std::sync::Mutex::new(Vec::new());
#[cfg(test)]
thread_local! {
    static FORGOTTEN_MEETING_FILES_CELL: std::cell::RefCell<Vec<memory::DeletedMeetingFiles>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn push_forgotten_meeting_files(files: memory::DeletedMeetingFiles) {
    #[cfg(not(test))]
    FORGOTTEN_MEETING_FILES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(files);
    #[cfg(test)]
    FORGOTTEN_MEETING_FILES_CELL.with(|c| c.borrow_mut().push(files));
}

/// The files of every meeting a pulled tombstone deleted since the last call. The app deletes them the way
/// it deletes a forgotten meeting's own (`api::retention::delete_all_meeting_files`). Not persisted: a crash
/// before the app takes them leaves them on disk.
pub fn take_forgotten_meeting_files() -> Vec<memory::DeletedMeetingFiles> {
    #[cfg(not(test))]
    return std::mem::take(
        &mut *FORGOTTEN_MEETING_FILES
            .lock()
            .unwrap_or_else(|e| e.into_inner()),
    );
    #[cfg(test)]
    return FORGOTTEN_MEETING_FILES_CELL.with(|c| c.take());
}

/// Most `pushed_meeting_row` records one round's cleanup handles; the rest wait for the next round.
const FORGOTTEN_ROWS_PER_ROUND: i64 = 1000;

/// The most one round spends deleting forgotten meetings' rows, so it can never eat `SYNC_TIMEOUT`.
const FORGOTTEN_CLEANUP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Deletes these exact `local_seq`s of one `row_type` of `?1`'s rows, created at or after `?3`: a primary-key
/// seek per key. The time bound keeps an earlier database's row at a reused `local_seq` (the relay INSERT was a
/// no-op for the new one) out of the delete.
fn forgotten_rows_delete_sql(count: usize) -> String {
    let placeholders: Vec<String> = (4..count + 4).map(|i| format!("?{i}")).collect();
    format!(
        "DELETE FROM sync_row WHERE device_id = ?1 AND row_type = ?2 AND created_at_unix_ms >= ?3 AND local_seq IN ({})",
        placeholders.join(", ")
    )
}

/// A forgotten meeting's `meeting` rows by their entity tag (epoch-scoped, so never an earlier database's):
/// a seek on the ops index. Catches versions pushed before `pushed_meeting_row` existed.
const FORGOTTEN_MEETING_TAG_DELETE_SQL: &str =
    "DELETE FROM sync_row WHERE device_id = ?1 AND entity_tag = ?2";

/// The recorder's cleanup: deletes this device's own server rows of meetings it deleted for a tombstone (its
/// own forget, or a peer's naming its meeting): by primary key from `pushed_meeting_row`, then the `meeting`
/// row by entity tag once the ops index exists. Only `device_id = own` rows, so another device's rows are never
/// touched. Best effort: a failure or timeout is logged, what is left stays recorded, and the next round retries.
pub(crate) async fn delete_forgotten_meeting_rows(
    conn: &Connection,
    group_key: &GroupKey,
    store: &Store,
    own_device_id: &str,
) {
    let delete_all = async {
        let rows = store
            .forgotten_pushed_meeting_rows(FORGOTTEN_ROWS_PER_ROUND)
            .map_err(|e| e.to_string())?;
        let mut groups: std::collections::BTreeMap<(i64, &str, i64), Vec<i64>> = Default::default();
        for (meeting_id, local_seq, row_type, not_before) in &rows {
            groups
                .entry((*meeting_id, row_type.as_str(), *not_before))
                .or_default()
                .push(*local_seq);
        }
        for ((meeting_id, row_type, not_before), seqs) in groups {
            for chunk in seqs.chunks(PENDING_LOOKUP_CHUNK) {
                let mut values = vec![
                    Value::Text(own_device_id.to_string()),
                    Value::Text(row_type.to_string()),
                    Value::Integer(not_before),
                ];
                values.extend(chunk.iter().map(|seq| Value::Integer(*seq)));
                conn.execute(
                    &forgotten_rows_delete_sql(chunk.len()),
                    Params::Positional(values),
                )
                .await
                .map_err(|e| e.to_string())?;
                let done: Vec<(i64, i64, String)> = chunk
                    .iter()
                    .map(|seq| (meeting_id, *seq, row_type.to_string()))
                    .collect();
                store
                    .clear_pushed_meeting_rows(&done)
                    .map_err(|e| e.to_string())?;
            }
        }
        let pending_tags = store
            .meetings_awaiting_tag_delete()
            .map_err(|e| e.to_string())?;
        if pending_tags.is_empty()
            || !ENTITY_TAG_COLUMN_SEEN.get()
            || !compaction_index_ready(conn).await
        {
            return Ok(());
        }
        let epoch = store.sync_entity_epoch().map_err(|e| e.to_string())?;
        for meeting_id in pending_tags {
            let tag = tag_for(
                group_key,
                own_device_id,
                &epoch,
                &format!("{ROW_TYPE_MEETING}:{meeting_id}"),
            );
            conn.execute(
                FORGOTTEN_MEETING_TAG_DELETE_SQL,
                params![own_device_id, tag],
            )
            .await
            .map_err(|e| e.to_string())?;
            store
                .mark_meeting_tag_deleted(meeting_id)
                .map_err(|e| e.to_string())?;
        }
        Ok::<(), String>(())
    };
    match tokio::time::timeout(FORGOTTEN_CLEANUP_TIMEOUT, delete_all).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => eprintln!("[sync] forgotten meeting rows kept for the next round: {e}"),
        Err(_) => eprintln!("[sync] forgotten meeting rows kept for the next round: no reply within {FORGOTTEN_CLEANUP_TIMEOUT:?}"),
    }
}

/// The row types that delete something everywhere. A build that did not know one skipped its rows and moved
/// its cursor past them for good, so a build that knows them reads them all again once ([`catch_up_tombstones`]).
const TOMBSTONE_ROW_TYPES: [&str; 3] = [
    ROW_TYPE_MEETING_FORGOTTEN,
    ROW_TYPE_CLIENT_ITEM_FORGOTTEN,
    ROW_TYPE_TOPIC_SEED_PURGED,
];

/// The set of [`TOMBSTONE_ROW_TYPES`] this build knows; bump it when one is added, so every device reads them
/// all again once.
const TOMBSTONE_TYPES_VERSION: &str = "1";

/// kv: the [`TOMBSTONE_TYPES_VERSION`] this database has caught up to, with every peer done.
const TOMBSTONE_CATCH_UP_KEY: &str = "sync_tombstone_types_caught_up";

/// kv per peer: how far the catch-up has read that peer's rows (`local_seq`), or `done`.
fn catch_up_peer_key(peer: &str) -> String {
    format!("sync_tombstone_catch_up:{TOMBSTONE_TYPES_VERSION}:{peer}")
}

/// One page of a peer's tombstone rows in `(?2, ?3]` of `local_seq`: a seek on the primary key's
/// `(device_id, local_seq)` prefix, filtered to the three types. The window bounds what one query reads, so a peer
/// with millions of other rows between two tombstones still moves forward query by query. Over pages and windows
/// it reads every row of the peer, once per database per tombstone-types version.
const CATCH_UP_SQL: &str = "SELECT local_seq, row_type, ciphertext FROM sync_row \
     WHERE device_id = ?1 AND local_seq > ?2 AND local_seq <= ?3 AND row_type IN (?4, ?5, ?6) ORDER BY local_seq LIMIT ?7";

/// A peer's highest `local_seq`: where its catch-up is done.
const CATCH_UP_END_SQL: &str = "SELECT MAX(local_seq) FROM sync_row WHERE device_id = ?1";

const CATCH_UP_PAGE: i64 = 500;

/// How many `local_seq` values one catch-up query spans.
const CATCH_UP_SPAN: i64 = 20_000;

/// How many `local_seq` values one kind-filtered pull query spans (the runner). Without a window, a peer with
/// few rows of the kinds asked for makes one query scan its whole remaining range before it fills LIMIT
/// (no index covers `kind_tag`). A window bounds the rows one query reads; a short window moves the cursor
/// (or the gap's top) to its end, and the pass goes on to the next window while its budget lasts.
/// 50k comes from a local SQLite measurement, not Turso: tune it from the relay's rows-read and per-query latency.
const FILTERED_PULL_SPAN: i64 = 50_000;

/// The most one round spends catching up, well inside `SYNC_TIMEOUT`; the rest resumes next round. Checked after
/// every row, and progress is saved row by row, so a round that runs out mid-page keeps what it did (X6 final
/// review, Major 12).
const CATCH_UP_BUDGET: std::time::Duration = std::time::Duration::from_secs(5);

/// A hard stop past the budget for a relay read that never returns (one bounded query or row): what was saved
/// before it stays. Kept small: the catch-up runs before the round's live ranges, inside `SYNC_TIMEOUT`.
const CATCH_UP_BACKSTOP_SLACK: std::time::Duration = std::time::Duration::from_secs(3);

#[cfg(test)]
thread_local! {
    /// Tests: this thread pulls as a build from before the tombstone row types (it skips them, no catch-up).
    pub(crate) static OLD_BUILD_PULLER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Tests: the catch-up fails for this peer.
    pub(crate) static CATCH_UP_FAILS_FOR: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
    /// Tests: the catch-up budget, and a delay per row (a slow relay).
    pub(crate) static CATCH_UP_TEST_PACE: std::cell::Cell<Option<(std::time::Duration, std::time::Duration)>> =
        const { std::cell::Cell::new(None) };
    /// Tests: the span of one filtered pull query.
    pub(crate) static FILTERED_PULL_TEST_SPAN: std::cell::Cell<Option<i64>> = const { std::cell::Cell::new(None) };
    /// Tests: the span of one catch-up query.
    pub(crate) static CATCH_UP_TEST_SPAN: std::cell::Cell<Option<i64>> = const { std::cell::Cell::new(None) };
}

/// Whether this puller predates the tombstone row types (only ever true in tests).
fn old_build_puller() -> bool {
    #[cfg(test)]
    return OLD_BUILD_PULLER.with(|c| c.get());
    #[cfg(not(test))]
    false
}

/// The catch-up budget and the delay per row (none outside tests).
fn catch_up_pace() -> (std::time::Duration, Option<std::time::Duration>) {
    #[cfg(test)]
    if let Some((budget, delay)) = CATCH_UP_TEST_PACE.with(|c| c.get()) {
        return (budget, Some(delay));
    }
    (CATCH_UP_BUDGET, None)
}

fn filtered_pull_span() -> i64 {
    #[cfg(test)]
    if let Some(span) = FILTERED_PULL_TEST_SPAN.with(|c| c.get()) {
        return span.max(1);
    }
    FILTERED_PULL_SPAN
}

fn catch_up_span() -> i64 {
    #[cfg(test)]
    if let Some(span) = CATCH_UP_TEST_SPAN.with(|c| c.get()) {
        return span;
    }
    CATCH_UP_SPAN
}

/// Reads every tombstone row from every peer once per database and [`TOMBSTONE_TYPES_VERSION`], so a device
/// that pulled them on an older build (which skipped them and moved its cursor past) applies them now. Leaves
/// cursors alone; progress is saved per peer after every row, so a long catch-up spreads over rounds within
/// [`CATCH_UP_BUDGET`] and always moves forward. Never fails the pull: a failure is logged and that peer resumes
/// next round. The database is marked caught up only once every peer is done.
async fn catch_up_tombstones(
    conn: &Connection,
    group_key: &GroupKey,
    store: &Store,
    own_device_id: &str,
) {
    if old_build_puller()
        || store
            .get_meta(TOMBSTONE_CATCH_UP_KEY)
            .ok()
            .flatten()
            .as_deref()
            == Some(TOMBSTONE_TYPES_VERSION)
    {
        return;
    }
    let (budget, delay) = catch_up_pace();
    let deadline = std::time::Instant::now() + budget;
    let all = async {
        let mut all_done = true;
        for peer in distinct_peer_device_ids(conn, own_device_id).await? {
            match catch_up_peer(conn, group_key, store, &peer, deadline, delay).await {
                Ok(true) => {}
                // Out of time: the rest resumes next round, from what was saved.
                Ok(false) => {
                    all_done = false;
                    break;
                }
                Err(e) => {
                    eprintln!("[sync] tombstone catch-up for a peer resumes next round: {e}");
                    all_done = false;
                }
            }
        }
        Ok::<bool, SyncEngineError>(all_done)
    };
    match tokio::time::timeout(budget + CATCH_UP_BACKSTOP_SLACK, all).await {
        Ok(Ok(true)) => {
            if let Err(e) = store.set_meta(TOMBSTONE_CATCH_UP_KEY, TOMBSTONE_TYPES_VERSION) {
                eprintln!("[sync] tombstone catch-up not recorded: {e}");
            }
        }
        Ok(Ok(false)) => eprintln!("[sync] tombstone catch-up resumes next round"),
        Ok(Err(e)) => eprintln!("[sync] tombstone catch-up resumes next round: {e}"),
        Err(_) => {
            eprintln!("[sync] tombstone catch-up resumes next round: a relay read took too long")
        }
    }
}

/// One peer's catch-up, window by window from its saved progress, which is saved after every row handled (or
/// skipped) and at the end of every window. `Ok(true)`: done; `Ok(false)`: `deadline` passed first (at least one
/// row or window is always handled, so every round moves forward).
async fn catch_up_peer(
    conn: &Connection,
    group_key: &GroupKey,
    store: &Store,
    peer: &str,
    deadline: std::time::Instant,
    delay: Option<std::time::Duration>,
) -> Result<bool, SyncEngineError> {
    let q = |e: &dyn std::fmt::Display| SyncEngineError::Query(e.to_string());
    #[cfg(test)]
    if CATCH_UP_FAILS_FOR.with(|c| c.borrow().as_deref() == Some(peer)) {
        return Err(SyncEngineError::Query("injected".into()));
    }
    let key = catch_up_peer_key(peer);
    let mut after = match store.get_meta(&key).map_err(|e| q(&e))?.as_deref() {
        Some("done") => return Ok(true),
        Some(seq) => seq.parse::<i64>().unwrap_or(0),
        None => 0,
    };
    let end: i64 = {
        let mut rows = conn
            .query(CATCH_UP_END_SQL, params![peer])
            .await
            .map_err(|e| q(&e))?;
        match rows.next().await.map_err(|e| q(&e))? {
            Some(row) => row.get::<Option<i64>>(0).map_err(|e| q(&e))?.unwrap_or(0),
            None => 0,
        }
    };
    let span = catch_up_span().max(1);
    loop {
        if after >= end {
            store.set_meta(&key, "done").map_err(|e| q(&e))?;
            return Ok(true);
        }
        let window_end = after.saturating_add(span).min(end);
        let mut rows = conn
            .query(
                CATCH_UP_SQL,
                params![
                    peer,
                    after,
                    window_end,
                    TOMBSTONE_ROW_TYPES[0],
                    TOMBSTONE_ROW_TYPES[1],
                    TOMBSTONE_ROW_TYPES[2],
                    CATCH_UP_PAGE
                ],
            )
            .await
            .map_err(|e| q(&e))?;
        let mut page = 0;
        while let Some(row) = rows.next().await.map_err(|e| q(&e))? {
            page += 1;
            let local_seq: i64 = row.get(0).map_err(|e| q(&e))?;
            let row_type: String = row.get(1).map_err(|e| q(&e))?;
            let ciphertext: String = row.get(2).map_err(|e| q(&e))?;
            if let Some(d) = delay {
                tokio::time::sleep(d).await;
            }
            let plaintext = STANDARD.decode(&ciphertext).ok().and_then(|c| {
                let aad = content_row_aad(peer, local_seq, &row_type);
                sync::decrypt_payload(group_key, &c, &aad).ok()
            });
            if let Some(plaintext) = plaintext {
                if !replay_would_reach_a_newer_meeting(store, &row_type, &plaintext)? {
                    apply_parent_row(store, peer, local_seq, &row_type, &plaintext)?;
                }
            }
            // Handled (or skipped for good): saved at once, so a round cut short keeps it.
            after = local_seq;
            store
                .set_meta(&key, &after.to_string())
                .map_err(|e| q(&e))?;
            if std::time::Instant::now() >= deadline {
                return Ok(false);
            }
        }
        if page < CATCH_UP_PAGE {
            // The window is read to its end.
            after = window_end;
            store
                .set_meta(&key, &after.to_string())
                .map_err(|e| q(&e))?;
        }
        if after < end && std::time::Instant::now() >= deadline {
            return Ok(false);
        }
    }
}

/// A replayed `meeting_forgotten` without an origin epoch (from a build before epochs) matches any meeting
/// with that origin and id, and a long-delayed replay could reach a meeting a re-created database later
/// reused the id for. Such a tombstone applies on replay only when it names the start of the meeting mapped
/// here; otherwise it is skipped (the meeting it meant was never here, or is a different one).
fn replay_would_reach_a_newer_meeting(
    store: &Store,
    row_type: &str,
    plaintext: &[u8],
) -> Result<bool, SyncEngineError> {
    if row_type != ROW_TYPE_MEETING_FORGOTTEN {
        return Ok(false);
    }
    let Ok(t) = serde_json::from_slice::<memory::MeetingForgotten>(plaintext) else {
        return Ok(false);
    };
    if t.origin_epoch.is_some()
        || store.own_device_id().as_deref() == Some(t.origin_device_id.as_str())
    {
        return Ok(false);
    }
    let q = |e: &dyn std::fmt::Display| SyncEngineError::Query(e.to_string());
    let Some(local) = store
        .resolve_sync_meeting_map(&t.origin_device_id, t.source_meeting_id)
        .map_err(|e| q(&e))?
    else {
        return Ok(false);
    };
    let start = store
        .get_meeting(local)
        .map_err(|e| q(&e))?
        .map(|m| m.start_unix_ms);
    Ok(t.source_start_unix_ms.is_none() || start != t.source_start_unix_ms)
}

/// Rewrites the raw calendar identities a client delete left in `client_relay_cleanup` into this device's
/// entity tags (the relay's own form, keyed by the group key), so no raw calendar id outlives one round. Runs
/// at the start of every round, whether or not the tagged rows can be deleted yet. Best effort.
pub fn seal_client_cleanup(store: &Store, group_key: &GroupKey, own_device_id: &str) {
    let sealed = store.sync_entity_epoch().and_then(|epoch| {
        store.seal_client_relay_cleanup(|identity| {
            tag_for(group_key, own_device_id, &epoch, identity)
        })
    });
    if let Err(e) = sealed {
        eprintln!("[sync] client cleanup targets not sealed this round: {e}");
    }
}

/// This device's relay row of a forgotten item by primary key, created within the forget's bounds (an
/// earlier database's row at a reused `local_seq` is older; nothing later is one of the forgotten rows).
const CLIENT_SEQ_DELETE_SQL: &str = "DELETE FROM sync_row WHERE device_id = ?1 AND local_seq = ?2 \
     AND row_type = ?3 AND created_at_unix_ms BETWEEN ?4 AND ?5";

/// This device's tagged relay rows of a forgotten calendar event created up to the forget: a seek on the ops
/// index. A row re-filed under the same identity after the forget survives.
const CLIENT_TAG_DELETE_SQL: &str =
    "DELETE FROM sync_row WHERE device_id = ?1 AND entity_tag = ?2 AND created_at_unix_ms <= ?3";

/// Deletes this device's own relay rows of items a client delete forgot (its own delete, or a peer's tombstone
/// naming its rows): rows it sent through the outbox and its own captures by primary key, its own calendar
/// events by sealed entity tag once the ops index exists. Only `device_id = own` rows, each bounded by the
/// forget time. Best effort and time-boxed like `delete_forgotten_meeting_rows`: what is left stays recorded
/// for the next round.
pub(crate) async fn delete_forgotten_client_rows(
    conn: &Connection,
    store: &Store,
    own_device_id: &str,
) {
    let delete_all = async {
        let targets = store
            .client_relay_cleanup(FORGOTTEN_ROWS_PER_ROUND)
            .map_err(|e| e.to_string())?;
        let tags_ready = ENTITY_TAG_COLUMN_SEEN.get() && compaction_index_ready(conn).await;
        for (target, not_before, not_after) in targets {
            if let Some(rest) = target.strip_prefix("seq:") {
                let (row_type, seq) = rest.rsplit_once(':').ok_or("unreadable cleanup target")?;
                let seq: i64 = seq.parse().map_err(|_| "unreadable cleanup target")?;
                conn.execute(
                    CLIENT_SEQ_DELETE_SQL,
                    params![own_device_id, seq, row_type, not_before, not_after],
                )
                .await
                .map_err(|e| e.to_string())?;
            } else if let Some(tag) = target.strip_prefix("tag:") {
                if !tags_ready {
                    continue;
                }
                conn.execute(
                    CLIENT_TAG_DELETE_SQL,
                    params![own_device_id, tag, not_after],
                )
                .await
                .map_err(|e| e.to_string())?;
            } else {
                // `raw:`: sealed at the start of the next round.
                continue;
            }
            store
                .clear_client_relay_cleanup(std::slice::from_ref(&target))
                .map_err(|e| e.to_string())?;
        }
        Ok::<(), String>(())
    };
    match tokio::time::timeout(FORGOTTEN_CLEANUP_TIMEOUT, delete_all).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => eprintln!("[sync] forgotten client rows kept for the next round: {e}"),
        Err(_) => eprintln!("[sync] forgotten client rows kept for the next round: no reply within {FORGOTTEN_CLEANUP_TIMEOUT:?}"),
    }
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
    pull_content_with_options(
        conn,
        group_key,
        store,
        own_device_id,
        &PullOptions::default(),
    )
    .await
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
    /// Pull only these row types (the cloud runner's grant); `None` pulls every type. An entry is a row
    /// type (`meeting`, `routine_run`, `items`, ...) or one kind of `items` row (`items:capture`,
    /// `items:calendar_event`, `items:transcript`), which also takes that kind's legacy row type
    /// (`capture_item`, `transcript_segment`). A transcript grant without `meeting` applies only the
    /// meetings its applied lines belong to (fetched by key), never the rest: meeting rows are still read
    /// (a line names its meeting only inside the ciphertext), but only their key is kept. Tombstones
    /// (`meeting_forgotten`, `client_item_forgotten`, `topic_seed_purged`) and marker rows are always
    /// pulled. The row type is filtered at the query, and so is the item kind of a row tagged with one
    /// (`kind_tag`), and the calendar source class and month of a row with a `slice_tag` (against
    /// `non_google_calendar_only` and `item_window_ms`); an untagged row's kind is checked after decrypting, as
    /// is every row's; a row left out
    /// still moves the cursor past it, so this is for a store that is never pulled without it (the
    /// runner's per-run store).
    pub row_type_allowlist: Option<Vec<String>>,
    /// Skip `items` rows of every kind, and legacy `capture_item` rows, whose own `ts_unix_ms` is older
    /// than now minus this many ms (dated by the item, so a future calendar event is always kept).
    /// A meeting is dated by its start: one started before the window is applied only as the parent of an
    /// applied transcript line. A legacy `transcript_segment` line has no date of its own: it is dated by its
    /// meeting's start plus its offset into the meeting. Tombstones are never windowed. `None` applies everything.
    pub item_window_ms: Option<i64>,
    /// Apply a `calendar_event` item only when its stored source is known and is not Google Calendar (the
    /// cloud runner: Google Calendar text stays off it until Google verification). An event with no
    /// source is treated as Google's. A row left out still moves the cursor past it. `false` (the
    /// default) applies every calendar event.
    pub non_google_calendar_only: bool,
}

/// The row types an allowlist always takes: what deletes something, and the markers.
const ALWAYS_PULLED_ROW_TYPES: [&str; 9] = [
    ROW_TYPE_MEETING_FORGOTTEN,
    ROW_TYPE_CLIENT_ITEM_FORGOTTEN,
    ROW_TYPE_TOPIC_SEED_PURGED,
    ROW_TYPE_DEVICE_RETIRED,
    ROW_TYPE_ITEMS_CAPABILITY,
    ROW_TYPE_TOPICS_CAPABILITY,
    ROW_TYPE_TOPIC_PAGE_CAPABILITY,
    ROW_TYPE_TOPIC_SNAPSHOT_CAPABILITY,
    ROW_TYPE_CLIENT_CONTROLS_CAPABILITY,
];

/// The most row types, kind tags and slice tags one pull query carries, well under SQLite's 999 variables
/// (the query adds five of its own).
const MAX_TAG_QUERY_VARIABLES: usize = 900;

/// Set once this process has said a pull's slice tags were too many, so it says it once, not every round.
static SLICE_TAGS_TOO_MANY_LOGGED: AtomicBool = AtomicBool::new(false);

/// What one pull round takes, from [`PullOptions`]: the row types asked for at the query (`None`: all),
/// the `items` kinds applied (`None`: all), and the two time cutoffs.
#[derive(Debug, Default)]
struct RowFilter {
    row_types: Option<Vec<String>>,
    item_kinds: Option<Vec<String>>,
    /// The `kind_tag`s of `item_kinds` when it restricts kinds (the runner), so the relay query leaves
    /// out other kinds' tagged rows; `None` when every kind is wanted.
    kind_tags: Option<Vec<String>>,
    /// This round found the relay's `sync_row` has no `kind_tag`: the rest of it queries without tags.
    no_kind_tag_column: AtomicBool,
    /// The `slice_tag`s this pull asks for beside `kind_tags` (see [`RowFilter::slices`]); `None` without kind
    /// tags, or when they would make the query too long.
    slice_tags: Option<Vec<String>>,
    /// This round found the relay's `sync_row` has no `slice_tag`: the rest of it queries with kind tags only.
    no_slice_tag_column: AtomicBool,
    capture_cutoff: Option<i64>,
    item_cutoff: Option<i64>,
    /// `meeting` is pulled only as the parent of a granted transcript (it was not granted itself): a meeting
    /// row is not applied, only remembered by key ([`remember_parent_meeting`]), and fetched by that key
    /// when an applied transcript line needs it.
    meetings_as_parents_only: bool,
    non_google_calendar_only: bool,
    /// Whether the relay has the runner's pull indexes, read once per round (a filter lives for one round;
    /// [`RowFilter::use_pull_indexes`]).
    pull_indexes: tokio::sync::OnceCell<bool>,
    /// An indexed sub-query failed this round ([`give_up_pull_indexes`]): the rest of the round scans.
    no_pull_indexes: AtomicBool,
}

/// Whether `sql` (a query of index names) finds both `idx_sync_row_slice` and `idx_sync_row_type`; any error is false.
async fn pull_indexes_exist(conn: &Connection, sql: &str) -> bool {
    let Ok(mut rows) = conn.query(sql, ()).await else {
        return false;
    };
    let (mut slice, mut kind) = (false, false);
    loop {
        match rows.next().await {
            Ok(Some(row)) => match row.get::<String>(0).as_deref() {
                Ok("idx_sync_row_slice") => slice = true,
                Ok("idx_sync_row_type") => kind = true,
                _ => {}
            },
            Ok(None) => break,
            Err(_) => return false,
        }
    }
    slice && kind
}

/// True only when the relay has both pull indexes, `idx_sync_row_slice` and `idx_sync_row_type`. The name is
/// the contract (the partial predicate is set where the index is created); any error means false.
async fn relay_has_pull_indexes(conn: &Connection) -> bool {
    pull_indexes_exist(
        conn,
        "SELECT name FROM sqlite_master WHERE type='index' AND name IN ('idx_sync_row_slice','idx_sync_row_type')",
    )
    .await
}

impl RowFilter {
    /// Whether this round may use the indexed queries: only with slice tags in the query, and the relay has both
    /// pull indexes. Checked once per round; false for the rest of the round once an indexed sub-query failed.
    async fn use_pull_indexes(&self, conn: &Connection) -> bool {
        if self.query_slice_tags().is_none() || self.no_pull_indexes.load(Ordering::Relaxed) {
            return false;
        }
        *self
            .pull_indexes
            .get_or_init(|| relay_has_pull_indexes(conn))
            .await
    }

    fn new(
        options: &PullOptions,
        skip_skins: bool,
        now_unix_ms: i64,
        group_key: &GroupKey,
    ) -> Self {
        let mut filter = RowFilter {
            capture_cutoff: options.capture_window_ms.map(|w| now_unix_ms - w),
            item_cutoff: options.item_window_ms.map(|w| now_unix_ms - w),
            non_google_calendar_only: options.non_google_calendar_only,
            ..Default::default()
        };
        let Some(allowlist) = &options.row_type_allowlist else {
            return filter;
        };
        let mut types: Vec<String> = Vec::new();
        let mut kinds: Vec<String> = Vec::new();
        let mut every_kind = false;
        let explicit_meeting = allowlist.iter().any(|e| e == ROW_TYPE_MEETING);
        let add = |t: &str, types: &mut Vec<String>| {
            if !types.iter().any(|have| have == t) {
                types.push(t.to_string());
            }
        };
        for entry in allowlist {
            match entry.strip_prefix("items:") {
                Some(kind) => {
                    add(ROW_TYPE_ITEMS, &mut types);
                    if !kinds.iter().any(|k| k == kind) {
                        kinds.push(kind.to_string());
                    }
                    match kind {
                        "capture" => add(ROW_TYPE_CAPTURE_ITEM, &mut types),
                        "transcript" => {
                            add(ROW_TYPE_TRANSCRIPT_SEGMENT, &mut types);
                            add(ROW_TYPE_MEETING, &mut types);
                        }
                        _ => {}
                    }
                }
                None => {
                    every_kind |= entry == ROW_TYPE_ITEMS;
                    add(entry, &mut types);
                    if entry == ROW_TYPE_TRANSCRIPT_SEGMENT || entry == ROW_TYPE_ITEMS {
                        add(ROW_TYPE_MEETING, &mut types);
                    }
                }
            }
        }
        for t in ALWAYS_PULLED_ROW_TYPES {
            add(t, &mut types);
        }
        if skip_skins {
            types.retain(|t| t != ROW_TYPE_SKIN && t != ROW_TYPE_SKIN_IMAGE_CHUNK);
        }
        filter.meetings_as_parents_only =
            !explicit_meeting && types.iter().any(|t| t == ROW_TYPE_MEETING);
        let type_count = types.len();
        filter.row_types = Some(types);
        filter.kind_tags = (!every_kind && !kinds.is_empty()).then(|| {
            kinds
                .iter()
                .map(|kind| sync::kind_tag(group_key, kind))
                .collect()
        });
        if filter.kind_tags.is_some() {
            filter.slice_tags = filter.slices(&kinds, type_count, group_key);
        }
        filter.item_kinds = (!every_kind).then_some(kinds);
        filter
    }

    /// The slice tags for `kinds`: each kind, each source class it may carry in this pull (a calendar event only
    /// `Other` when Google Calendar is left out, else `Google` and `Other`; any other kind `None`), each month from
    /// the item cutoff's through [`sync::MAX_MONTH_BUCKET`] (every month without a window, so Google events are
    /// still left out). A sliced row of the cutoff's month is fetched and checked after decrypting, like any
    /// other. `None` when the query would carry more than [`MAX_TAG_QUERY_VARIABLES`]; the kind tags alone then.
    fn slices(
        &self,
        kinds: &[String],
        type_count: usize,
        group_key: &GroupKey,
    ) -> Option<Vec<String>> {
        use sync::SourceClass;
        let first = self.item_cutoff.map_or(0, sync::month_bucket);
        let months = first..=sync::MAX_MONTH_BUCKET;
        let classes = |kind: &str| -> &'static [SourceClass] {
            match kind {
                "calendar_event" if self.non_google_calendar_only => &[SourceClass::Other],
                "calendar_event" => &[SourceClass::Google, SourceClass::Other],
                _ => &[SourceClass::None],
            }
        };
        let count: usize = kinds.iter().map(|k| classes(k).len() * months.len()).sum();
        if type_count + kinds.len() + count > MAX_TAG_QUERY_VARIABLES {
            if !SLICE_TAGS_TOO_MANY_LOGGED.swap(true, Ordering::Relaxed) {
                eprintln!("[sync] {count} slice tags would make the pull query too long: it filters items by kind only");
            }
            return None;
        }
        let mut tags = Vec::with_capacity(count);
        for kind in kinds {
            for class in classes(kind) {
                tags.extend(
                    months
                        .clone()
                        .map(|month| sync::slice_tag(group_key, kind, *class, month)),
                );
            }
        }
        Some(tags)
    }

    /// The kind tags to put in the relay query: none without a kind restriction, or once this pull has
    /// found the relay without the column (the filter lives for one round). Never decided by `KIND_TAG_COLUMN_SEEN` (the runner's relay
    /// may not have it whatever this process saw); a query that finds no column falls back instead.
    fn query_kind_tags(&self) -> Option<&[String]> {
        if self.no_kind_tag_column.load(Ordering::Relaxed) {
            return None;
        }
        self.kind_tags.as_deref()
    }

    /// The slice tags to put in the relay query beside [`RowFilter::query_kind_tags`]: never without them (the
    /// slice clause falls back to the kind tag), and none once this round has found the relay without the
    /// `slice_tag` column.
    fn query_slice_tags(&self) -> Option<&[String]> {
        if self.no_slice_tag_column.load(Ordering::Relaxed) {
            return None;
        }
        self.query_kind_tags()?;
        self.slice_tags.as_deref()
    }

    /// Whether a tag-filtered query failed only because the relay has no `slice_tag` or no `kind_tag`; if so,
    /// the rest of this round queries without slice tags, or without any tags (the post-decrypt checks still
    /// filter). A missing `slice_tag` counts once, so the retry without slices can never loop.
    fn tag_column_missing(&self, error: &SyncEngineError) -> bool {
        let error = error.to_string();
        if is_no_such_slice_tag_column(&error) {
            let first = !self.no_slice_tag_column.swap(true, Ordering::Relaxed);
            if first {
                eprintln!("[sync] the relay has no slice_tag column: this round filters items by kind only");
            }
            return first;
        }
        let missing = is_no_such_kind_tag_column(&error);
        if missing && !self.no_kind_tag_column.swap(true, Ordering::Relaxed) {
            eprintln!(
                "[sync] the relay has no kind_tag column: this round fetches every item kind"
            );
        }
        missing
    }

    fn kind_allowed(&self, kind: &str) -> bool {
        self.item_kinds
            .as_ref()
            .is_none_or(|kinds| kinds.iter().any(|k| k == kind))
    }

    /// A calendar event this pull leaves out for its source (see `PullOptions::non_google_calendar_only`).
    fn drops_calendar_source(&self, kind: &str, source: Option<&str>) -> bool {
        self.non_google_calendar_only
            && kind == "calendar_event"
            && source
                .map(str::trim)
                .is_none_or(|s| s.is_empty() || s == "google_calendar")
    }

    /// An `items` row of `kind` dated `ts_unix_ms` is outside a window.
    fn outside_item_window(&self, kind: &str, ts_unix_ms: i64) -> bool {
        let before = |cutoff: Option<i64>| cutoff.is_some_and(|c| ts_unix_ms < c);
        before(self.item_cutoff) || (kind == "capture" && before(self.capture_cutoff))
    }

    /// Whether a legacy `capture_item` row has to be decoded for its date.
    fn windows_captures(&self) -> bool {
        self.capture_cutoff.is_some() || self.item_cutoff.is_some()
    }

    /// Whether a meeting row is held back from applying: pulled only as a parent, or (with an item window)
    /// started before it. Never with `Default` options.
    fn holds_meetings(&self) -> bool {
        self.meetings_as_parents_only || self.item_cutoff.is_some()
    }

    fn meeting_held(&self, start_unix_ms: i64) -> bool {
        self.meetings_as_parents_only || self.item_cutoff.is_some_and(|c| start_unix_ms < c)
    }
}

/// [`pull_content_with_options`] for a pull-only peer (the phone, which has no skins): skin rows are
/// left out at the query, so neither the skins nor their images are fetched or applied. It pulls a
/// backlog newest first (#572): see [`pull_round_mode`].
pub(crate) async fn pull_content_without_skins(
    conn: &Connection,
    group_key: &GroupKey,
    store: &Store,
    own_device_id: &str,
    options: &PullOptions,
) -> Result<(), SyncEngineError> {
    let pace = pull_only_pace(more_to_pull());
    // Cleared up front, so a failed or timed-out round falls back to the normal pace next time.
    MORE_TO_PULL.store(false, std::sync::atomic::Ordering::Relaxed);
    pull_content_with_budget_mode(
        conn,
        group_key,
        store,
        own_device_id,
        pace,
        options,
        true,
        true,
    )
    .await
    .map(|more| MORE_TO_PULL.store(more, std::sync::atomic::Ordering::Relaxed))
}

/// One pull round of the cloud runner: as the phone's (no skins, newest first), at the catch-up pace,
/// since its store starts empty every run. Returns whether rows are still waiting (an open gap), and
/// how many rows the round applied and fetched.
pub(crate) async fn runner_pull_round(
    conn: &Connection,
    group_key: &GroupKey,
    store: &Store,
    own_device_id: &str,
    options: &PullOptions,
) -> Result<RunnerRound, SyncEngineError> {
    let mut stats = Vec::new();
    let result = pull_round_mode(
        conn,
        group_key,
        store,
        own_device_id,
        CATCH_UP_PACE,
        options,
        true,
        true,
        &mut stats,
    )
    .await;
    if let (Ok(json), Ok(mut last)) = (serde_json::to_string(&stats), LAST_PULL_STATS.lock()) {
        *last = json;
    }
    let mut round = RunnerRound {
        applied: stats.iter().map(|s| s.applied).sum(),
        fetched: stats.iter().map(|s| s.fetched + s.gap_fetched).sum(),
        already_applied: stats.iter().map(|s| s.already_applied).sum(),
        undecodable: stats.iter().map(|s| s.undecodable).sum(),
        skipped_allowlist: stats.iter().map(|s| s.skipped_allowlist).sum(),
        skipped_window: stats.iter().map(|s| s.skipped_window).sum(),
        skip_mcp: stats.iter().map(|s| s.skip_mcp).sum(),
        skip_kind: stats.iter().map(|s| s.skip_kind).sum(),
        skip_google_cal: stats.iter().map(|s| s.skip_google_cal).sum(),
        skip_item_window: stats.iter().map(|s| s.skip_item_window).sum(),
        refetch: stats.iter().map(|s| s.refetch).sum(),
        deferred: stats.iter().map(|s| s.deferred).sum(),
        fetch_ms: stats
            .iter()
            .map(|s| s.fetch_ms + s.gap_fetch_us / 1000)
            .sum(),
        apply_ms: stats
            .iter()
            .map(|s| s.apply_ms + s.gap_apply_us / 1000)
            .sum(),
        slice_tags: stats.iter().map(|s| s.slice_tags).max().unwrap_or(0),
        pull_index: stats.iter().any(|s| s.pull_index),
        ..Default::default()
    };
    for s in &stats {
        for (total, add) in round.by_type.iter_mut().zip(s.by_type) {
            total.fetched += add.fetched;
            total.applied += add.applied;
        }
    }
    result.map(|more| RunnerRound { more, ..round })
}

/// What one [`runner_pull_round`] read: counts and milliseconds only.
#[derive(Default)]
pub(crate) struct RunnerRound {
    pub more: bool,
    pub applied: usize,
    pub fetched: usize,
    pub by_type: [crate::PullTypeCounts; crate::PULL_TYPE_NAMES.len()],
    pub already_applied: usize,
    pub undecodable: usize,
    pub skipped_allowlist: usize,
    pub skipped_window: usize,
    pub skip_mcp: usize,
    pub skip_kind: usize,
    pub skip_google_cal: usize,
    pub skip_item_window: usize,
    pub refetch: usize,
    pub deferred: usize,
    pub fetch_ms: u64,
    pub apply_ms: u64,
    /// The slice tags a query carried (the same every query of a round): the most any peer's did.
    pub slice_tags: usize,
    /// An indexed fetch ran for at least one query this round.
    pub pull_index: bool,
}

/// Whether every other device on the relay has announced the topics capability. A sender holds its client
/// settings and filings (`topic_seed`, `topic_verdict`) back while any device lacks it, so until then a pull
/// that read every row can still be missing an exclusion: the cloud runner must not treat "no client
/// settings read" as "nothing excluded".
pub(crate) async fn client_settings_published(
    conn: &Connection,
    own_device_id: &str,
) -> Result<bool, SyncEngineError> {
    Ok(
        peers_without_marker(conn, own_device_id, ROW_TYPE_TOPICS_CAPABILITY)
            .await?
            .is_empty(),
    )
}

/// The cloud runner's own earlier results: the `row_types` rows (of `routine_run` and `meeting_prep`) this
/// device sent in earlier runs, read back into its new store as a peer's would be. Its store starts empty
/// each run and a pull reads only other devices, so without them a second run the same day (a retry, or
/// a later job for meeting prep) would not see today's brief or a meeting's brief and would write it
/// again. A row that does not decrypt or apply is skipped. Returns how many applied.
pub(crate) async fn read_own_results(
    conn: &Connection,
    group_key: &GroupKey,
    store: &Store,
    own_device_id: &str,
    row_types: &[&str],
) -> Result<usize, SyncEngineError> {
    let row_types: Vec<&str> = row_types
        .iter()
        .copied()
        .filter(|t| [ROW_TYPE_ROUTINE_RUN, ROW_TYPE_MEETING_PREP].contains(t))
        .collect();
    if row_types.is_empty() {
        return Ok(0);
    }
    let placeholders = (0..row_types.len())
        .map(|i| format!("?{}", i + 2))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!("SELECT local_seq, row_type, ciphertext FROM sync_row WHERE device_id = ?1 AND row_type IN ({placeholders}) ORDER BY local_seq");
    let mut values = vec![Value::Text(own_device_id.to_string())];
    values.extend(row_types.iter().map(|t| Value::Text(t.to_string())));
    let rows = conn
        .query(&sql, Params::Positional(values))
        .await
        .map_err(query_err)?;
    let mut applied = 0;
    for env in read_envelopes(rows).await? {
        let Some(plaintext) = decrypt_row(group_key, own_device_id, &env) else {
            continue;
        };
        if apply_parent_row(
            store,
            own_device_id,
            env.local_seq,
            &env.row_type,
            &plaintext,
        )
        .is_ok()
        {
            applied += 1;
        }
    }
    Ok(applied)
}

/// For the cloud runner's store, which is new every run: no cursor of it has moved past a tombstone, so its
/// live ranges read every tombstone row from the start, as any first pull does. The once-per-database
/// catch-up exists for a device whose cursor an older build moved past tombstones; redone from zero every run
/// (the store is in memory) it spends most of each round and, on a large account, never finishes inside the
/// pull budget. A pull that does not read every row is still reported incomplete, so nothing runs on rows
/// a delete removed. Call only on a store that has never pulled.
pub(crate) fn mark_new_store_tombstones_read(store: &Store) -> Result<(), SyncEngineError> {
    store
        .set_meta(TOMBSTONE_CATCH_UP_KEY, TOMBSTONE_TYPES_VERSION)
        .map_err(query_err)
}

/// Whether this database has read every peer's tombstones once ([`catch_up_tombstones`]). A desktop or
/// phone store that starts empty is not, until its catch-up finishes; the runner's new store is marked done
/// ([`mark_new_store_tombstones_read`]).
pub(crate) fn tombstones_caught_up(store: &Store) -> bool {
    old_build_puller()
        || store
            .get_meta(TOMBSTONE_CATCH_UP_KEY)
            .ok()
            .flatten()
            .as_deref()
            == Some(TOMBSTONE_TYPES_VERSION)
}

/// Whether any peer's rows still wait on a dependency (a transcript line whose meeting was not yet read).
pub(crate) async fn any_pending_dependency(
    conn: &Connection,
    store: &Store,
    own_device_id: &str,
) -> Result<bool, SyncEngineError> {
    for peer in distinct_peer_device_ids(conn, own_device_id).await? {
        if !store
            .list_sync_pending_dependencies(&peer)
            .map_err(query_err)?
            .is_empty()
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The highest `local_seq` this device has on the relay (0 with none): the runner's per-run store
/// numbers its rows above it.
pub(crate) async fn own_top_seq(
    conn: &Connection,
    own_device_id: &str,
) -> Result<i64, SyncEngineError> {
    Ok(peer_top_seq(conn, own_device_id).await?.unwrap_or(0))
}

/// [`pull_content`] with [`PullOptions`] (the phone's capture window).
pub async fn pull_content_with_options(
    conn: &Connection,
    group_key: &GroupKey,
    store: &Store,
    own_device_id: &str,
    options: &PullOptions,
) -> Result<(), SyncEngineError> {
    pull_content_with_budget(
        conn,
        group_key,
        store,
        own_device_id,
        NORMAL_PACE,
        options,
        false,
    )
    .await
    .map(|more| MORE_TO_PULL.store(more, std::sync::atomic::Ordering::Relaxed))
}

/// Set by each pull: it stopped with rows still waiting, so the shell can come back soon.
static MORE_TO_PULL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether the last pull left rows on the relay.
pub fn more_to_pull() -> bool {
    MORE_TO_PULL.load(std::sync::atomic::Ordering::Relaxed)
}

/// One peer's pass in the last pull round (#572: tells slow progress from a loop).
/// `peer` is a short hash tag, never the device id.
#[derive(Debug, Default, serde::Serialize)]
struct PeerPullStats {
    peer: String,
    cursor_before: i64,
    processed_through: i64,
    fetched: usize,
    fetched_through: i64,
    applied: usize,
    already_applied: usize,
    skipped_window: usize,
    /// `items` rows of a kind the allowlist leaves out (the runner). Left out of the JSON when 0, so a
    /// phone's or desktop's stats read as before.
    #[serde(skip_serializing_if = "is_zero")]
    skipped_allowlist: usize,
    /// Of `skipped_allowlist`/`skipped_window`: why an `items` row was left out (runner diagnostics; not in the JSON).
    #[serde(skip)]
    skip_mcp: usize,
    #[serde(skip)]
    skip_kind: usize,
    #[serde(skip)]
    skip_google_cal: usize,
    #[serde(skip)]
    skip_item_window: usize,
    deferred: usize,
    pending_before: usize,
    pending_after: usize,
    undecodable: usize,
    budget_stopped: bool,
    /// Time fetching the forward page from the relay, and applying it.
    fetch_ms: u64,
    apply_ms: u64,
    /// Rows fetched and applied per row type (the runner's pull diagnostics); not in the JSON.
    /// Rows of a page left unhandled (a budget stopped it), to be fetched again.
    #[serde(skip)]
    refetch: usize,
    /// Gap pages' fetch and apply time in microseconds (the forward page's are `fetch_ms`/`apply_ms`); the
    /// runner's report adds both to its totals.
    #[serde(skip)]
    gap_fetch_us: u64,
    #[serde(skip)]
    gap_apply_us: u64,
    #[serde(skip)]
    by_type: [crate::PullTypeCounts; crate::PULL_TYPE_NAMES.len()],
    /// How many slice tags this peer's queries carried (after any fallback): the runner's diagnostics.
    #[serde(skip)]
    slice_tags: usize,
    /// An indexed fetch (not the span-window scan) served at least one of this peer's queries.
    #[serde(skip)]
    pull_index: bool,
    /// The page was full (this peer is behind), so its items were stored without embeddings.
    deferred_embedding: bool,
    /// Newest first only (#572): this round recorded the peer's gap and moved the live cursor
    /// to the peer's top (a never-pulled peer's `cursor_before` is then that top).
    gap_opened: bool,
    /// Rows the gap page fetched (the counts above include what it applied).
    gap_fetched: usize,
    /// The gap still open after this round (both exclusive); `None` once it is pulled.
    gap_lo: Option<i64>,
    gap_hi: Option<i64>,
    /// The peer has no rows left on the relay (removed, or a relay reset), so its open gap
    /// was closed with nothing more to pull. Only this field is set on such an entry.
    gap_closed_peer_gone: bool,
}

impl PeerPullStats {
    /// Called after a fetch, so a query that fell back for a missing `slice_tag` column counts none.
    fn note_slice_tags(&mut self, filter: &RowFilter) {
        self.slice_tags = self
            .slice_tags
            .max(filter.query_slice_tags().map_or(0, <[String]>::len));
    }
}

/// Set by each pull round: one entry per peer, as JSON.
static LAST_PULL_STATS: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// The last pull round's per-peer stats as a JSON array (empty string before any pull).
/// Holds counts, local_seqs and hashed peer tags only: no ids, URLs or content.
pub fn last_pull_stats_json() -> String {
    LAST_PULL_STATS
        .lock()
        .map(|s| s.clone())
        .unwrap_or_default()
}

impl PeerPullStats {
    fn count_fetched(&mut self, envelopes: &[RowEnvelope]) {
        for env in envelopes {
            self.by_type[crate::pull_type_index(&env.row_type)].fetched += 1;
        }
    }

    fn count_applied(&mut self, row_type: &str) {
        self.applied += 1;
        self.by_type[crate::pull_type_index(row_type)].applied += 1;
    }
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

fn peer_tag(peer: &str) -> String {
    memory::skin_sync::sha256_hex(peer.as_bytes())[..6].to_string()
}

/// [`pull_content`] with the [`Pace`] as a parameter. Each peer's pass always processes
/// at least one row, so a zero budget (tests) still makes progress.
/// Publishes the round's stats (even a failed round's, up to the failure).
async fn pull_content_with_budget(
    conn: &Connection,
    group_key: &GroupKey,
    store: &Store,
    own_device_id: &str,
    pace: Pace,
    options: &PullOptions,
    skip_skins: bool,
) -> Result<bool, SyncEngineError> {
    pull_content_with_budget_mode(
        conn,
        group_key,
        store,
        own_device_id,
        pace,
        options,
        skip_skins,
        false,
    )
    .await
}

/// [`pull_content_with_budget`], newest first when `newest_first` (the phone, #572).
#[allow(clippy::too_many_arguments)]
async fn pull_content_with_budget_mode(
    conn: &Connection,
    group_key: &GroupKey,
    store: &Store,
    own_device_id: &str,
    pace: Pace,
    options: &PullOptions,
    skip_skins: bool,
    newest_first: bool,
) -> Result<bool, SyncEngineError> {
    let mut stats = Vec::new();
    let result = pull_round_mode(
        conn,
        group_key,
        store,
        own_device_id,
        pace,
        options,
        skip_skins,
        newest_first,
        &mut stats,
    )
    .await;
    if let (Ok(json), Ok(mut last)) = (serde_json::to_string(&stats), LAST_PULL_STATS.lock()) {
        *last = json;
    }
    result
}

/// Tests drive one round directly; production goes through `pull_round_mode`.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
async fn pull_round(
    conn: &Connection,
    group_key: &GroupKey,
    store: &Store,
    own_device_id: &str,
    pace: Pace,
    options: &PullOptions,
    skip_skins: bool,
    all_stats: &mut Vec<PeerPullStats>,
) -> Result<bool, SyncEngineError> {
    pull_round_mode(
        conn,
        group_key,
        store,
        own_device_id,
        pace,
        options,
        skip_skins,
        false,
        all_stats,
    )
    .await
}

/// One pull round over every peer: its live range forward from the cursor, then its pending
/// retries. `newest_first` (the phone, #572) also pulls a backlog newest first. A peer it has
/// never pulled (cursor 0), or whose live page comes back full (behind by a page or more),
/// gets a gap: the rows from the cursor up to the peer's current top, recorded in the same
/// local transaction that moves the live cursor to that top, so no row is in neither range.
/// After every peer's live range, [`pull_gaps`] pulls each open gap downward. Recent meetings
/// then arrive in the first round and older history fills in afterwards. A caught-up peer
/// never gets a gap, so its rounds are exactly as before. Returns whether rows still wait,
/// which includes any open gap.
#[allow(clippy::too_many_arguments)]
async fn pull_round_mode(
    conn: &Connection,
    group_key: &GroupKey,
    store: &Store,
    own_device_id: &str,
    pace: Pace,
    options: &PullOptions,
    skip_skins: bool,
    newest_first: bool,
    all_stats: &mut Vec<PeerPullStats>,
) -> Result<bool, SyncEngineError> {
    let mut more = false;
    // Before the live ranges, so an update applies the tombstones its older build skipped in this round.
    catch_up_tombstones(conn, group_key, store, own_device_id).await;
    let round_started = std::time::Instant::now();
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
    let filter = RowFilter::new(options, skip_skins, now_unix_ms(), group_key);
    restore_own_provider_spend(conn, group_key, store, own_device_id).await?;
    let peers = distinct_peer_device_ids(conn, own_device_id).await?;
    let first_stats = all_stats.len();
    for peer in &peers {
        let peer = peer.as_str();
        // This peer's forward budget: its own, cut to what's left of the round's.
        let budget = pace.round_budget.map_or(pace.peer_budget, |round| {
            pace.peer_budget
                .min(round.saturating_sub(round_started.elapsed()))
        });
        let mut cursor = store
            .get_sync_cursor(peer)
            .map_err(|e| SyncEngineError::Query(e.to_string()))?;
        let known_skin_peer = store
            .is_skin_peer(peer)
            .map_err(|e| SyncEngineError::Query(e.to_string()))?;
        if !skip_skins && !known_skin_peer {
            let mut marker = conn
                .query(PEER_MARKER_SQL, params![peer, ROW_TYPE_SKINS_CAPABILITY])
                .await
                .map_err(|e| SyncEngineError::Query(e.to_string()))?;
            if marker
                .next()
                .await
                .map_err(|e| SyncEngineError::Query(e.to_string()))?
                .is_some()
            {
                store
                    .record_skin_peer(peer)
                    .map_err(|e| SyncEngineError::Query(e.to_string()))?;
            }
        }
        // A peer never pulled before: everything it has is the gap, and the live range starts
        // at its top, so its newest rows come first.
        let mut gap_opened = false;
        if newest_first && cursor == 0 && store.sync_gap(peer).map_err(query_err)?.is_none() {
            if let Some(top) = peer_top_seq(conn, peer).await?.filter(|top| *top > 0) {
                store
                    .open_sync_gap(peer, 0, top + 1, top)
                    .map_err(query_err)?;
                cursor = top;
                gap_opened = true;
            }
        }
        // Snapshot before this pass records its own new pending rows: those were just
        // tried, so retrying them in the same round would only repeat the lookup.
        let pending_before = store
            .list_sync_pending_dependencies(peer)
            .map_err(|e| SyncEngineError::Query(e.to_string()))?;
        all_stats.push(PeerPullStats {
            peer: peer_tag(peer),
            cursor_before: cursor,
            processed_through: cursor,
            pending_before: pending_before.len(),
            gap_opened,
            ..Default::default()
        });
        let stats = all_stats.last_mut().expect("just pushed");
        // The round is spent: fetch only the one row this peer still applies (every peer progresses),
        // not a page it can't apply, which costs rows read and data for nothing.
        let spent = pace.round_budget.is_some() && budget.is_zero();
        let rows = if spent { 1 } else { pace.rows };
        // The runner's kind filter reads the live range in windows of `local_seq` up to the peer's top as
        // read here ([`FILTERED_PULL_SPAN`]). Without it (every phone and desktop pull) this loop runs once,
        // exactly as before. Keyed on the kind tags alone: slice tags are only ever queried beside them, and
        // narrow the same query further.
        let windowed_top = match filter.query_kind_tags() {
            Some(_) => peer_top_seq(conn, peer).await?,
            None => None,
        };
        let pass_started = std::time::Instant::now();
        let mut from = cursor;
        let (full_page, processed_through) = loop {
            let mut window_hi =
                windowed_top.map(|top| from.saturating_add(filtered_pull_span()).min(top));
            let fetch_started = std::time::Instant::now();
            // With the relay's pull indexes the window is the whole range up to the peer's top. Its cost tracks
            // matches, not positions, but not linearly (Q1/Q2 ~ matches^2 / limit, Q3 ~ unsliced rows; see
            // [`fetch_rows_indexed`]). If the indexed fetch gives way, this page scans its span window.
            let indexed = match windowed_top {
                Some(top) if filter.use_pull_indexes(conn).await => {
                    fetch_rows_indexed(conn, peer, from, top, rows, &filter)
                        .await?
                        .inspect(|_| window_hi = Some(top))
                }
                _ => None,
            };
            let indexed_used = indexed.is_some();
            let (envelopes, windowed) = match indexed {
                Some(envelopes) => (envelopes, true),
                None => {
                    fetch_rows_since(
                        conn,
                        peer,
                        from,
                        window_hi.unwrap_or(i64::MAX),
                        rows,
                        skip_skins,
                        &filter,
                    )
                    .await?
                }
            };
            stats.fetch_ms += fetch_started.elapsed().as_millis() as u64;
            stats.pull_index |= indexed_used;
            stats.note_slice_tags(&filter);
            let full_page = envelopes.len() as i64 >= rows;
            more |= spent || full_page;
            stats.fetched += envelopes.len();
            stats.count_fetched(&envelopes);
            stats.fetched_through = envelopes.last().map_or(from, |e| e.local_seq);
            let apply_started = std::time::Instant::now();
            // A full page means this peer is behind: on a desktop its items are stored without
            // embeddings, which the reindex sweep adds later, so a backlog applies at row speed
            // instead of ~0.5 s a row. A caught-up peer still embeds each row as it arrives. A
            // pull-only phone (`skip_skins`) keeps embedding as before.
            let defer = full_page && !skip_skins;
            stats.deferred_embedding |= defer;
            // The first page has the whole budget, as without windows; a later window what is left.
            let page_budget = if from == cursor {
                budget
            } else {
                budget.saturating_sub(pass_started.elapsed())
            };
            let page = with_defer_embedding(
                store,
                defer,
                apply_page(
                    conn,
                    group_key,
                    store,
                    peer,
                    &envelopes,
                    page_budget,
                    &filter,
                    stats,
                ),
            )
            .await;
            let page = page?;
            stats.apply_ms += apply_started.elapsed().as_millis() as u64;
            stats.refetch += envelopes.len() - page.handled;
            more |= page.stopped;
            // The highest local_seq this pass handled (applied, skipped, or deferred to the
            // dependency pass): where the cursor goes, since the pass can stop early.
            let handled_through = page
                .handled
                .checked_sub(1)
                .map_or(from, |last| envelopes[last].local_seq);
            // A short window handled to its end: every row up to the window's end was read (the query
            // left out only rows the post-decrypt checks would skip: another kind's, and with slice tags a
            // Google event the pull leaves out or an item from before the cutoff's month, judged on the same
            // source and date the push tagged), so the cursor moves to it, as an unfiltered pull's would over
            // the skipped rows.
            let window_read = windowed && !full_page && page.handled == envelopes.len();
            match (window_hi, windowed_top) {
                (Some(hi), Some(top)) if window_read && hi > from => {
                    store.set_sync_cursor(peer, hi).map_err(query_err)?;
                    from = hi;
                    if hi < top {
                        if pass_started.elapsed() < budget {
                            continue;
                        }
                        more = true;
                    }
                    break (false, hi);
                }
                _ => break (full_page, handled_through),
            }
        };

        // Advance to the highest row this pass handled: every fetched row, unless the time
        // budget stopped it early, in which case the rest are fetched again next pass.
        // Unresolved dependencies are tracked in sync_pending_dependency and retried
        // independently above, so a row that may never resolve no longer caps how far
        // this device can ever pull.
        let new_cursor = processed_through;
        store
            .set_sync_cursor(peer, new_cursor)
            .map_err(|e| SyncEngineError::Query(e.to_string()))?;
        stats.processed_through = new_cursor;

        // Behind by a page or more: the rest up to the peer's top becomes the gap, and the
        // live range jumps to the top. Never on a spent round's one-row page.
        if newest_first && !spent && full_page && store.sync_gap(peer).map_err(query_err)?.is_none()
        {
            if let Some(top) = peer_top_seq(conn, peer)
                .await?
                .filter(|top| *top > processed_through)
            {
                store
                    .open_sync_gap(peer, processed_through, top + 1, top)
                    .map_err(query_err)?;
                stats.gap_opened = true;
            }
        }

        // Retry every row still waiting on a dependency from an earlier pass. These can sit
        // well behind the cursor's window (the cursor no longer waits for them), so they're
        // looked up by key, in batches, AFTER the cursor is saved: the forward pull never
        // waits on this pass, and a meeting fetched above already resolves its segments
        // here. The pass has its own time budget, so a large pending set (227 entries hit
        // SYNC_TIMEOUT on the phone, one round trip each) can never stall the round; what
        // it doesn't reach stays pending for the next round. Always one chunk, like the
        // forward pass's one row, so a zero budget still progresses.
        let pending_started = std::time::Instant::now();
        // Never more than the normal 3 s, even when catching up (see CATCH_UP_PACE).
        let pending_budget = budget.min(PULL_PEER_TIME_BUDGET);
        let mut retry = Vec::new();
        for (dep_local_seq, dep_row_type) in pending_before {
            if store
                .resolve_sync_row_map(peer, dep_local_seq, &dep_row_type)
                .map_err(|e| SyncEngineError::Query(e.to_string()))?
                .is_some()
            {
                // Already resolved by some other path -- just clear the stale entry.
                store
                    .delete_sync_pending_dependency(peer, dep_local_seq, &dep_row_type)
                    .map_err(|e| SyncEngineError::Query(e.to_string()))?;
                continue;
            }
            retry.push((dep_local_seq, dep_row_type));
        }
        for (i, chunk) in retry.chunks(PENDING_LOOKUP_CHUNK).enumerate() {
            // ponytail: the budget is checked per chunk, so one chunk's applies run to the end
            // (fine without embedding; with it, check the budget per row in resolve_segments).
            if i > 0 && pending_started.elapsed() >= pending_budget {
                break;
            }
            // An undecodable row is dropped here: it stays pending and is retried next round
            // rather than lost. A vanished row is gone for good (a forgotten meeting's recorder
            // deletes it, the relay compacts it, or a fresh start wipes the database), so it
            // stops waiting.
            let fetched = fetch_rows_by_key(conn, peer, chunk).await?;
            for (seq, row_type) in chunk {
                if !fetched
                    .iter()
                    .any(|env| env.local_seq == *seq && env.row_type == *row_type)
                {
                    store
                        .delete_sync_pending_dependency(peer, *seq, row_type)
                        .map_err(|e| SyncEngineError::Query(e.to_string()))?;
                }
            }
            let pending = fetched
                .into_iter()
                .filter_map(|env| decode_pending_segment(group_key, peer, &env).map(|p| (env, p)))
                .collect();
            resolve_segments(conn, group_key, store, peer, pending, &filter, stats).await?;
        }
        stats.pending_after = store
            .list_sync_pending_dependencies(peer)
            .map_err(|e| SyncEngineError::Query(e.to_string()))?
            .len();
    }
    if newest_first {
        pull_gaps(
            conn,
            group_key,
            store,
            &peers,
            pace,
            round_started,
            skip_skins,
            &filter,
            &mut all_stats[first_stats..],
        )
        .await?;
        // A gap whose peer has no rows left on the relay closes as an empty page would, so it
        // never keeps older history loading forever.
        for gone in store.sync_gap_peers().map_err(query_err)? {
            if !peers.contains(&gone) {
                store.close_sync_gap(&gone).map_err(query_err)?;
                eprintln!(
                    "[sync] peer {} left the relay; its older history is closed",
                    peer_tag(&gone)
                );
                all_stats.push(PeerPullStats {
                    peer: peer_tag(&gone),
                    gap_closed_peer_gone: true,
                    ..Default::default()
                });
            }
        }
        more |= store.older_sync_history_loading().map_err(query_err)?;
    }
    // A peer's client deletes applied this round (or in an earlier round whose clean-up failed): the full-text
    // index pages are merged once, as on the deleting device (X6 final review, Major 4). Never fails the pull.
    if let Err(e) = store.run_owed_client_cleanup() {
        eprintln!("[sync] index clean-up after a peer's delete resumes next round: {e}");
    }
    Ok(more)
}

/// The gap pass (#572): one page per open gap, newest first, after every peer's live range and
/// pending retries, so a gap never holds up any peer's live rows. It gets what is left of the
/// round's budget (one peer budget for a pace without a round budget), each gap at most a peer
/// budget of it, and every gap still handles at least one row a round, so a zero budget
/// finishes too. A page saves the gap's upper bound at the lowest row it handled; a short page
/// handled to its end, or an empty one, closes the gap. The fetch counts against the gap's
/// budget, and the page is sized to the budget left ([`gap_page_rows`]).
#[allow(clippy::too_many_arguments)]
async fn pull_gaps(
    conn: &Connection,
    group_key: &GroupKey,
    store: &Store,
    peers: &[String],
    pace: Pace,
    round_started: std::time::Instant,
    skip_skins: bool,
    filter: &RowFilter,
    peer_stats: &mut [PeerPullStats],
) -> Result<(), SyncEngineError> {
    let gaps_started = std::time::Instant::now();
    let total = pace.round_budget.map_or(pace.peer_budget, |round| {
        round.saturating_sub(round_started.elapsed())
    });
    for (peer, stats) in peers.iter().zip(peer_stats.iter_mut()) {
        let Some((gap_lo, gap_hi)) = store.sync_gap(peer).map_err(query_err)? else {
            continue;
        };
        let peer_started = std::time::Instant::now();
        let budget = pace
            .peer_budget
            .min(total.saturating_sub(gaps_started.elapsed()));
        // Under a second left: fetch only the one row it still handles, as a spent round does,
        // not a page it can't apply.
        let mut rows = gap_page_rows(pace, budget);
        let mut gap_hi = gap_hi;
        // Without the runner's kind filter this loop runs once, exactly as before. With it (and its slice
        // tags, which never come without it), a page on the scan reads one window of `local_seq` below the gap's
        // top ([`FILTERED_PULL_SPAN`]); a short window handled to its end lowers the gap's top to the window's
        // floor and the next window follows while the gap's budget lasts. With the relay's pull indexes as well
        // ([`fetch_gap_rows_indexed`]) the window is the whole gap, where span windows over ~98M positions would
        // take thousands of rounds. Its cost is bounded by matches, not positions, but not linearly: Q1 and Q2 sort
        // every match left in the gap on each page (about `matches^2 / limit` over the gap), and Q3 tracks the
        // peer's unsliced `items` rows (`kind_tag` is not in the index). Against the span scan's one read per
        // position the crossover is about `sqrt(positions * limit)`: ~220k matches per device gap at 98M
        // positions and `limit` 500. A short page closes the gap. If the indexed fetch gives way, later pages go
        // back to span windows from the gap's current top.
        loop {
            let mut window_lo = match filter.query_kind_tags() {
                Some(_) => gap_lo.max(
                    gap_hi
                        .saturating_sub(filtered_pull_span())
                        .saturating_sub(1),
                ),
                None => gap_lo,
            };
            let fetch_started = std::time::Instant::now();
            // With the relay's pull indexes the window is the whole gap (see above). If they give way, this page
            // scans its span window.
            let indexed = match filter.query_kind_tags() {
                Some(_) if filter.use_pull_indexes(conn).await => {
                    fetch_gap_rows_indexed(conn, peer, gap_lo, gap_hi, rows, filter).await?
                }
                _ => None,
            };
            if indexed.is_some() {
                window_lo = gap_lo;
            }
            let indexed_used = indexed.is_some();
            let (envelopes, windowed) = match indexed {
                Some(envelopes) => (envelopes, true),
                None => {
                    fetch_gap_rows(
                        conn, peer, gap_lo, window_lo, gap_hi, rows, skip_skins, filter,
                    )
                    .await?
                }
            };
            stats.gap_fetch_us += fetch_started.elapsed().as_micros() as u64;
            stats.pull_index |= indexed_used;
            stats.note_slice_tags(filter);
            stats.gap_fetched += envelopes.len();
            stats.count_fetched(&envelopes);
            // The fetch is part of the gap's budget.
            let page_budget = budget.saturating_sub(peer_started.elapsed());
            let apply_started = std::time::Instant::now();
            let page = apply_page(
                conn,
                group_key,
                store,
                peer,
                &envelopes,
                page_budget,
                filter,
                stats,
            )
            .await?;
            stats.gap_apply_us += apply_started.elapsed().as_micros() as u64;
            stats.refetch += envelopes.len() - page.handled;
            let floor = if windowed { window_lo } else { gap_lo };
            if page.handled == envelopes.len() && (envelopes.len() as i64) < rows {
                if floor == gap_lo {
                    store.close_sync_gap(peer).map_err(query_err)?;
                    break;
                }
                // Every row of the window is read: what is left is at or below its floor.
                gap_hi = floor + 1;
                store.set_sync_gap_hi(peer, gap_hi).map_err(query_err)?;
                let left = budget.saturating_sub(peer_started.elapsed());
                if left.is_zero() {
                    break;
                }
                rows = gap_page_rows(pace, left);
                continue;
            } else if let Some(last) = page.handled.checked_sub(1) {
                store
                    .set_sync_gap_hi(peer, envelopes[last].local_seq)
                    .map_err(query_err)?;
            }
            break;
        }
        let gap = store.sync_gap(peer).map_err(query_err)?;
        stats.gap_lo = gap.map(|(lo, _)| lo);
        stats.gap_hi = gap.map(|(_, hi)| hi);
    }
    Ok(())
}

/// A store error on the pull path.
fn query_err(e: impl std::fmt::Display) -> SyncEngineError {
    SyncEngineError::Query(e.to_string())
}

/// What [`apply_page`] did with a page of one peer's rows.
struct PageOutcome {
    /// How many of the page's rows, from its front, were handled (applied, skipped, or
    /// deferred to the dependency pass).
    handled: usize,
    /// A budget stopped it early: the rest of the page waits for a later pass.
    stopped: bool,
}

/// Runs `fut` with `store`'s embedding deferred as `defer`, then puts the previous setting back,
/// however it ends: also when the future is dropped at an `.await` (the round's timeout cancels
/// it, for example during a pause in [`apply_page`]). Left on, the connection would store every
/// later row without an embedding, and the reindex sweep would not be revived to add them.
async fn with_defer_embedding<T>(
    store: &Store,
    defer: bool,
    fut: impl std::future::Future<Output = T>,
) -> T {
    struct Restore<'a> {
        store: &'a Store,
        was: bool,
    }
    impl Drop for Restore<'_> {
        fn drop(&mut self) {
            self.store.set_defer_embedding(self.was);
        }
    }
    let _restore = Restore {
        store,
        was: store.set_defer_embedding(defer),
    };
    fut.await
}

/// Applies a page of one peer's rows in page order (ascending for the live range, descending
/// for a gap page), then resolves the transcript rows it deferred. Stops once `budget` is
/// spent, having handled at least one row.
#[allow(clippy::too_many_arguments)]
async fn apply_page(
    conn: &Connection,
    group_key: &GroupKey,
    store: &Store,
    peer: &str,
    envelopes: &[RowEnvelope],
    budget: std::time::Duration,
    filter: &RowFilter,
    stats: &mut PeerPullStats,
) -> Result<PageOutcome, SyncEngineError> {
    let started = std::time::Instant::now();
    let mut chunk_bytes = 0usize;
    let mut handled = 0usize;
    let mut stopped = false;
    // Two kinds of "needs the parent meeting resolved first" rows now exist:
    // legacy transcript_segment envelopes, and items-format envelopes whose
    // decrypted kind is "transcript". Both go into `segments`, already decoded
    // and already dedupe-checked against sync_row_map -- the second loop below
    // only needs to attempt FK resolution and apply. An items envelope's `kind`
    // can't be known without decrypting it, so unlike the previous version both
    // envelope shapes are now decrypted here rather than lazily in the second pass.
    let mut segments: Vec<(RowEnvelope, PendingSegment)> = Vec::new();
    // Rows apply one autocommit write each, back to back (at row speed when embedding is
    // deferred): pause now and then, with no transaction open, so another connection's write (a
    // recording start above all) is not starved (#850 review, I2). The pauses count against the
    // budget. Not one transaction per chunk: some apply paths open their own transaction
    // (`apply_synced_topic_seed`), which fails inside an outer one.
    let mut yielder = memory::WriteYield::new();
    for env in envelopes {
        // The pace's time budget: stop once it's spent, having handled at least one row.
        if handled > 0 && started.elapsed() >= budget {
            stopped = true;
            stats.budget_stopped = true;
            break;
        }
        if yielder.due() {
            tokio::time::sleep(memory::RETENTION_BATCH_PAUSE).await;
            yielder.paused();
        }
        if env.row_type == ROW_TYPE_SKIN_IMAGE_CHUNK {
            // The image budget: the rest of a large image follows next round (the skin stays held).
            if handled > 0 && chunk_bytes >= MAX_SKIN_CHUNK_BYTES_PER_PASS {
                stopped = true;
                break;
            }
            chunk_bytes += env.ciphertext_base64.len();
        }
        if env.row_type == ROW_TYPE_SKIN {
            tracing::info!(peer = %peer_tag(peer), local_seq = env.local_seq, bytes = env.ciphertext_base64.len(), "fetched skin row");
        }
        handled += 1;
        if env.row_type == ROW_TYPE_TRANSCRIPT_SEGMENT {
            if store
                .resolve_sync_row_map(peer, env.local_seq, &env.row_type)
                .map_err(|e| SyncEngineError::Query(e.to_string()))?
                .is_some()
            {
                stats.already_applied += 1;
                continue; // already applied
            }
            let Some((decoded, origin_epoch)) = decrypt_row(group_key, peer, env).and_then(|p| {
                serde_json::from_slice::<TranscriptSegmentPayload>(&p)
                    .ok()
                    .map(|d| (d, origin_epoch_of(&p)))
            }) else {
                stats.undecodable += 1;
                continue;
            };
            stats.deferred += 1;
            segments.push((
                env.clone(),
                PendingSegment::Legacy {
                    source_meeting_id: decoded.source_meeting_id,
                    meeting_local_seq: decoded.meeting_local_seq,
                    origin_epoch,
                    segment: decoded.segment,
                },
            ));
            continue;
        }
        if env.row_type == ROW_TYPE_ITEMS {
            if store
                .resolve_sync_row_map(peer, env.local_seq, &env.row_type)
                .map_err(|e| SyncEngineError::Query(e.to_string()))?
                .is_some()
            {
                stats.already_applied += 1;
                continue; // already applied in a prior cycle
            }
            let Some((decoded, origin_epoch)) = decrypt_row(group_key, peer, env).and_then(|p| {
                serde_json::from_slice::<ItemPayload>(&p)
                    .ok()
                    .map(|d| (d, origin_epoch_of(&p)))
            }) else {
                stats.undecodable += 1;
                continue;
            };
            // Added items stay on the computer that received them (bridge spec 8.2): handled,
            // never applied, like a kind the allowlist does not grant.
            if decoded.item.kind == memory::MCP_ITEM_KIND {
                stats.skipped_allowlist += 1;
                stats.skip_mcp += 1;
                continue;
            }
            // Not granted (the runner's allowlist names item kinds): handled, never applied.
            let kind_out = !filter.kind_allowed(&decoded.item.kind);
            if kind_out
                || filter.drops_calendar_source(&decoded.item.kind, decoded.item.source.as_deref())
            {
                stats.skipped_allowlist += 1;
                if kind_out {
                    stats.skip_kind += 1;
                } else {
                    stats.skip_google_cal += 1;
                }
                continue;
            }
            if filter.outside_item_window(&decoded.item.kind, decoded.item.ts_unix_ms) {
                stats.skipped_window += 1;
                stats.skip_item_window += 1;
                continue;
            }
            if decoded.item.kind == "transcript" {
                stats.deferred += 1;
                // Never apply here: apply_synced_item would stamp a NULL thread_id,
                // permanently orphaning the row. Defer to the resolution pass below.
                segments.push((
                    env.clone(),
                    PendingSegment::Item {
                        source_meeting_id: decoded.source_meeting_id,
                        meeting_local_seq: decoded.meeting_local_seq,
                        origin_epoch,
                        item: decoded.item,
                    },
                ));
            } else if forgotten_item(store, peer, env.local_seq, &decoded.item)? {
                // A client delete forgot it (here or on a peer): dropped, counted as handled like a
                // forgotten meeting's late row.
                stats.count_applied(&env.row_type);
            } else {
                let local_id = store
                    .apply_synced_item(peer, env.local_seq, None, &decoded.item)
                    .map_err(|e| SyncEngineError::Query(e.to_string()))?;
                store
                    .record_sync_row_map(peer, env.local_seq, &env.row_type, local_id)
                    .map_err(|e| SyncEngineError::Query(e.to_string()))?;
                stats.count_applied(&env.row_type);
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
                .resolve_sync_row_map(peer, env.local_seq, &env.row_type)
                .map_err(|e| SyncEngineError::Query(e.to_string()))?
                .is_some()
        {
            stats.already_applied += 1;
            continue; // already applied in a prior cycle
        }
        let Some(plaintext) = decrypt_row(group_key, peer, env) else {
            stats.undecodable += 1;
            tracing::warn!(peer = %peer_tag(peer), local_seq = env.local_seq, row_type = %env.row_type, "pulled row couldn't be decrypted");
            continue;
        };
        // The runner: a meeting it was not granted, or one started before its window, is not applied. It is
        // remembered by key, so a granted in-window transcript line still fetches it ([`resolve_segments`]).
        if env.row_type == ROW_TYPE_MEETING && filter.holds_meetings() {
            if let Ok(payload) = serde_json::from_slice::<MeetingPayload>(&plaintext) {
                if filter.meeting_held(payload.meeting.start_unix_ms) {
                    if let Some(source_id) = payload.source_meeting_id {
                        remember_parent_meeting(store, peer, source_id, env.local_seq)?;
                    }
                    if filter.meetings_as_parents_only {
                        stats.skipped_allowlist += 1;
                    } else {
                        stats.skipped_window += 1;
                    }
                    continue;
                }
            }
        }
        if env.row_type == ROW_TYPE_CAPTURE_ITEM
            && filter.windows_captures()
            && serde_json::from_slice::<memory::AppliedCaptureItem>(&plaintext)
                .is_ok_and(|item| filter.outside_item_window("capture", item.ts_unix_ms))
        {
            stats.skipped_window += 1;
            continue;
        }
        apply_parent_row(store, peer, env.local_seq, &env.row_type, &plaintext)?;
        stats.count_applied(&env.row_type);
    }
    // A legacy line the window dropped was counted as deferred when it was set aside: take it back.
    let dropped = resolve_segments(conn, group_key, store, peer, segments, filter, stats).await?;
    stats.deferred = stats.deferred.saturating_sub(dropped);
    Ok(PageOutcome { handled, stopped })
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
        origin_epoch: Option<String>,
        segment: memory::AppliedTranscriptSegment,
    },
    Item {
        source_meeting_id: Option<i64>,
        meeting_local_seq: Option<i64>,
        origin_epoch: Option<String>,
        item: memory::AppliedItem,
    },
}

/// The recording database's epoch a child row carries (`origin_epoch`, added by its sender), so a forget
/// names exactly that database's meeting. Rows from older builds have none.
#[derive(serde::Deserialize)]
struct OriginEpoch {
    #[serde(default)]
    origin_epoch: Option<String>,
}

fn origin_epoch_of(plaintext: &[u8]) -> Option<String> {
    serde_json::from_slice::<OriginEpoch>(plaintext)
        .ok()
        .and_then(|e| e.origin_epoch)
        .filter(|e| !e.is_empty())
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
                origin_epoch: origin_epoch_of(&plaintext),
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
                origin_epoch: origin_epoch_of(&plaintext),
                item: decoded.item,
            })
        }
        _ => None,
    }
}

/// kv: the `local_seq` of the newest meeting row of `peer`'s meeting `source_id` that a runner pull held back
/// (ids only, never content).
fn held_meeting_key(peer: &str, source_id: i64) -> String {
    format!("sync_held_meeting:{peer}:{source_id}")
}

/// Remembers a held-back meeting row by key, keeping the newest (highest `local_seq`) seen.
fn remember_parent_meeting(
    store: &Store,
    peer: &str,
    source_id: i64,
    local_seq: i64,
) -> Result<(), SyncEngineError> {
    if remembered_parent_meeting(store, peer, source_id)?.is_some_and(|seen| seen >= local_seq) {
        return Ok(());
    }
    store
        .set_meta(&held_meeting_key(peer, source_id), &local_seq.to_string())
        .map_err(query_err)
}

fn remembered_parent_meeting(
    store: &Store,
    peer: &str,
    source_id: i64,
) -> Result<Option<i64>, SyncEngineError> {
    Ok(store
        .get_meta(&held_meeting_key(peer, source_id))
        .map_err(query_err)?
        .and_then(|seq| seq.parse().ok()))
}

/// Resolves each segment's parent meeting and applies it, or records it in
/// `sync_pending_dependency` for a later round. A meeting that isn't applied locally but
/// whose exact `meeting_local_seq` is known is fetched by key first (batched): it may never
/// have been in a fetch window at all, e.g. the cursor advanced past its local_seq while
/// the meeting was still in flight to the relay.
///
/// With an item window (the runner), a legacy line has no timestamp of its own: it is dated by its
/// meeting's start plus `start_cs * 10` ms, and one before the window is dropped for good (counted in
/// `skipped_window`, never pending). A fetched meeting is applied only as the parent of a line that is
/// applied. Lines applied here count in `stats` under their own row type. Returns how many legacy
/// lines the window dropped.
async fn resolve_segments(
    conn: &Connection,
    group_key: &GroupKey,
    store: &Store,
    peer: &str,
    segments: Vec<(RowEnvelope, PendingSegment)>,
    filter: &RowFilter,
    stats: &mut PeerPullStats,
) -> Result<usize, SyncEngineError> {
    let cutoff = filter.item_cutoff;
    // (local_seq, source_meeting_id, start_unix_ms) of each meeting fetched by key, windowed pulls only.
    let mut fetched_starts: Vec<(i64, Option<i64>, i64)> = Vec::new();
    let mut dropped = 0usize;
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
    let resolve = |source_meeting_id: Option<i64>,
                   fallback_local_seq: Option<i64>|
     -> Result<Option<i64>, SyncEngineError> {
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

    // Lines of a forgotten meeting: dropped before any fetch of their meeting, and never left pending.
    let mut forgotten = Vec::with_capacity(segments.len());
    for (_, pending) in &segments {
        let epoch = match pending {
            PendingSegment::Legacy { origin_epoch, .. }
            | PendingSegment::Item { origin_epoch, .. } => origin_epoch.as_deref(),
        };
        forgotten.push(forgotten_child(store, peer, epoch, fallback(pending).0)?);
    }

    let mut missing_meetings: Vec<(i64, String)> = Vec::new();
    for ((_, pending), forgotten) in segments.iter().zip(&forgotten) {
        let (source_meeting_id, fallback_local_seq) = fallback(pending);
        if *forgotten {
            continue;
        }
        if resolve(source_meeting_id, fallback_local_seq)?.is_some() {
            continue;
        }
        // The legacy seq the line carries, and a meeting row the runner held back for this line's meeting.
        let held = match source_meeting_id {
            Some(source_id) => remembered_parent_meeting(store, peer, source_id)?,
            None => None,
        };
        for seq in [fallback_local_seq, held].into_iter().flatten() {
            if !missing_meetings.iter().any(|(s, _)| *s == seq) {
                missing_meetings.push((seq, ROW_TYPE_MEETING.to_string()));
            }
        }
    }
    // Every copy of a meeting fetched here, decrypted first so a windowed pull dates a line by the same
    // copy (the newest, highest `local_seq`) whether it decides to apply the meeting or to drop the line.
    let mut fetched: Vec<(RowEnvelope, Vec<u8>)> = Vec::new();
    for chunk in missing_meetings.chunks(PENDING_LOOKUP_CHUNK) {
        for meeting_env in fetch_rows_by_key(conn, peer, chunk).await? {
            if let Some(plaintext) = decrypt_row(group_key, peer, &meeting_env) {
                if cutoff.is_some() {
                    if let Ok(payload) = serde_json::from_slice::<MeetingPayload>(&plaintext) {
                        fetched_starts.push((
                            meeting_env.local_seq,
                            payload.source_meeting_id,
                            payload.meeting.start_unix_ms,
                        ));
                    }
                }
                fetched.push((meeting_env, plaintext));
            }
        }
    }
    // The start of the newest fetched copy of the meeting a line names.
    let fetched_start = |source_id: Option<i64>, seq: Option<i64>| {
        fetched_starts
            .iter()
            .filter(|(s, src, _)| seq == Some(*s) || (source_id.is_some() && *src == source_id))
            .max_by_key(|(s, _, _)| *s)
            .map(|(_, _, start)| *start)
    };
    for (meeting_env, plaintext) in &fetched {
        if let Some(cutoff) = cutoff {
            if let Ok(payload) = serde_json::from_slice::<MeetingPayload>(plaintext) {
                // Only the parent of a line that gets applied: an item line is already in the window.
                let needed = segments
                    .iter()
                    .zip(&forgotten)
                    .any(|((_, pending), forgotten)| {
                        let (source_id, seq) = fallback(pending);
                        let names_it = (source_id.is_some()
                            && source_id == payload.source_meeting_id)
                            || seq == Some(meeting_env.local_seq);
                        !*forgotten
                            && names_it
                            && match pending {
                                PendingSegment::Item { .. } => true,
                                PendingSegment::Legacy { segment, .. } => {
                                    let start = fetched_start(source_id, seq)
                                        .unwrap_or(payload.meeting.start_unix_ms);
                                    segment.start_cs * 10 + start >= cutoff
                                }
                            }
                    });
                if !needed {
                    continue;
                }
            }
        }
        apply_parent_row(
            store,
            peer,
            meeting_env.local_seq,
            &meeting_env.row_type,
            plaintext,
        )?;
    }

    // The transcript lines apply one write each, back to back: pause now and then, as the
    // page's own loop does (`apply_page`; #850 review, I2).
    let mut yielder = memory::WriteYield::new();
    for ((env, pending), forgotten) in segments.iter().zip(forgotten) {
        if yielder.due() {
            tokio::time::sleep(memory::RETENTION_BATCH_PAUSE).await;
            yielder.paused();
        }
        let (source_meeting_id, fallback_local_seq) = fallback(pending);
        if forgotten {
            store
                .delete_sync_pending_dependency(peer, env.local_seq, &env.row_type)
                .map_err(|e| SyncEngineError::Query(e.to_string()))?;
            continue;
        }
        // A legacy line before the window: dropped for good. A meeting nobody has (yet) leaves it pending.
        if let (Some(cutoff), PendingSegment::Legacy { segment, .. }) = (cutoff, pending) {
            let start = match fetched_start(source_meeting_id, fallback_local_seq) {
                Some(start) => Some(start),
                None => match resolve(source_meeting_id, fallback_local_seq)? {
                    Some(id) => store
                        .get_meeting(id)
                        .map_err(query_err)?
                        .map(|m| m.start_unix_ms),
                    None => None,
                },
            };
            if start.is_some_and(|start| start + segment.start_cs * 10 < cutoff) {
                stats.skipped_window += 1;
                dropped += 1;
                store
                    .delete_sync_pending_dependency(peer, env.local_seq, &env.row_type)
                    .map_err(|e| SyncEngineError::Query(e.to_string()))?;
                continue;
            }
        }
        // No forgotten check once resolved: a tombstone deletes the meeting and its map entry, so a
        // meeting that still resolves is alive, and an epoch-less check here could only over-drop.
        let resolved_meeting_id = resolve(source_meeting_id, fallback_local_seq)?;
        match resolved_meeting_id {
            Some(local_meeting_id) => {
                // A line past the meeting's trim was pushed before the owner trimmed it
                // (no tombstones): drop it. Untrimmed meetings keep every late line.
                let trim = store
                    .meeting_trim(local_meeting_id)
                    .map_err(|e| SyncEngineError::Query(e.to_string()))?;
                let past_trim = trim.is_some_and(|(start, keep)| match pending {
                    PendingSegment::Legacy { segment, .. } => segment.start_cs * 10 >= keep,
                    PendingSegment::Item { item, .. } => item.ts_unix_ms - start >= keep,
                });
                if past_trim {
                    store
                        .delete_sync_pending_dependency(peer, env.local_seq, &env.row_type)
                        .map_err(|e| SyncEngineError::Query(e.to_string()))?;
                    continue;
                }
                let local_id = match pending {
                    PendingSegment::Legacy { segment, .. } => store
                        .apply_synced_transcript_segment(env.local_seq, local_meeting_id, segment)
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
                stats.count_applied(&env.row_type);
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
    Ok(dropped)
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
            .query(
                "SELECT COUNT(*) FROM sync_row WHERE device_id = 'device-a' AND local_seq != 0",
                (),
            )
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
            push_content(&conn, &group_key, &store, "device-a")
                .await
                .unwrap();
            expected = (expected + MAX_ROWS_PER_PUSH_PASS).min(backlog);
            let (count, max_seq) = count_pushed(&conn, "device-a").await;
            assert_eq!(
                count as usize, expected,
                "pass {pass} sends at most one bounded pass"
            );
            assert_eq!(
                store.get_sync_push_cursor().unwrap(),
                max_seq,
                "pass {pass}: the cursor is the highest row sent"
            );
        }
        let cursor = store.get_sync_push_cursor().unwrap();
        push_content(&conn, &group_key, &store, "device-a")
            .await
            .unwrap();
        assert_eq!(
            count_pushed(&conn, "device-a").await.0 as usize,
            backlog,
            "nothing left to send"
        );
        assert_eq!(store.get_sync_push_cursor().unwrap(), cursor);

        // A peer pulls every row (the pull side caps its own passes too) and decrypts them.
        let store_b = Store::open_in_memory().unwrap();
        for _ in 0..(backlog / 500 + 2) {
            pull_content(&conn, &group_key, &store_b, "device-b")
                .await
                .unwrap();
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
            .query(
                "SELECT COUNT(*) FROM sync_row WHERE device_id = 'device-a' AND local_seq != 0",
                (),
            )
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
            .query("SELECT COUNT(*) FROM sync_row WHERE local_seq != 0", ())
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
            seed_content_row(
                conn,
                &GroupKey::from_bytes([7; 32]),
                device_id,
                seq,
                "items",
                "{}",
            )
            .await;
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
        assert!(distinct_peer_device_ids(&conn, "dev-a")
            .await
            .unwrap()
            .contains(&"dev-own".to_string()));
    }

    /// Inside a round the peers and their markers are read once and answer every gate the same way; outside
    /// a round (and after `end`) every call reads the relay, as before.
    #[tokio::test]
    async fn a_round_reads_the_peers_and_markers_once_and_answers_every_gate_from_them() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("remote.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        for device in ["dev-own", "dev-a", "dev-b"] {
            seed_rows(&conn, device, 2).await;
        }
        seed_peer_capability_marker(&conn, "dev-a").await;

        round_peers::begin("dev-own");
        assert!(
            !all_peers_items_capable(&conn, "dev-own").await.unwrap(),
            "dev-b has no marker"
        );
        assert_eq!(
            peers_without_marker(&conn, "dev-own", ROW_TYPE_ITEMS_CAPABILITY)
                .await
                .unwrap(),
            ["dev-b"]
        );
        // Changes on the relay during the round are seen next round, not this one.
        seed_peer_capability_marker(&conn, "dev-b").await;
        seed_rows(&conn, "dev-c", 1).await;
        assert!(!all_peers_items_capable(&conn, "dev-own").await.unwrap());
        assert_eq!(
            distinct_peer_device_ids(&conn, "dev-own").await.unwrap(),
            ["dev-a", "dev-b"]
        );
        // Forgetting the markers (a marker this round published) reads them again.
        round_peers::forget_markers();
        assert!(all_peers_items_capable(&conn, "dev-own").await.unwrap());
        round_peers::end();

        // Outside a round: the relay as it is now.
        assert_eq!(
            distinct_peer_device_ids(&conn, "dev-own").await.unwrap(),
            ["dev-a", "dev-b", "dev-c"]
        );
        assert!(
            !all_peers_items_capable(&conn, "dev-own").await.unwrap(),
            "dev-c has no marker"
        );
    }

    #[tokio::test]
    async fn all_peers_items_capable_needs_a_marker_from_every_peer() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("remote.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        seed_rows(&conn, "dev-own", 2).await;
        assert!(
            all_peers_items_capable(&conn, "dev-own").await.unwrap(),
            "no peers is vacuously true"
        );

        seed_rows(&conn, "dev-a", 2).await;
        assert!(!all_peers_items_capable(&conn, "dev-own").await.unwrap());
        seed_peer_capability_marker(&conn, "dev-a").await;
        assert!(all_peers_items_capable(&conn, "dev-own").await.unwrap());

        seed_rows(&conn, "dev-b", 2).await;
        assert!(
            !all_peers_items_capable(&conn, "dev-own").await.unwrap(),
            "a second peer without a marker"
        );
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
            let mut rows = conn
                .query(&format!("EXPLAIN QUERY PLAN {sql}"), args)
                .await
                .unwrap();
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
            attendee_details: vec![],
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
            trimmed_keep_ms: None,
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
            origin_epoch: None,
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
        seed_content_row(
            &conn,
            &group_key,
            "device-a",
            1,
            ROW_TYPE_ROUTINE_RUN,
            &first,
        )
        .await;
        let updated = first.replace("first brief", "revised brief");
        seed_content_row(
            &conn,
            &group_key,
            "device-a",
            2,
            ROW_TYPE_ROUTINE_RUN,
            &updated,
        )
        .await;
        let unknown = serde_json::json!({
            "source_run_id": 5, "template": "from_the_future", "ran_at_unix_ms": 1,
            "status": "success", "output_summary": "x", "output_json": "{}"
        })
        .to_string();
        seed_content_row(
            &conn,
            &group_key,
            "device-a",
            3,
            ROW_TYPE_ROUTINE_RUN,
            &unknown,
        )
        .await;

        let store = Store::open_in_memory().unwrap();
        pull_content(&conn, &group_key, &store, "device-b")
            .await
            .unwrap();

        let runs = store.recent_routine_runs_across_all(10).unwrap();
        assert_eq!(
            runs.len(),
            1,
            "the same origin run twice is one run; an unknown template is ignored"
        );
        assert_eq!(runs[0].output_summary, "revised brief");
        assert_eq!(runs[0].origin_device_id.as_deref(), Some("device-a"));
        assert_eq!(
            store.get_sync_cursor("device-a").unwrap(),
            3,
            "the cursor still advances past an ignored row"
        );
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
        seed_content_row(
            &conn,
            &group_key,
            "device-a",
            1,
            ROW_TYPE_ROUTINE_RUN,
            &future_status,
        )
        .await;
        seed_content_row(
            &conn,
            &group_key,
            "device-a",
            2,
            ROW_TYPE_ROUTINE_RUN,
            "not json at all",
        )
        .await;
        let good = serde_json::to_string(&memory::AppliedRoutineRun {
            source_run_id: 3,
            template: "daily_brief".into(),
            ran_at_unix_ms: 5,
            status: memory::RunStatus::Success,
            output_summary: "good".into(),
            output_json: "{}".into(),
        })
        .unwrap();
        seed_content_row(
            &conn,
            &group_key,
            "device-a",
            3,
            ROW_TYPE_ROUTINE_RUN,
            &good,
        )
        .await;

        let store = Store::open_in_memory().unwrap();
        pull_content(&conn, &group_key, &store, "device-b")
            .await
            .expect("a bad routine_run row must not fail the pull");
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
        seed_content_row(
            &conn,
            &group_key,
            "device-a",
            1,
            ROW_TYPE_MEETING_PREP,
            "not json at all",
        )
        .await;
        seed_content_row(
            &conn,
            &group_key,
            "device-a",
            2,
            ROW_TYPE_MEETING_PREP,
            &good,
        )
        .await;

        let store = Store::open_in_memory().unwrap();
        pull_content(&conn, &group_key, &store, "device-b")
            .await
            .expect("a bad meeting_prep row must not fail the pull");
        assert_eq!(
            store
                .get_meeting_prep("ev1")
                .unwrap()
                .unwrap()
                .generated_at_unix_ms,
            5
        );
        assert_eq!(store.get_sync_cursor("device-a").unwrap(), 2);
    }

    #[tokio::test]
    async fn pull_content_applies_a_peer_profile_suggestion_and_skips_a_bad_one() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("shared.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        let group_key = GroupKey::generate();
        let good = serde_json::json!({"source_id": 4, "text": "Prefers mornings", "created_at_unix_ms": 5}).to_string();
        seed_content_row(
            &conn,
            &group_key,
            "device-a",
            1,
            ROW_TYPE_PROFILE_SUGGESTION,
            "not json at all",
        )
        .await;
        seed_content_row(
            &conn,
            &group_key,
            "device-a",
            2,
            ROW_TYPE_PROFILE_SUGGESTION,
            &good,
        )
        .await;

        let store = Store::open_in_memory().unwrap();
        pull_content(&conn, &group_key, &store, "device-b")
            .await
            .expect("a bad profile_suggestion row must not fail the pull");
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
            captures_with_text(&store_b, &format!("peer captured text {total_rows}")),
            0
        );

        pull_content(&conn, &group_key, &store_b, "device-b")
            .await
            .unwrap();

        // Second call resumes from the persisted cursor and finishes the backlog.
        assert_eq!(store_b.get_sync_cursor("device-a").unwrap(), total_rows);
        assert_eq!(
            captures_with_text(&store_b, &format!("peer captured text {total_rows}")),
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
            captures_with_text(&store_b, &format!("backlog text {backlog_end}")),
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
            store_b
                .list_sync_pending_dependencies("device-a")
                .unwrap()
                .is_empty(),
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
        let found = |text: &str| captures_with_text(&store_b, text);
        let zero = std::time::Duration::ZERO;

        let more = pull_content_with_budget(
            &conn,
            &group_key,
            &store_b,
            "device-b",
            budget_only(zero),
            &PullOptions::default(),
            false,
        )
        .await
        .unwrap();
        assert!(more, "a pass cut short says rows are waiting");
        assert_eq!(
            store_b.get_sync_cursor("device-a").unwrap(),
            1,
            "the cursor stops at the last row handled"
        );
        assert_eq!(
            store_b.get_sync_cursor("device-c").unwrap(),
            1,
            "the other peer progresses in the same call"
        );
        assert_eq!((found("alpha row 1"), found("alpha row 2")), (1, 0));

        for _ in 0..2 {
            pull_content_with_budget(
                &conn,
                &group_key,
                &store_b,
                "device-b",
                budget_only(zero),
                &PullOptions::default(),
                false,
            )
            .await
            .unwrap();
        }
        assert_eq!(store_b.get_sync_cursor("device-a").unwrap(), 3);
        assert_eq!(store_b.get_sync_cursor("device-c").unwrap(), 2);
        let more = pull_content_with_budget(
            &conn,
            &group_key,
            &store_b,
            "device-b",
            budget_only(zero),
            &PullOptions::default(),
            false,
        )
        .await
        .unwrap();
        assert!(!more, "a caught-up device has nothing waiting");
        for text in ["alpha row 2", "alpha row 3", "gamma row 1", "gamma row 2"] {
            assert_eq!(found(text), 1, "{text} applied exactly once");
        }
    }

    async fn seed_peer_captures(
        conn: &Connection,
        group_key: &GroupKey,
        device: &str,
        word: &str,
        rows: i64,
    ) {
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
                conn,
                group_key,
                device,
                local_seq,
                ROW_TYPE_CAPTURE_ITEM,
                &capture_json,
            )
            .await;
        }
    }

    /// #572: a pull-only round after one that left rows behind takes the bigger catch-up
    /// page; a caught-up one (and every desktop pull) keeps the normal page.
    #[tokio::test]
    async fn a_pull_only_round_catches_up_with_bigger_pages_after_more_to_pull() {
        assert_eq!(pull_only_pace(false), NORMAL_PACE);
        assert_eq!(pull_only_pace(true), CATCH_UP_PACE);
        assert!(
            CATCH_UP_PACE.rows > NORMAL_PACE.rows
                && CATCH_UP_PACE.peer_budget > NORMAL_PACE.peer_budget
        );
        assert!(
            CATCH_UP_PACE.round_budget.unwrap() + PULL_PEER_TIME_BUDGET * 2 < crate::SYNC_TIMEOUT
        );

        let dir = tempfile::tempdir().unwrap();
        let group_key = GroupKey::generate();
        let conn = local_connection(&dir.path().join("shared.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        let total = CATCH_UP_PACE.rows + NORMAL_PACE.rows + 3;
        seed_peer_captures(&conn, &group_key, "device-a", "alpha", total).await;
        let store_b = Store::open_in_memory().unwrap();
        let options = PullOptions::default();

        let more = pull_content_with_budget(
            &conn,
            &group_key,
            &store_b,
            "device-b",
            pull_only_pace(false),
            &options,
            true,
        )
        .await
        .unwrap();
        assert!(more);
        assert_eq!(
            store_b.get_sync_cursor("device-a").unwrap(),
            NORMAL_PACE.rows
        );
        let more = pull_content_with_budget(
            &conn,
            &group_key,
            &store_b,
            "device-b",
            pull_only_pace(more),
            &options,
            true,
        )
        .await
        .unwrap();
        assert!(more, "a full catch-up page still says rows are waiting");
        assert_eq!(
            store_b.get_sync_cursor("device-a").unwrap(),
            NORMAL_PACE.rows + CATCH_UP_PACE.rows
        );
        let more = pull_content_with_budget(
            &conn,
            &group_key,
            &store_b,
            "device-b",
            pull_only_pace(more),
            &options,
            true,
        )
        .await
        .unwrap();
        assert!(!more);
        assert_eq!(store_b.get_sync_cursor("device-a").unwrap(), total);
        assert_eq!(
            captures_with_text(&store_b, &format!("alpha row {total}")),
            1
        );
    }

    /// A catch-up round whose round budget is spent still moves every peer by a row.
    #[tokio::test]
    async fn a_spent_catch_up_round_still_progresses_every_peer() {
        let dir = tempfile::tempdir().unwrap();
        let group_key = GroupKey::generate();
        let conn = local_connection(&dir.path().join("shared.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        seed_peer_captures(&conn, &group_key, "device-a", "alpha", 3).await;
        seed_peer_captures(&conn, &group_key, "device-c", "gamma", 3).await;
        let store_b = Store::open_in_memory().unwrap();
        let spent = Pace {
            round_budget: Some(std::time::Duration::ZERO),
            ..CATCH_UP_PACE
        };
        let more = pull_content_with_budget(
            &conn,
            &group_key,
            &store_b,
            "device-b",
            spent,
            &PullOptions::default(),
            true,
        )
        .await
        .unwrap();
        assert!(more);
        assert_eq!(store_b.get_sync_cursor("device-a").unwrap(), 1);
        assert_eq!(store_b.get_sync_cursor("device-c").unwrap(), 1);
    }

    /// Seeds `count` transcript segments (local_seq 1..=count) whose meeting
    /// (meeting_local_seq 999) isn't on the relay, and pulls once so they all go pending.
    async fn seed_pending_segments(
        conn: &Connection,
        group_key: &GroupKey,
        store: &Store,
        count: i64,
    ) {
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
            seed_content_row(
                conn,
                group_key,
                "device-a",
                local_seq,
                ROW_TYPE_TRANSCRIPT_SEGMENT,
                &payload,
            )
            .await;
        }
        pull_content(conn, group_key, store, "device-b")
            .await
            .unwrap();
        assert_eq!(
            store
                .list_sync_pending_dependencies("device-a")
                .unwrap()
                .len() as i64,
            count
        );
    }

    fn take_row_queries() -> usize {
        ROW_QUERIES.with(|c| c.replace(0))
    }

    async fn seed_tagged_row(
        conn: &Connection,
        device_id: &str,
        local_seq: i64,
        row_type: &str,
        tag: &str,
    ) {
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

        store
            .insert_capture_item(&sample_new_capture_item("first capture"))
            .unwrap();
        // A distinct hash, or the per-app dedup drops it.
        store
            .insert_capture_item(&memory::NewCaptureItem {
                hash: 10,
                ..sample_new_capture_item("second capture")
            })
            .unwrap();
        let mut event = memory::NewCalendarEventItem {
            device_id: "dev-a".to_string(),
            source: "google_calendar".to_string(),
            external_id: "evt-1".to_string(),
            ts_unix_ms: 5000,
            title: "Planning".to_string(),
            actors: vec![],
        };
        store
            .upsert_calendar_event_items(std::slice::from_ref(&event))
            .unwrap();
        push_content(&conn, &key, &store, "dev-a").await.unwrap();
        event.title = "Planning (moved)".to_string();
        store
            .upsert_calendar_event_items(std::slice::from_ref(&event))
            .unwrap();
        push_content(&conn, &key, &store, "dev-a").await.unwrap();

        let tagged =
            "SELECT COUNT(*) FROM sync_row WHERE row_type = 'items' AND entity_tag IS NOT NULL";
        let untagged =
            "SELECT COUNT(*) FROM sync_row WHERE row_type = 'items' AND entity_tag IS NULL";
        assert_eq!(single_i64(&conn, tagged).await, 1);
        assert_eq!(single_i64(&conn, untagged).await, 2);
    }

    #[tokio::test]
    async fn a_capture_only_push_issues_no_delete() {
        let dir = tempfile::tempdir().unwrap();
        let conn = compacting_relay(&dir).await;
        let store = Store::open_in_memory().unwrap();
        store
            .insert_capture_item(&sample_new_capture_item("only a capture"))
            .unwrap();
        take_compaction_deletes();

        push_content(&conn, &GroupKey::generate(), &store, "dev-a")
            .await
            .unwrap();

        assert_eq!(
            single_i64(
                &conn,
                "SELECT COUNT(*) FROM sync_row WHERE row_type = 'items'"
            )
            .await,
            1
        );
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

        push_content(&conn, &GroupKey::generate(), &store, "dev-a")
            .await
            .unwrap();

        assert_eq!(
            single_i64(
                &conn,
                "SELECT COUNT(*) FROM sync_row WHERE row_type = 'items'"
            )
            .await,
            120
        );
        assert_eq!(
            FAILED_TAGGED_INSERTS.with(|c| c.get()),
            1,
            "three chunks, one doomed INSERT"
        );
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
        let forged = tag_for(
            &key,
            "device-a",
            &store.sync_entity_epoch().unwrap(),
            &format!("meeting:{id}"),
        );
        // Two versions, so device-b's older row would go if the DELETE lost its device scope.
        seed_tagged_row(&conn, "device-b", 1, "meeting", &forged).await;
        seed_tagged_row(&conn, "device-b", 2, "meeting", &forged).await;
        // Without a marker, device-b would close the items cutover gate.
        seed_peer_capability_marker(&conn, "device-b").await;
        push_content(&conn, &key, &store, "device-a").await.unwrap();
        store.set_meeting_title(id, "Renamed").unwrap();
        push_content(&conn, &key, &store, "device-a").await.unwrap();

        assert_eq!(
            meeting_rows(&conn, "device-a").await,
            1,
            "device-a's own old version is compacted"
        );
        assert_eq!(
            meeting_rows(&conn, "device-b").await,
            2,
            "device-b's rows survive"
        );
        assert_eq!(
            single_i64(
                &conn,
                "SELECT COUNT(DISTINCT entity_tag) FROM sync_row WHERE row_type = 'meeting'"
            )
            .await,
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
        push_content(&conn, &GroupKey::generate(), &store, "device-a")
            .await
            .unwrap();
        store.set_meeting_title(id, "Renamed").unwrap();
        push_content(&conn, &GroupKey::generate(), &store, "device-a")
            .await
            .unwrap();

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
        seed_content_row(
            &conn,
            &GroupKey::from_bytes([7; 32]),
            "device-a",
            3,
            "items",
            "{}",
        )
        .await;

        let tags = ["a".to_string(), "b".to_string(), "unseen".to_string()];
        assert_eq!(compact_entity_tags(&conn, "device-a", &tags).await, 2);
        assert_eq!(
            compact_entity_tags(&conn, "device-a", &tags).await,
            0,
            "idempotent"
        );
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
        let (old_db, new_db) = (
            Store::open_in_memory().unwrap(),
            Store::open_in_memory().unwrap(),
        );
        let tag = |store: &Store| {
            tag_for(
                &key,
                "device-a",
                &store.sync_entity_epoch().unwrap(),
                "meeting:1",
            )
        };
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
        push_content(&conn, &key, &old_db, "device-a")
            .await
            .unwrap();
        let old_seq = single_i64(
            &conn,
            "SELECT local_seq FROM sync_row WHERE row_type = 'meeting'",
        )
        .await;

        let new_db = Store::open_in_memory().unwrap();
        let new_id = completed_meeting(&new_db, "device-a");
        assert_eq!(new_id, old_id, "the wipe reuses the meeting id");
        push_content(&conn, &key, &new_db, "device-a")
            .await
            .unwrap();

        assert_eq!(
            meeting_rows(&conn, "device-a").await,
            2,
            "neither database's meeting is deleted"
        );
        assert!(
            new_db.get_sync_push_cursor().unwrap() < old_seq,
            "the new row sits below the old MAX"
        );
    }

    #[test]
    fn missing_entity_tag_column_is_recognized() {
        assert!(is_missing_entity_tag_column(
            "table sync_row has no column named entity_tag"
        ));
        assert!(!is_missing_entity_tag_column("database is locked"));
        // Mentions the column but is not the missing-column error: must not trigger the retry.
        assert!(!is_missing_entity_tag_column(
            "CHECK constraint failed: entity_tag"
        ));
    }

    async fn has_column(conn: &Connection, column: &str) -> bool {
        let mut rows = conn.query("PRAGMA table_info(sync_row)", ()).await.unwrap();
        while let Some(row) = rows.next().await.unwrap() {
            if row.get::<String>(1).unwrap() == column {
                return true;
            }
        }
        false
    }

    async fn kind_tag_of(conn: &Connection, filter: &str) -> Option<String> {
        let mut rows = conn
            .query(&format!("SELECT kind_tag FROM sync_row WHERE {filter}"), ())
            .await
            .unwrap();
        rows.next()
            .await
            .unwrap()
            .unwrap()
            .get::<Option<String>>(0)
            .unwrap()
    }

    fn push_fixture_store() -> Store {
        let store = Store::open_in_memory().unwrap();
        store
            .insert_capture_item(&sample_new_capture_item("a capture"))
            .unwrap();
        store
            .upsert_calendar_event_items(&[memory::NewCalendarEventItem {
                device_id: "dev-a".to_string(),
                source: "google_calendar".to_string(),
                external_id: "evt-1".to_string(),
                ts_unix_ms: 5000,
                title: "Planning".to_string(),
                actors: vec![],
            }])
            .unwrap();
        completed_meeting(&store, "dev-a");
        store
    }

    #[tokio::test]
    async fn a_fresh_relay_has_the_kind_tag_column() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("relay.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        assert!(has_column(&conn, "kind_tag").await);
        assert!(KIND_TAG_COLUMN_SEEN.get());
    }

    #[tokio::test]
    async fn ensure_kind_tag_column_upgrades_an_entity_tag_relay_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("relay.db")).await;
        conn.execute(OLD_SYNC_ROW_DDL, ()).await.unwrap();
        assert!(ensure_entity_tag_column(&conn).await);
        assert!(!has_column(&conn, "kind_tag").await);
        assert!(ensure_kind_tag_column(&conn).await);
        assert!(has_column(&conn, "kind_tag").await);
        assert!(
            ensure_kind_tag_column(&conn).await,
            "a second call is a no-op"
        );
    }

    #[tokio::test]
    async fn pushed_items_carry_their_kind_tag_and_other_rows_none() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("relay.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        let key = GroupKey::generate();
        let store = push_fixture_store();

        push_content(&conn, &key, &store, "dev-a").await.unwrap();

        let by_kind = |kind: &str| {
            let tag = sync::kind_tag(&key, kind);
            format!("SELECT COUNT(*) FROM sync_row WHERE row_type = 'items' AND kind_tag = '{tag}'")
        };
        assert_eq!(single_i64(&conn, &by_kind("calendar_event")).await, 1);
        assert_eq!(single_i64(&conn, &by_kind("capture")).await, 1);
        assert_eq!(meeting_rows(&conn, "dev-a").await, 1);
        assert_eq!(
            kind_tag_of(&conn, "row_type = 'meeting'").await,
            None,
            "a meeting row stays NULL"
        );
        assert_eq!(
            single_i64(
                &conn,
                "SELECT COUNT(*) FROM sync_row WHERE kind_tag IS NOT NULL AND row_type != 'items'"
            )
            .await,
            0
        );
    }

    #[tokio::test]
    async fn kind_tag_for_tags_only_item_rows() {
        // libsql before rusqlite, as in outbox.rs.
        let dir = tempfile::tempdir().unwrap();
        let _conn = local_connection(&dir.path().join("relay.db")).await;
        let key = GroupKey::generate();
        let store = push_fixture_store();
        let rows = store.unpushed_item_rows("dev-a", 0).unwrap();
        assert!(!rows.is_empty());
        for row in &rows {
            let SyncableRow::Item { item, .. } = row else {
                panic!("item rows only")
            };
            assert_eq!(
                kind_tag_for(&key, row),
                Some(sync::kind_tag(&key, &item.kind))
            );
        }
        let meetings = store.unpushed_meeting_rows("dev-a", 0).unwrap();
        assert!(!meetings.is_empty());
        assert!(meetings.iter().all(|r| kind_tag_for(&key, r).is_none()));
    }

    /// The column is independent of `entity_tag`: a relay with only `kind_tag` is tagged by kind.
    #[tokio::test]
    async fn a_relay_with_only_kind_tag_is_tagged_by_kind() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("relay.db")).await;
        conn.execute(OLD_SYNC_ROW_DDL, ()).await.unwrap();
        conn.execute("ALTER TABLE sync_row ADD COLUMN kind_tag TEXT", ())
            .await
            .unwrap();
        ENTITY_TAG_COLUMN_SEEN.set();
        KIND_TAG_COLUMN_SEEN.set();
        let key = GroupKey::generate();
        let store = push_fixture_store();
        FAILED_TAGGED_INSERTS.with(|c| c.set(0));

        push_content(&conn, &key, &store, "dev-a").await.unwrap();

        assert_eq!(
            kind_tag_of(&conn, "row_type = 'items' AND local_seq = (SELECT MIN(local_seq) FROM sync_row WHERE row_type = 'items')")
                .await
                .map(|t| t.len()),
            Some(24)
        );
        assert_eq!(
            single_i64(
                &conn,
                "SELECT COUNT(*) FROM sync_row WHERE kind_tag IS NOT NULL"
            )
            .await,
            2
        );
        assert_eq!(
            FAILED_TAGGED_INSERTS.with(|c| c.get()),
            1,
            "one entity_tag miss, then kind-only"
        );
    }

    /// Account-switch case: the latch says yes but this relay has no column.
    #[tokio::test]
    async fn a_relay_without_kind_tag_still_takes_the_push_untagged() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("relay.db")).await;
        conn.execute(OLD_SYNC_ROW_DDL, ()).await.unwrap();
        KIND_TAG_COLUMN_SEEN.set();
        let store = push_fixture_store();
        FAILED_TAGGED_INSERTS.with(|c| c.set(0));

        push_content(&conn, &GroupKey::generate(), &store, "dev-a")
            .await
            .unwrap();

        assert_eq!(
            single_i64(
                &conn,
                "SELECT COUNT(*) FROM sync_row WHERE row_type = 'items'"
            )
            .await,
            2
        );
        assert_eq!(FAILED_TAGGED_INSERTS.with(|c| c.get()), 1);
    }

    /// The ALTER could not run (latch never set): untagged, and no doomed INSERT.
    #[tokio::test]
    async fn a_failed_kind_tag_alter_pushes_untagged_without_a_doomed_insert() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("relay.db")).await;
        conn.execute(OLD_SYNC_ROW_DDL, ()).await.unwrap();
        KIND_TAG_COLUMN_SEEN.reset();
        let store = push_fixture_store();
        FAILED_TAGGED_INSERTS.with(|c| c.set(0));

        push_content(&conn, &GroupKey::generate(), &store, "dev-a")
            .await
            .unwrap();

        assert_eq!(
            single_i64(
                &conn,
                "SELECT COUNT(*) FROM sync_row WHERE row_type = 'items'"
            )
            .await,
            2
        );
        assert_eq!(FAILED_TAGGED_INSERTS.with(|c| c.get()), 0);
    }

    #[test]
    fn missing_kind_tag_column_is_recognized() {
        assert!(is_missing_kind_tag_column(
            "table sync_row has no column named kind_tag"
        ));
        assert!(!is_missing_kind_tag_column(
            "table sync_row has no column named entity_tag"
        ));
        assert!(!is_missing_kind_tag_column("database is locked"));
    }

    async fn slice_tag_of(conn: &Connection, filter: &str) -> Option<String> {
        let mut rows = conn
            .query(
                &format!("SELECT slice_tag FROM sync_row WHERE {filter}"),
                (),
            )
            .await
            .unwrap();
        rows.next()
            .await
            .unwrap()
            .unwrap()
            .get::<Option<String>>(0)
            .unwrap()
    }

    #[tokio::test]
    async fn a_fresh_relay_has_the_slice_tag_column() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("relay.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        assert!(has_column(&conn, "slice_tag").await);
        assert!(SLICE_TAG_COLUMN_SEEN.get());
    }

    #[tokio::test]
    async fn ensure_slice_tag_column_upgrades_a_kind_tag_relay_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("relay.db")).await;
        conn.execute(OLD_SYNC_ROW_DDL, ()).await.unwrap();
        assert!(ensure_entity_tag_column(&conn).await);
        assert!(ensure_kind_tag_column(&conn).await);
        assert!(!has_column(&conn, "slice_tag").await);
        assert!(ensure_slice_tag_column(&conn).await);
        assert!(has_column(&conn, "slice_tag").await);
        assert!(
            ensure_slice_tag_column(&conn).await,
            "a second call is a no-op"
        );
        ensure_content_schema(&conn).await.unwrap();
    }

    #[tokio::test]
    async fn pushed_items_carry_their_slice_tag_and_other_rows_none() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("relay.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        let key = GroupKey::generate();
        let store = push_fixture_store();

        push_content(&conn, &key, &store, "dev-a").await.unwrap();

        let calendar = sync::slice_tag(
            &key,
            "calendar_event",
            sync::SourceClass::Google,
            sync::month_bucket(5000),
        );
        let capture = sync::slice_tag(
            &key,
            "capture",
            sync::SourceClass::None,
            sync::month_bucket(1000),
        );
        for tag in [calendar, capture] {
            let sql = format!(
                "SELECT COUNT(*) FROM sync_row WHERE row_type = 'items' AND slice_tag = '{tag}'"
            );
            assert_eq!(single_i64(&conn, &sql).await, 1);
        }
        assert_eq!(slice_tag_of(&conn, "row_type = 'meeting'").await, None);
        assert_eq!(
            single_i64(
                &conn,
                "SELECT COUNT(*) FROM sync_row WHERE slice_tag IS NOT NULL AND row_type != 'items'"
            )
            .await,
            0
        );
    }

    /// The push-time tag and the backfill's payload-derived tag must be the same function of the item.
    #[tokio::test]
    async fn slice_tag_for_equals_the_tag_derived_from_the_encoded_payload() {
        let key = GroupKey::generate();
        let store = Store::open_in_memory().unwrap();
        store
            .insert_capture_item(&sample_new_capture_item("a capture"))
            .unwrap();
        for source in ["google_calendar", "outlook"] {
            store
                .upsert_calendar_event_items(&[memory::NewCalendarEventItem {
                    device_id: "dev-a".to_string(),
                    source: source.to_string(),
                    external_id: format!("evt-{source}"),
                    ts_unix_ms: 1_700_000_000_000,
                    title: "Planning".to_string(),
                    actors: vec![],
                }])
                .unwrap();
        }
        let rows = store.unpushed_item_rows("dev-a", 0).unwrap();
        assert_eq!(rows.len(), 3);
        let mut seen = std::collections::HashSet::new();
        for row in &rows {
            let (_, _, payload_json) = encode_row(row).unwrap();
            let pushed = slice_tag_for(&key, row);
            assert!(pushed.is_some());
            assert_eq!(
                pushed,
                item_payload_slice_tag(&key, payload_json.as_bytes())
            );
            seen.insert(pushed);
        }
        assert_eq!(
            seen.len(),
            3,
            "capture, google and other calendar are three distinct slices"
        );
        let meetings = store.unpushed_meeting_rows("dev-a", 0).unwrap();
        assert!(meetings.iter().all(|r| slice_tag_for(&key, r).is_none()));
    }

    #[test]
    fn a_missing_source_is_google_only_for_calendar_events() {
        let key = GroupKey::generate();
        let op = |payload: &str, row_type: &str| memory::ClaimedOutboxOperation {
            operation_id: "op".to_string(),
            device_id: "device-a".to_string(),
            local_seq: 1,
            row_type: row_type.to_string(),
            aggregate_type: row_type.to_string(),
            aggregate_id: "1".to_string(),
            operation_kind: memory::OutboxOperationKind::Upsert,
            payload_json: payload.to_string(),
            connector: memory::OUTBOX_CONNECTOR.to_string(),
            attempts: 0,
        };
        let month = sync::month_bucket(1_700_000_000_000);
        let cal = op(
            r#"{"kind":"calendar_event","ts_unix_ms":1700000000000}"#,
            ROW_TYPE_ITEMS,
        );
        assert_eq!(
            outbox_slice_tag(&key, &cal),
            Some(sync::slice_tag(
                &key,
                "calendar_event",
                sync::SourceClass::Google,
                month
            ))
        );
        let other = op(
            r#"{"kind":"calendar_event","source":"outlook","ts_unix_ms":1700000000000}"#,
            ROW_TYPE_ITEMS,
        );
        assert_eq!(
            outbox_slice_tag(&key, &other),
            Some(sync::slice_tag(
                &key,
                "calendar_event",
                sync::SourceClass::Other,
                month
            ))
        );
        let capture = op(
            r#"{"kind":"capture","ts_unix_ms":1700000000000}"#,
            ROW_TYPE_ITEMS,
        );
        assert_eq!(
            outbox_slice_tag(&key, &capture),
            Some(sync::slice_tag(
                &key,
                "capture",
                sync::SourceClass::None,
                month
            ))
        );
        let no_ts = op(r#"{"kind":"capture"}"#, ROW_TYPE_ITEMS);
        assert_eq!(
            outbox_slice_tag(&key, &no_ts),
            None,
            "no ts_unix_ms: left NULL, not guessed"
        );
        let meeting = op(r#"{"kind":"capture","ts_unix_ms":1}"#, ROW_TYPE_MEETING);
        assert_eq!(outbox_slice_tag(&key, &meeting), None);
    }

    /// A relay with the older columns but no `slice_tag`: the latch says yes, the INSERT fails once and the
    /// push goes on untagged (kind_tag survives).
    #[tokio::test]
    async fn a_relay_without_slice_tag_still_takes_the_push_untagged() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("relay.db")).await;
        conn.execute(OLD_SYNC_ROW_DDL, ()).await.unwrap();
        conn.execute("ALTER TABLE sync_row ADD COLUMN kind_tag TEXT", ())
            .await
            .unwrap();
        KIND_TAG_COLUMN_SEEN.set();
        SLICE_TAG_COLUMN_SEEN.set();
        let store = push_fixture_store();
        FAILED_TAGGED_INSERTS.with(|c| c.set(0));

        push_content(&conn, &GroupKey::generate(), &store, "dev-a")
            .await
            .unwrap();

        assert_eq!(
            single_i64(
                &conn,
                "SELECT COUNT(*) FROM sync_row WHERE row_type = 'items'"
            )
            .await,
            2
        );
        assert_eq!(
            single_i64(
                &conn,
                "SELECT COUNT(*) FROM sync_row WHERE kind_tag IS NOT NULL"
            )
            .await,
            2
        );
        assert_eq!(
            FAILED_TAGGED_INSERTS.with(|c| c.get()),
            1,
            "one slice_tag miss, then no more asking"
        );
    }

    /// `kind_tag` missing but `slice_tag` present: the kind arm fires first, slice_tag is still written.
    #[tokio::test]
    async fn a_relay_with_slice_tag_but_no_kind_tag_still_tags_by_slice() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("relay.db")).await;
        conn.execute(OLD_SYNC_ROW_DDL, ()).await.unwrap();
        conn.execute("ALTER TABLE sync_row ADD COLUMN slice_tag TEXT", ())
            .await
            .unwrap();
        KIND_TAG_COLUMN_SEEN.set();
        SLICE_TAG_COLUMN_SEEN.set();
        let store = push_fixture_store();
        FAILED_TAGGED_INSERTS.with(|c| c.set(0));

        push_content(&conn, &GroupKey::generate(), &store, "dev-a")
            .await
            .unwrap();

        assert_eq!(
            single_i64(
                &conn,
                "SELECT COUNT(*) FROM sync_row WHERE slice_tag IS NOT NULL"
            )
            .await,
            2
        );
        assert_eq!(
            FAILED_TAGGED_INSERTS.with(|c| c.get()),
            1,
            "one kind_tag miss, then slice-only"
        );
    }

    /// Both newer columns missing with both latches set: two misses (one per arm), then untagged.
    #[tokio::test]
    async fn a_relay_with_neither_kind_nor_slice_tag_pushes_untagged() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("relay.db")).await;
        conn.execute(OLD_SYNC_ROW_DDL, ()).await.unwrap();
        KIND_TAG_COLUMN_SEEN.set();
        SLICE_TAG_COLUMN_SEEN.set();
        let store = push_fixture_store();
        FAILED_TAGGED_INSERTS.with(|c| c.set(0));

        push_content(&conn, &GroupKey::generate(), &store, "dev-a")
            .await
            .unwrap();

        assert_eq!(
            single_i64(
                &conn,
                "SELECT COUNT(*) FROM sync_row WHERE row_type = 'items'"
            )
            .await,
            2
        );
        assert_eq!(FAILED_TAGGED_INSERTS.with(|c| c.get()), 2);
    }

    #[test]
    fn missing_slice_tag_column_is_recognized() {
        assert!(is_missing_slice_tag_column(
            "table sync_row has no column named slice_tag"
        ));
        assert!(!is_missing_slice_tag_column(
            "table sync_row has no column named kind_tag"
        ));
        assert!(!is_missing_slice_tag_column("database is locked"));
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
        assert_eq!(
            take_compaction_deletes(),
            1,
            "the next round compacts, no restart"
        );
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
        push_content(&conn, &GroupKey::generate(), &store, "device-a")
            .await
            .unwrap();

        assert_eq!(meeting_rows(&conn, "device-a").await, 1);
        assert_eq!(
            single_i64(
                &conn,
                "SELECT COUNT(*) FROM sync_row WHERE entity_tag IS NOT NULL"
            )
            .await,
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
        push_content(&conn, &GroupKey::generate(), &store, "dev-a")
            .await
            .unwrap();
        let spent = started.elapsed();

        assert!(
            spent <= COMPACTION_TIMEOUT + std::time::Duration::from_secs(1),
            "{spent:?}"
        );
        assert_eq!(
            take_compaction_deletes(),
            1,
            "after the timeout, the round stops compacting"
        );
        assert_eq!(
            single_i64(
                &conn,
                "SELECT COUNT(*) FROM sync_row WHERE row_type = 'items' AND entity_tag IS NOT NULL"
            )
            .await,
            120,
            "every chunk is still inserted, tagged"
        );
        assert_eq!(
            single_i64(
                &conn,
                "SELECT MAX(local_seq) FROM sync_row WHERE device_id = 'dev-a'"
            )
            .await,
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
        let mut rows = conn
            .query(&format!("EXPLAIN QUERY PLAN {sql}"), args)
            .await
            .unwrap();
        let mut plan = Vec::new();
        while let Some(row) = rows.next().await.unwrap() {
            plan.push(row.get::<String>(3).unwrap());
        }
        // A bare "USING COVERING INDEX idx_sync_row_entity" with no key terms walks the whole index.
        let seek = "INDEX idx_sync_row_entity (device_id=? AND entity_tag=?)";
        let seeks = plan.iter().filter(|step| step.contains(seek)).count();
        assert_eq!(
            seeks, 2,
            "the outer DELETE and the MAX subquery both seek: {plan:?}"
        );
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
        pull_content(&conn, &group_key, &store_b, "device-b")
            .await
            .unwrap();
        let queries = take_row_queries();
        assert!(
            queries <= 5,
            "300 pending entries took {queries} sync_row queries"
        );
        assert_eq!(
            store_b
                .list_sync_pending_dependencies("device-a")
                .unwrap()
                .len(),
            300,
            "still waiting on the meeting"
        );

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
        let queries = take_row_queries();
        assert!(
            queries <= 5,
            "resolving 300 pending entries took {queries} sync_row queries"
        );
        assert!(store_b
            .list_sync_pending_dependencies("device-a")
            .unwrap()
            .is_empty());
        assert_eq!(
            store_b
                .search_transcript_keyword("pending segment", 500)
                .unwrap()
                .len(),
            300
        );
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
        seed_content_row(
            &conn,
            &group_key,
            "device-a",
            999,
            ROW_TYPE_MEETING,
            &meeting_json,
        )
        .await;
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
        seed_content_row(
            &conn,
            &group_key,
            "device-a",
            1000,
            ROW_TYPE_CAPTURE_ITEM,
            &capture_json,
        )
        .await;

        let zero = std::time::Duration::ZERO;
        let pending = |s: &Store| s.list_sync_pending_dependencies("device-a").unwrap().len();
        // A zero budget still handles one forward row per round; two rounds reach 1000.
        for _ in 0..2 {
            pull_content_with_budget(
                &conn,
                &group_key,
                &store_b,
                "device-b",
                budget_only(zero),
                &PullOptions::default(),
                false,
            )
            .await
            .unwrap();
        }
        assert_eq!(
            store_b.get_sync_cursor("device-a").unwrap(),
            1000,
            "the forward cursor advances"
        );
        assert_eq!(
            store_b
                .search_keyword("fresh forward row", 10)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            pending(&store_b),
            450 - 2 * PENDING_LOOKUP_CHUNK,
            "each round retries one chunk"
        );
        pull_content_with_budget(
            &conn,
            &group_key,
            &store_b,
            "device-b",
            budget_only(zero),
            &PullOptions::default(),
            false,
        )
        .await
        .unwrap();
        assert_eq!(pending(&store_b), 0, "a later round drains the rest");
        assert_eq!(
            store_b
                .search_transcript_keyword("pending segment", 500)
                .unwrap()
                .len(),
            450
        );
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
            store_b
                .list_sync_pending_dependencies("device-a")
                .unwrap()
                .is_empty(),
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
            origin_epoch: None,
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
    async fn pushes_the_snapshot_capability_marker() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("shared.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        let store = Store::open_in_memory().unwrap();
        let group_key = GroupKey::generate();

        push_content(&conn, &group_key, &store, "dev-a")
            .await
            .unwrap();

        let mut rows = conn
            .query(
                "SELECT COUNT(*) FROM sync_row WHERE device_id = ?1 AND row_type = ?2",
                params!["dev-a", ROW_TYPE_TOPIC_SNAPSHOT_CAPABILITY],
            )
            .await
            .unwrap();
        let n: i64 = rows.next().await.unwrap().unwrap().get(0).unwrap();
        assert_eq!(n, 1);
    }

    fn snapshot_json(key: &str, built_at: i64) -> serde_json::Value {
        serde_json::json!({
            "topic_key": key, "name": "Maximus", "kind": null, "deleted": false, "lead": null,
            "role": null, "next_meeting": null, "you_owe": [], "owed_to_you": [], "done": [],
            "recent": [], "recent_total": 0, "truncated": false,
            "built_at_unix_ms": built_at, "built_by": "device-a",
        })
    }

    #[test]
    fn applies_a_pulled_snapshot() {
        let store = Store::open_in_memory().unwrap();
        let body = serde_json::to_vec(&snapshot_json("maximus", crate::now_unix_ms())).unwrap();
        apply_parent_row(&store, "peer", 1, ROW_TYPE_TOPIC_PAGE_SNAPSHOT, &body).unwrap();
        let got = store
            .topic_page_snapshot("maximus")
            .unwrap()
            .expect("stored");
        assert_eq!(got.name, "Maximus");

        // A payload that does not parse is dropped, like the other topic rows.
        apply_parent_row(&store, "peer", 2, ROW_TYPE_TOPIC_PAGE_SNAPSHOT, b"not json").unwrap();
        let wrong = serde_json::to_vec(&serde_json::json!({ "topic_key": "x" })).unwrap();
        apply_parent_row(&store, "peer", 3, ROW_TYPE_TOPIC_PAGE_SNAPSHOT, &wrong).unwrap();
        assert!(store.topic_page_snapshot("x").unwrap().is_none());
        assert_eq!(store.topic_page_snapshots().unwrap().len(), 1);
    }

    #[test]
    fn snapshot_identity_is_tagged() {
        let op = memory::ClaimedOutboxOperation {
            row_type: ROW_TYPE_TOPIC_PAGE_SNAPSHOT.to_string(),
            aggregate_id: "maximus".to_string(),
            operation_id: "op".to_string(),
            device_id: "device-a".to_string(),
            local_seq: 1,
            aggregate_type: ROW_TYPE_TOPIC_PAGE_SNAPSHOT.to_string(),
            operation_kind: memory::OutboxOperationKind::Upsert,
            payload_json: "{}".to_string(),
            connector: memory::OUTBOX_CONNECTOR.to_string(),
            attempts: 0,
        };
        assert_eq!(
            outbox_entity_identity(&op).as_deref(),
            Some("topic_page_snapshot:maximus")
        );
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
            origin_epoch: None,
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
        push_content(&conn, &key, &store_a, "device-a")
            .await
            .unwrap();
        pull_content(&conn, &key, &store_b, "device-b")
            .await
            .unwrap();
        let old_local = store_b.recent_meetings(10).unwrap()[0].id;
        assert_eq!(
            store_b
                .search_transcript_keyword("selfrepair", 10)
                .unwrap()
                .len(),
            1
        );

        store_b.delete_meeting(old_local).unwrap();
        assert!(store_b.recent_meetings(10).unwrap().is_empty());

        store_a
            .update_meeting_transcript_and_notes(id, "/path/transcript.txt", Some("later notes"))
            .unwrap();
        push_content(&conn, &key, &store_a, "device-a")
            .await
            .unwrap();
        pull_content(&conn, &key, &store_b, "device-b")
            .await
            .unwrap();

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
        pull_content(&conn, &key, &store_b, "device-b")
            .await
            .unwrap();
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
        store_b
            .apply_synced_meeting("device-a", 99, None, &meeting)
            .unwrap();
        pull_content(&conn, &key, &store_b, "device-b")
            .await
            .unwrap();
        store_b.set_sync_cursor("device-a", 0).unwrap();
        pull_content(&conn, &key, &store_b, "device-b")
            .await
            .unwrap();
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
        push_content(&conn, &key, &store_a, "device-a")
            .await
            .unwrap();
        pull_content(&conn, &key, &store_b, "device-b")
            .await
            .unwrap();
        store_a
            .update_meeting_transcript_and_notes(id, "/path/transcript.txt", Some("some notes"))
            .unwrap();
        push_content(&conn, &key, &store_a, "device-a")
            .await
            .unwrap();
        store_a
            .update_meeting_summary(id, "tl;dr", &[], &[], &[], "anthropic", "claude-haiku-4-5")
            .unwrap();
        push_content(&conn, &key, &store_a, "device-a")
            .await
            .unwrap();
        assert_eq!(
            meeting_rows(&conn, "device-a").await,
            1,
            "precondition: the older versions were compacted"
        );

        pull_content(&conn, &key, &store_b, "device-b")
            .await
            .unwrap();

        let meetings = store_b.recent_meetings(10).unwrap();
        assert_eq!(meetings.len(), 1);
        assert_eq!(meetings[0].notes.as_deref(), Some("some notes"));
        assert_eq!(meetings[0].summary_tldr.as_deref(), Some("tl;dr"));
        assert_eq!(
            store_b.get_sync_cursor("device-a").unwrap(),
            store_a.get_sync_push_cursor().unwrap()
        );
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

        store_a
            .upsert_calendar_event_items(std::slice::from_ref(&event))
            .unwrap();
        push_content(&conn, &key, &store_a, "device-a")
            .await
            .unwrap();
        pull_content(&conn, &key, &store_b, "device-b")
            .await
            .unwrap();
        for title in ["v2", "v3"] {
            event.title = title.to_string();
            store_a
                .upsert_calendar_event_items(std::slice::from_ref(&event))
                .unwrap();
            push_content(&conn, &key, &store_a, "device-a")
                .await
                .unwrap();
        }
        assert_eq!(
            single_i64(
                &conn,
                "SELECT COUNT(*) FROM sync_row WHERE device_id = 'device-a' AND row_type = 'items'"
            )
            .await,
            1,
            "precondition: v1 and v2 were compacted"
        );
        pull_content(&conn, &key, &store_b, "device-b")
            .await
            .unwrap();

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
        let o = single_i64(
            &conn,
            "SELECT local_seq FROM sync_row WHERE device_id = 'dev-a' AND row_type = 'meeting'",
        )
        .await;
        store_a
            .update_meeting_summary(
                meeting_id,
                "tl;dr",
                &[],
                &[],
                &[],
                "anthropic",
                "claude-haiku-4-5",
            )
            .unwrap();
        push_content(&conn, &key, &store_a, "dev-a").await.unwrap();
        assert_eq!(
            single_i64(
                &conn,
                &format!(
                    "SELECT COUNT(*) FROM sync_row WHERE device_id = 'dev-a' AND local_seq = {o}"
                )
            )
            .await,
            0,
            "precondition: the meeting version the item points at was compacted"
        );
        assert_eq!(
            single_i64(
                &conn,
                "SELECT COUNT(*) FROM sync_row WHERE device_id = 'dev-a' AND row_type = 'items'"
            )
            .await,
            1,
            "precondition: the transcript item is still on the relay"
        );

        let store_b = Store::open_in_memory().unwrap();
        pull_content(&conn, &key, &store_b, "dev-b").await.unwrap();
        pull_content(&conn, &key, &store_b, "dev-b").await.unwrap();

        let n = single_i64(
            &conn,
            "SELECT local_seq FROM sync_row WHERE device_id = 'dev-a' AND row_type = 'meeting'",
        )
        .await;
        let local_meeting = store_b
            .resolve_sync_row_map("dev-a", n, ROW_TYPE_MEETING)
            .unwrap()
            .unwrap();
        let hits = store_b
            .search_transcript_keyword("backfilled transcript line", 10)
            .unwrap();
        assert_eq!((hits.len(), hits[0].meeting_id), (1, local_meeting));
        assert!(store_b
            .list_sync_pending_dependencies("dev-a")
            .unwrap()
            .is_empty());
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
        let meeting_seq = single_i64(
            &conn,
            "SELECT local_seq FROM sync_row WHERE device_id = 'dev-a' AND row_type = 'meeting'",
        )
        .await;
        conn.execute(
            "DELETE FROM sync_row WHERE device_id = 'dev-a' AND row_type = 'meeting'",
            params![],
        )
        .await
        .unwrap();
        let legacy_json = serde_json::to_string(&sample_applied_meeting("Standup")).unwrap();
        seed_content_row(
            &conn,
            &key,
            "dev-a",
            meeting_seq,
            ROW_TYPE_MEETING,
            &legacy_json,
        )
        .await;

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
        let local_meeting = store_c
            .resolve_sync_meeting_map("dev-a", meeting_id)
            .unwrap()
            .unwrap();
        let hits = store_c
            .search_transcript_keyword("backfilled transcript line", 10)
            .unwrap();
        assert_eq!((hits.len(), hits[0].meeting_id), (1, local_meeting));
        assert!(store_c
            .list_sync_pending_dependencies("dev-a")
            .unwrap()
            .is_empty());
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

    fn mcp_item_writer() -> memory::mcp_items::McpWriter {
        memory::mcp_items::McpWriter {
            token_id: 1,
            client_name: "Claude Code".to_string(),
        }
    }

    fn mcp_new_item(id: &str) -> memory::mcp_items::NewMcpItem {
        memory::mcp_items::NewMcpItem {
            label: "outlook-work".to_string(),
            external_id: id.to_string(),
            kind: "email".to_string(),
            ts: "2026-11-21T14:03:00Z".to_string(),
            title: Some("Quokka forecast".to_string()),
            text: "quokkacanary numbers".to_string(),
            url: None,
            people: vec![],
        }
    }

    /// A peer that pushes an `items` row of kind `mcp_item` (a bug, or a newer build) never
    /// creates a row here: added items stay on the computer that received them.
    #[tokio::test]
    async fn a_pulled_mcp_item_row_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("shared.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        let group_key = GroupKey::generate();
        let now = crate::now_unix_ms();
        for (seq, kind, text) in [
            (1, memory::MCP_ITEM_KIND, "quokkacanary added"),
            (2, "capture", "plain peer capture"),
        ] {
            let json = serde_json::to_string(&ItemPayload {
                source_meeting_id: None,
                meeting_local_seq: None,
                item: applied_item(kind, now, text),
            })
            .unwrap();
            seed_content_row(&conn, &group_key, "dev-a", seq, ROW_TYPE_ITEMS, &json).await;
        }
        let store = Store::open_in_memory().unwrap();
        let mut stats = Vec::new();
        pull_round(
            &conn,
            &group_key,
            &store,
            "dev-b",
            NORMAL_PACE,
            &PullOptions::default(),
            true,
            &mut stats,
        )
        .await
        .unwrap();
        assert!(
            applied(&store, 2, ROW_TYPE_ITEMS),
            "the capture applies, so the pull ran"
        );
        assert!(!applied(&store, 1, ROW_TYPE_ITEMS));
        assert_eq!(captures_with_text(&store, "quokkacanary added"), 0);
        assert!(store.search_keyword("quokkacanary", 10).unwrap().is_empty());
        // The catch-up replay applies rows one by one through `apply_parent_row`: ignored there too.
        let body = serde_json::to_vec(&ItemPayload {
            source_meeting_id: None,
            meeting_local_seq: None,
            item: applied_item(memory::MCP_ITEM_KIND, now, "quokkacanary replayed"),
        })
        .unwrap();
        apply_parent_row(&store, "dev-c", 9, ROW_TYPE_ITEMS, &body).unwrap();
        assert!(store
            .resolve_sync_row_map("dev-c", 9, ROW_TYPE_ITEMS)
            .unwrap()
            .is_none());
        assert!(store.search_keyword("quokkacanary", 10).unwrap().is_empty());
    }

    /// Checked at the relay, not only in `unpushed_item_rows`: after added items are written,
    /// updated and deleted and a full push round runs, no relay row decrypts to one.
    #[tokio::test]
    async fn no_relay_row_decrypts_to_an_mcp_item_after_a_full_round() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("shared.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        let group_key = GroupKey::generate();
        let store = Store::open_in_memory().unwrap();
        store.set_own_device_id("device-a");
        store
            .insert_capture_item(&sample_new_capture_item("a capture that is pushed"))
            .unwrap();
        let clock = memory::mcp_items::WriteClock {
            now_unix_ms: 1_795_269_780_000,
            day_start_unix_ms: 1_795_219_200_000,
        };
        {
            let live: &memory::Store = &store;
            while live.list_mcp_client_tokens().unwrap().len() < 12 {
                live.insert_mcp_client_token(&memory::NewMcpClientToken {
                    name: "t".into(),
                    token_hash: "h".into(),
                    token_salt: "s".into(),
                    scopes: vec!["write".into()],
                })
                .unwrap();
            }
        }
        let items: Vec<_> = (0..5).map(|i| mcp_new_item(&format!("m{i}"))).collect();
        store
            .upsert_mcp_items(&mcp_item_writer(), &items, &Default::default(), clock)
            .unwrap();
        let mut changed = mcp_new_item("m0");
        changed.text = "quokkacanary revised".to_string();
        store
            .upsert_mcp_items(&mcp_item_writer(), &[changed], &Default::default(), clock)
            .unwrap();
        store
            .delete_mcp_items(1, "outlook-work", &["m1".to_string()], 1_800_000_000_000)
            .unwrap();

        push_content(&conn, &group_key, &store, "device-a")
            .await
            .unwrap();

        let mut rows = conn
            .query(
                "SELECT device_id, local_seq, row_type, ciphertext FROM sync_row",
                (),
            )
            .await
            .unwrap();
        let mut decrypted = 0;
        while let Some(row) = rows.next().await.unwrap() {
            let device: String = row.get(0).unwrap();
            let seq: i64 = row.get(1).unwrap();
            let row_type: String = row.get(2).unwrap();
            let ciphertext: String = row.get(3).unwrap();
            let aad = content_row_aad(&device, seq, &row_type);
            let plain =
                sync::decrypt_payload(&group_key, &STANDARD.decode(&ciphertext).unwrap(), &aad)
                    .unwrap();
            let text = String::from_utf8_lossy(&plain);
            assert!(
                !text.contains("mcp_item") && !text.contains("quokkacanary"),
                "{row_type}: {text}"
            );
            decrypted += 1;
        }
        assert!(decrypted >= 1, "the capture was pushed, so the round ran");
    }

    fn applied(store: &Store, local_seq: i64, row_type: &str) -> bool {
        store
            .resolve_sync_row_map("dev-a", local_seq, row_type)
            .unwrap()
            .is_some()
    }

    /// Applied captures whose text is exactly `text`. `search_keyword` matches ANY word
    /// of the query (#622), so its bare result count also counts every row that shares
    /// one word ("peer captured text 1" for "peer captured text 503").
    fn captures_with_text(store: &Store, text: &str) -> usize {
        store
            .search_keyword(text, 1000)
            .unwrap()
            .iter()
            .filter(|item| item.text == text)
            .count()
    }

    #[tokio::test]
    async fn a_peer_with_no_round_budget_left_fetches_one_row() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("shared.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        let group_key = GroupKey::generate();
        seed_old_and_new_captures(&conn, &group_key).await;
        let store = Store::open_in_memory().unwrap();
        let spent = Pace {
            round_budget: Some(std::time::Duration::ZERO),
            ..NORMAL_PACE
        };
        let mut stats = Vec::new();
        let more = pull_round(
            &conn,
            &group_key,
            &store,
            "dev-b",
            spent,
            &PullOptions::default(),
            true,
            &mut stats,
        )
        .await
        .unwrap();
        assert!(more, "the peer still has rows");
        assert_eq!(
            stats[0].fetched, 1,
            "one row, not a page the spent round can't apply"
        );
    }

    #[tokio::test]
    async fn pull_round_reports_per_peer_stats() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("shared.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        let group_key = GroupKey::generate();
        seed_old_and_new_captures(&conn, &group_key).await; // seqs 1-4: 2 old, 2 new
                                                            // seq 5: a segment whose meeting never arrives; seq 6: undecryptable.
        let segment = serde_json::to_string(&TranscriptSegmentPayload {
            meeting_local_seq: 999,
            source_meeting_id: None,
            segment: AppliedTranscriptSegment {
                start_cs: 1,
                speaker_label: "A".to_string(),
                text: "s".to_string(),
            },
        })
        .unwrap();
        seed_content_row(
            &conn,
            &group_key,
            "dev-a",
            5,
            ROW_TYPE_TRANSCRIPT_SEGMENT,
            &segment,
        )
        .await;
        seed_content_row(
            &conn,
            &GroupKey::generate(),
            "dev-a",
            6,
            ROW_TYPE_ITEMS,
            "{}",
        )
        .await;

        let store = Store::open_in_memory().unwrap();
        let options = PullOptions {
            capture_window_ms: Some(7 * DAY_MS),
            ..Default::default()
        };
        let budget = NORMAL_PACE;
        let mut stats = Vec::new();
        pull_round(
            &conn, &group_key, &store, "dev-b", budget, &options, true, &mut stats,
        )
        .await
        .unwrap();
        assert_eq!(stats.len(), 1);
        let s = &stats[0];
        assert_eq!(s.peer, peer_tag("dev-a"));
        assert_eq!(s.peer.len(), 6);
        assert_eq!(
            (
                s.cursor_before,
                s.processed_through,
                s.fetched,
                s.fetched_through
            ),
            (0, 6, 6, 6)
        );
        assert_eq!(
            (s.applied, s.skipped_window, s.deferred, s.undecodable),
            (2, 2, 1, 1)
        );
        assert_eq!(
            (
                s.pending_before,
                s.pending_after,
                s.already_applied,
                s.budget_stopped
            ),
            (0, 1, 0, false)
        );

        // A caught-up round fetches nothing and the cursor stays put.
        let mut stats = Vec::new();
        pull_round(
            &conn, &group_key, &store, "dev-b", budget, &options, true, &mut stats,
        )
        .await
        .unwrap();
        assert_eq!(
            (
                stats[0].cursor_before,
                stats[0].processed_through,
                stats[0].fetched
            ),
            (6, 6, 0)
        );
        assert_eq!((stats[0].pending_before, stats[0].pending_after), (1, 1));

        // The public wrapper publishes JSON without the raw device id.
        pull_content_with_options(&conn, &group_key, &store, "dev-b", &options)
            .await
            .unwrap();
        let json = last_pull_stats_json();
        assert!(json.starts_with('[') && !json.contains("dev-a"), "{json}");
    }

    #[tokio::test]
    async fn capture_items_older_than_the_window_are_skipped_newer_applied() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("shared.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        let group_key = GroupKey::generate();
        seed_old_and_new_captures(&conn, &group_key).await;

        let store = Store::open_in_memory().unwrap();
        let options = PullOptions {
            capture_window_ms: Some(7 * DAY_MS),
            ..Default::default()
        };
        pull_content_with_options(&conn, &group_key, &store, "dev-b", &options)
            .await
            .unwrap();

        assert!(
            !applied(&store, 1, ROW_TYPE_CAPTURE_ITEM),
            "legacy capture older than the window"
        );
        assert!(
            applied(&store, 2, ROW_TYPE_CAPTURE_ITEM),
            "legacy capture inside the window"
        );
        assert!(
            !applied(&store, 3, ROW_TYPE_ITEMS),
            "items capture older than the window"
        );
        assert!(
            applied(&store, 4, ROW_TYPE_ITEMS),
            "items capture inside the window"
        );
        assert_eq!(captures_with_text(&store, "legacy old capture"), 0);
        assert_eq!(captures_with_text(&store, "items old capture"), 0);
        assert_eq!(captures_with_text(&store, "legacy new capture"), 1);
        assert_eq!(captures_with_text(&store, "items new capture"), 1);
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
        seed_content_row(
            &conn,
            &group_key,
            "dev-a",
            1,
            ROW_TYPE_ITEMS,
            &items(now - DAY_MS, "new"),
        )
        .await;
        seed_content_row(
            &conn,
            &group_key,
            "dev-a",
            2,
            ROW_TYPE_ITEMS,
            &items(now - 30 * DAY_MS, "old"),
        )
        .await;
        seed_content_row(
            &conn,
            &group_key,
            "dev-a",
            3,
            ROW_TYPE_ITEMS,
            &items(0, "items ts zero"),
        )
        .await;
        seed_content_row(
            &conn,
            &group_key,
            "dev-a",
            4,
            ROW_TYPE_CAPTURE_ITEM,
            &legacy(0, "legacy ts zero", 4),
        )
        .await;
        seed_content_row(
            &conn,
            &group_key,
            "dev-a",
            5,
            ROW_TYPE_CAPTURE_ITEM,
            &legacy(now - 30 * DAY_MS, "legacy old", 5),
        )
        .await;

        let store = Store::open_in_memory().unwrap();
        let options = PullOptions {
            capture_window_ms: Some(7 * DAY_MS),
            ..Default::default()
        };
        pull_content_with_options(&conn, &group_key, &store, "dev-b", &options)
            .await
            .unwrap();

        assert!(applied(&store, 1, ROW_TYPE_ITEMS));
        for (seq, rt) in [
            (2, ROW_TYPE_ITEMS),
            (3, ROW_TYPE_ITEMS),
            (4, ROW_TYPE_CAPTURE_ITEM),
            (5, ROW_TYPE_CAPTURE_ITEM),
        ] {
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
        let options = PullOptions {
            capture_window_ms: Some(7 * DAY_MS),
            ..Default::default()
        };
        pull_content_with_options(&conn, &group_key, &store, "dev-b", &options)
            .await
            .unwrap();
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
            origin_epoch: None,
            meeting,
        })
        .unwrap();
        seed_content_row(
            &conn,
            &group_key,
            "dev-a",
            1,
            ROW_TYPE_MEETING,
            &meeting_json,
        )
        .await;
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
        seed_content_row(
            &conn,
            &group_key,
            "dev-a",
            2,
            ROW_TYPE_TRANSCRIPT_SEGMENT,
            &segment_json,
        )
        .await;
        let transcript_json = serde_json::to_string(&ItemPayload {
            source_meeting_id: Some(100),
            meeting_local_seq: Some(1),
            item: applied_item("transcript", old, "old transcript item"),
        })
        .unwrap();
        seed_content_row(
            &conn,
            &group_key,
            "dev-a",
            3,
            ROW_TYPE_ITEMS,
            &transcript_json,
        )
        .await;
        let calendar_json = serde_json::to_string(&ItemPayload {
            source_meeting_id: None,
            meeting_local_seq: None,
            item: applied_item("calendar_event", old, "old calendar event"),
        })
        .unwrap();
        seed_content_row(
            &conn,
            &group_key,
            "dev-a",
            4,
            ROW_TYPE_ITEMS,
            &calendar_json,
        )
        .await;

        let store = Store::open_in_memory().unwrap();
        let options = PullOptions {
            capture_window_ms: Some(7 * DAY_MS),
            ..Default::default()
        };
        pull_content_with_options(&conn, &group_key, &store, "dev-b", &options)
            .await
            .unwrap();

        assert!(applied(&store, 1, ROW_TYPE_MEETING), "meeting");
        assert!(
            applied(&store, 2, ROW_TYPE_TRANSCRIPT_SEGMENT),
            "legacy transcript segment"
        );
        assert!(applied(&store, 3, ROW_TYPE_ITEMS), "transcript-kind item");
        assert!(
            applied(&store, 4, ROW_TYPE_ITEMS),
            "calendar_event-kind item"
        );
    }

    #[tokio::test]
    async fn no_window_applies_everything_as_today() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("shared.db")).await;
        ensure_content_schema(&conn).await.unwrap();
        let group_key = GroupKey::generate();
        seed_old_and_new_captures(&conn, &group_key).await;

        let via_default = Store::open_in_memory().unwrap();
        pull_content(&conn, &group_key, &via_default, "dev-b")
            .await
            .unwrap();
        let via_none = Store::open_in_memory().unwrap();
        pull_content_with_options(
            &conn,
            &group_key,
            &via_none,
            "dev-b",
            &PullOptions::default(),
        )
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
        let row = SyncableRow::Item {
            local_seq: 1,
            source_meeting_id: None,
            meeting_local_seq: None,
            item,
        };
        assert_eq!(
            entity_identity(&row),
            Some("items:calendar_event:google_calendar:evt-1".to_string())
        );
    }

    #[test]
    fn a_calendar_item_missing_source_or_external_id_is_untagged() {
        for (source, external_id) in [
            (None, Some("evt-1")),
            (Some("google_calendar"), None),
            (None, None),
        ] {
            let mut item = applied_item("calendar_event", 1, "standup");
            item.source = source.map(str::to_string);
            item.external_id = external_id.map(str::to_string);
            let row = SyncableRow::Item {
                local_seq: 1,
                source_meeting_id: None,
                meeting_local_seq: None,
                item,
            };
            assert_eq!(entity_identity(&row), None, "{source:?} {external_id:?}");
        }
    }

    #[test]
    fn captures_transcripts_and_legacy_rows_are_untagged() {
        for kind in ["capture", "transcript"] {
            let mut item = applied_item(kind, 1, "x");
            item.external_id = Some("e".to_string());
            let row = SyncableRow::Item {
                local_seq: 1,
                source_meeting_id: Some(7),
                meeting_local_seq: Some(2),
                item,
            };
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
            segment: AppliedTranscriptSegment {
                start_cs: 0,
                speaker_label: "A".to_string(),
                text: "hi".to_string(),
            },
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
            assert_eq!(
                outbox_entity_identity(&op(row_type, id)),
                Some(expected.to_string())
            );
        }
        for row_type in [ROW_TYPE_ITEMS, ROW_TYPE_PROFILE_SUGGESTION] {
            assert_eq!(
                outbox_entity_identity(&op(row_type, "9")),
                None,
                "{row_type}"
            );
        }
    }
}

/// #572: the phone pulls each peer's backlog newest first (a live range above the old
/// cursor's top, and a gap below it pulled downward), so rows arrive newer-before-older.
#[cfg(test)]
mod newest_first_tests {
    use super::*;
    use memory::AppliedCaptureItem;

    const PEER: &str = "dev-a";

    async fn relay() -> (tempfile::TempDir, Connection, GroupKey) {
        let dir = tempfile::tempdir().unwrap();
        let db = libsql::Builder::new_local(dir.path().join("relay.db"))
            .build()
            .await
            .unwrap();
        let conn = db.connect().unwrap();
        ensure_content_schema(&conn).await.unwrap();
        (dir, conn, GroupKey::generate())
    }

    async fn seed(
        conn: &Connection,
        key: &GroupKey,
        device: &str,
        seq: i64,
        row_type: &str,
        json: &str,
    ) {
        let aad = content_row_aad(device, seq, row_type);
        let ciphertext = sync::encrypt_payload(key, json.as_bytes(), &aad);
        conn.execute(
            "INSERT INTO sync_row (device_id, local_seq, row_type, ciphertext, created_at_unix_ms) VALUES (?1, ?2, ?3, ?4, 0)",
            params![device, seq, row_type, STANDARD.encode(&ciphertext).as_str()],
        )
        .await
        .unwrap();
    }

    fn capture_json(seq: i64) -> String {
        serde_json::to_string(&AppliedCaptureItem {
            ts_unix_ms: now_unix_ms(),
            app_bundle_id: "com.a".to_string(),
            window_title: "W".to_string(),
            url: None,
            text: format!("capture row {seq}"),
            hash: seq,
            source: Some("ax".to_string()),
        })
        .unwrap()
    }

    async fn seed_captures(conn: &Connection, key: &GroupKey, seqs: std::ops::RangeInclusive<i64>) {
        for seq in seqs {
            seed(
                conn,
                key,
                PEER,
                seq,
                ROW_TYPE_CAPTURE_ITEM,
                &capture_json(seq),
            )
            .await;
        }
    }

    /// Times `capture row {seq}` was applied (0 = skipped, 2 = applied twice).
    fn capture_count(store: &Store, seq: i64) -> usize {
        let text = format!("capture row {seq}");
        store
            .search_keyword(&text, 1000)
            .unwrap()
            .iter()
            .filter(|item| item.text == text)
            .count()
    }

    fn meeting_json(source_id: i64, epoch: Option<&str>, title: &str) -> String {
        serde_json::to_string(&MeetingPayload {
            source_meeting_id: Some(source_id),
            origin_epoch: epoch.map(str::to_string),
            meeting: memory::AppliedMeeting {
                attendee_details: vec![],
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
                trimmed_keep_ms: None,
            },
        })
        .unwrap()
    }

    fn transcript_json(source_id: i64, meeting_local_seq: i64, text: &str) -> String {
        serde_json::to_string(&ItemPayload {
            source_meeting_id: Some(source_id),
            meeting_local_seq: Some(meeting_local_seq),
            item: memory::AppliedItem {
                kind: "transcript".to_string(),
                source: None,
                ts_unix_ms: 1500,
                actors: Some("[\"Me\"]".to_string()),
                title: None,
                body_text: text.to_string(),
                refs: None,
                flags: None,
                hash: None,
                external_id: None,
                app_bundle_id: None,
                url: None,
            },
        })
        .unwrap()
    }

    fn pace(rows: i64) -> Pace {
        Pace {
            rows,
            peer_budget: std::time::Duration::from_secs(60),
            round_budget: None,
        }
    }

    /// One phone round (newest first). Returns whether rows are still waiting.
    async fn phone_round(conn: &Connection, key: &GroupKey, store: &Store, pace: Pace) -> bool {
        pull_content_with_budget_mode(
            conn,
            key,
            store,
            "phone",
            pace,
            &PullOptions::default(),
            true,
            true,
        )
        .await
        .unwrap()
    }

    /// Phone rounds until nothing waits; panics if that takes more than `max` rounds.
    async fn drain(
        conn: &Connection,
        key: &GroupKey,
        store: &Store,
        pace: Pace,
        max: usize,
    ) -> usize {
        for round in 1..=max {
            if !phone_round(conn, key, store, pace).await {
                return round;
            }
        }
        panic!("still pulling after {max} rounds");
    }

    fn meeting_titles(store: &Store) -> Vec<String> {
        store
            .recent_meetings(100)
            .unwrap()
            .into_iter()
            .map(|m| m.title)
            .collect()
    }

    #[tokio::test]
    async fn a_fresh_phone_gets_the_newest_meeting_and_its_transcript_in_round_one() {
        let (_dir, conn, key) = relay().await;
        seed_captures(&conn, &key, 1..=30).await;
        seed(
            &conn,
            &key,
            PEER,
            31,
            ROW_TYPE_ITEMS,
            &transcript_json(7, 33, "hello there"),
        )
        .await;
        seed(
            &conn,
            &key,
            PEER,
            32,
            ROW_TYPE_ITEMS,
            &transcript_json(7, 33, "general kenobi"),
        )
        .await;
        seed(
            &conn,
            &key,
            PEER,
            33,
            ROW_TYPE_MEETING,
            &meeting_json(7, Some("e1"), "Newest"),
        )
        .await;
        let phone = Store::open_in_memory().unwrap();

        assert!(
            phone_round(&conn, &key, &phone, pace(5)).await,
            "older history waits"
        );
        assert_eq!(meeting_titles(&phone), vec!["Newest"]);
        for seq in [31, 32] {
            assert!(
                phone
                    .resolve_sync_row_map(PEER, seq, ROW_TYPE_ITEMS)
                    .unwrap()
                    .is_some(),
                "transcript row {seq} applied against its meeting"
            );
        }
        assert!(phone
            .list_sync_pending_dependencies(PEER)
            .unwrap()
            .is_empty());
        assert_eq!(capture_count(&phone, 30), 1);
        assert_eq!(capture_count(&phone, 1), 0, "the oldest rows come later");
        assert_eq!(
            phone.get_sync_cursor(PEER).unwrap(),
            33,
            "the live range starts at the top"
        );
        assert_eq!(phone.sync_gap(PEER).unwrap(), Some((0, 29)));
        assert!(phone.older_sync_history_loading().unwrap());

        drain(&conn, &key, &phone, pace(5), 10).await;
        for seq in 1..=30 {
            assert_eq!(capture_count(&phone, seq), 1, "capture {seq}");
        }
        assert_eq!(phone.sync_gap(PEER).unwrap(), None);
        assert!(!phone.older_sync_history_loading().unwrap());
        assert_eq!(phone.get_sync_cursor(PEER).unwrap(), 33);
        assert!(
            !phone_round(&conn, &key, &phone, pace(5)).await,
            "caught up"
        );
    }

    #[tokio::test]
    async fn an_older_meeting_version_pulled_after_a_newer_one_never_wins() {
        let (_dir, conn, key) = relay().await;
        seed(
            &conn,
            &key,
            PEER,
            2,
            ROW_TYPE_MEETING,
            &meeting_json(7, Some("e1"), "Draft"),
        )
        .await;
        // A legacy line names the meeting by the older snapshot's seq only.
        let legacy_line = serde_json::to_string(&TranscriptSegmentPayload {
            meeting_local_seq: 2,
            source_meeting_id: None,
            segment: memory::AppliedTranscriptSegment {
                start_cs: 1,
                speaker_label: "A".to_string(),
                text: "legacy line".to_string(),
            },
        })
        .unwrap();
        seed(
            &conn,
            &key,
            PEER,
            3,
            ROW_TYPE_TRANSCRIPT_SEGMENT,
            &legacy_line,
        )
        .await;
        // An older build's snapshot (no epoch) is still older by seq.
        let no_epoch = meeting_json(7, None, "No epoch");
        seed(&conn, &key, PEER, 4, ROW_TYPE_MEETING, &no_epoch).await;
        seed(
            &conn,
            &key,
            PEER,
            9,
            ROW_TYPE_MEETING,
            &meeting_json(7, Some("e1"), "Final"),
        )
        .await;
        let phone = Store::open_in_memory().unwrap();

        drain(&conn, &key, &phone, pace(1), 10).await;
        assert_eq!(meeting_titles(&phone), vec!["Final"]);
        let id = phone.resolve_sync_meeting_map(PEER, 7).unwrap().unwrap();
        assert_eq!(
            phone
                .resolve_sync_row_map(PEER, 2, ROW_TYPE_MEETING)
                .unwrap(),
            Some(id),
            "the skipped snapshot still names the meeting for legacy lines"
        );
        assert!(phone
            .resolve_sync_row_map(PEER, 3, ROW_TYPE_TRANSCRIPT_SEGMENT)
            .unwrap()
            .is_some());
        assert!(phone
            .list_sync_pending_dependencies(PEER)
            .unwrap()
            .is_empty());

        // Another epoch at a lower seq is older too: the relay seq decides, as ascending would.
        apply_parent_row(
            &phone,
            PEER,
            1,
            ROW_TYPE_MEETING,
            meeting_json(7, Some("e2"), "Recreated").as_bytes(),
        )
        .unwrap();
        assert_eq!(meeting_titles(&phone), vec!["Final"]);

        // In ascending order (a Mac) every version still applies.
        let mac = Store::open_in_memory().unwrap();
        for (seq, title) in [(2, "Draft"), (9, "Final"), (12, "Renamed")] {
            apply_parent_row(
                &mac,
                PEER,
                seq,
                ROW_TYPE_MEETING,
                meeting_json(7, Some("e1"), title).as_bytes(),
            )
            .unwrap();
            assert_eq!(meeting_titles(&mac), vec![title]);
        }
    }

    #[tokio::test]
    async fn an_older_routine_run_or_live_alert_never_overwrites_a_newer_one() {
        let (_dir, conn, key) = relay().await;
        let run = |summary: &str| {
            serde_json::to_string(&memory::AppliedRoutineRun {
                source_run_id: 4,
                template: "daily_brief".into(),
                ran_at_unix_ms: 9_000,
                status: memory::RunStatus::Success,
                output_summary: summary.into(),
                output_json: "{}".into(),
            })
            .unwrap()
        };
        let alert = |draft: Option<&str>| {
            serde_json::to_string(&memory::live_alert::AppliedLiveAlert {
                source_id: 3,
                source_meeting_id: 7,
                meeting_title: "Newest".into(),
                quote: "can you send the deck".into(),
                restatement: "send the deck".into(),
                asker: None,
                at_cs: 10,
                created_at_unix_ms: 1_000,
                draft_json: draft.map(str::to_string),
            })
            .unwrap()
        };
        seed(
            &conn,
            &key,
            PEER,
            1,
            ROW_TYPE_MEETING,
            &meeting_json(7, Some("e1"), "Newest"),
        )
        .await;
        seed(
            &conn,
            &key,
            PEER,
            2,
            ROW_TYPE_ROUTINE_RUN,
            &run("first brief"),
        )
        .await;
        seed(&conn, &key, PEER, 3, ROW_TYPE_LIVE_ALERT, &alert(None)).await;
        seed(
            &conn,
            &key,
            PEER,
            4,
            ROW_TYPE_ROUTINE_RUN,
            &run("revised brief"),
        )
        .await;
        seed(
            &conn,
            &key,
            PEER,
            5,
            ROW_TYPE_LIVE_ALERT,
            &alert(Some("{\"body\":\"Here it is\"}")),
        )
        .await;
        let phone = Store::open_in_memory().unwrap();

        drain(&conn, &key, &phone, pace(1), 10).await;
        let runs = phone.recent_routine_runs_across_all(10).unwrap();
        let meeting = phone.resolve_sync_meeting_map(PEER, 7).unwrap().unwrap();
        let alerts = phone.list_live_alerts(meeting).unwrap();
        assert_eq!((runs.len(), alerts.len()), (1, 1));
        assert_eq!(
            (
                runs[0].output_summary.as_str(),
                alerts[0].draft_json.as_deref()
            ),
            ("revised brief", Some("{\"body\":\"Here it is\"}")),
            "the newer run and the alert's draft are kept"
        );
    }

    #[tokio::test]
    async fn an_older_calendar_event_version_never_overwrites_a_newer_one() {
        let (_dir, conn, key) = relay().await;
        let event = |title: &str| {
            serde_json::to_string(&ItemPayload {
                source_meeting_id: None,
                meeting_local_seq: None,
                item: memory::AppliedItem {
                    kind: "calendar_event".to_string(),
                    source: Some("google_calendar".to_string()),
                    ts_unix_ms: now_unix_ms() + 60_000,
                    actors: None,
                    title: Some(title.to_string()),
                    body_text: title.to_string(),
                    refs: None,
                    flags: Some("{\"changed_at\":100}".to_string()),
                    hash: None,
                    external_id: Some("evt-1".to_string()),
                    app_bundle_id: None,
                    url: None,
                },
            })
            .unwrap()
        };
        seed(&conn, &key, PEER, 1, ROW_TYPE_ITEMS, &event("v1")).await;
        seed(&conn, &key, PEER, 2, ROW_TYPE_ITEMS, &event("v2")).await;
        let phone = Store::open_in_memory().unwrap();

        drain(&conn, &key, &phone, pace(1), 10).await;
        let events = phone.upcoming_calendar_events(0, i64::MAX, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].title, "v2");
    }

    fn forgotten_json(origin: &str, source_id: i64, epoch: &str) -> String {
        serde_json::to_string(&memory::MeetingForgotten {
            origin_device_id: origin.to_string(),
            origin_epoch: Some(epoch.to_string()),
            source_meeting_id: source_id,
            forgotten_at_unix_ms: 5_000,
            source_start_unix_ms: None,
        })
        .unwrap()
    }

    #[tokio::test]
    async fn a_forget_tombstone_wins_before_or_after_its_meeting() {
        // The recorder's own tombstone sits above its meeting, so newest first pulls it first.
        let (_dir, conn, key) = relay().await;
        seed(
            &conn,
            &key,
            PEER,
            4,
            ROW_TYPE_ITEMS,
            &transcript_json(7, 5, "a line"),
        )
        .await;
        seed(
            &conn,
            &key,
            PEER,
            5,
            ROW_TYPE_MEETING,
            &meeting_json(7, Some("e1"), "Forgotten"),
        )
        .await;
        seed(
            &conn,
            &key,
            PEER,
            6,
            ROW_TYPE_MEETING_FORGOTTEN,
            &forgotten_json(PEER, 7, "e1"),
        )
        .await;
        let phone = Store::open_in_memory().unwrap();
        drain(&conn, &key, &phone, pace(1), 10).await;
        assert!(meeting_titles(&phone).is_empty());
        assert!(
            phone
                .list_sync_pending_dependencies(PEER)
                .unwrap()
                .is_empty(),
            "its line is dropped, not left waiting"
        );

        // Another device's tombstone, pulled before ("dev-0" sorts first) or after ("dev-z") the meeting.
        for forgetter in ["dev-0", "dev-z"] {
            let (_dir, conn, key) = relay().await;
            seed(
                &conn,
                &key,
                PEER,
                5,
                ROW_TYPE_MEETING,
                &meeting_json(7, Some("e1"), "Forgotten"),
            )
            .await;
            seed(
                &conn,
                &key,
                PEER,
                4,
                ROW_TYPE_ITEMS,
                &transcript_json(7, 5, "a line"),
            )
            .await;
            seed(
                &conn,
                &key,
                forgetter,
                3,
                ROW_TYPE_MEETING_FORGOTTEN,
                &forgotten_json(PEER, 7, "e1"),
            )
            .await;
            let phone = Store::open_in_memory().unwrap();
            drain(&conn, &key, &phone, pace(1), 10).await;
            assert!(
                meeting_titles(&phone).is_empty(),
                "forgotten by {forgetter}"
            );
            assert!(phone
                .list_sync_pending_dependencies(PEER)
                .unwrap()
                .is_empty());
        }
    }

    #[tokio::test]
    async fn a_part_way_phone_switches_without_skipping_or_repeating_a_row() {
        let (_dir, conn, key) = relay().await;
        seed_captures(&conn, &key, 1..=23).await;
        let phone = Store::open_in_memory().unwrap();
        // An older build pulled two ascending pages.
        for _ in 0..2 {
            pull_content_with_budget_mode(
                &conn,
                &key,
                &phone,
                "phone",
                pace(5),
                &PullOptions::default(),
                true,
                false,
            )
            .await
            .unwrap();
        }
        assert_eq!(phone.get_sync_cursor(PEER).unwrap(), 10);

        // This build: the live page from 10 is full, so it applies it and switches.
        assert!(phone_round(&conn, &key, &phone, pace(5)).await);
        assert_eq!(phone.get_sync_cursor(PEER).unwrap(), 23);
        assert!(phone.sync_gap(PEER).unwrap().is_some());
        assert_eq!(
            capture_count(&phone, 23),
            1,
            "the newest rows arrive in the switching round"
        );

        drain(&conn, &key, &phone, pace(5), 10).await;
        for seq in 1..=23 {
            assert_eq!(capture_count(&phone, seq), 1, "capture {seq}");
        }
        assert_eq!(phone.sync_gap(PEER).unwrap(), None);
    }

    #[tokio::test]
    async fn a_zero_budget_still_moves_the_gap_one_row_a_round() {
        let (_dir, conn, key) = relay().await;
        seed_captures(&conn, &key, 1..=6).await;
        let phone = Store::open_in_memory().unwrap();
        let zero = Pace {
            peer_budget: std::time::Duration::ZERO,
            ..pace(5)
        };
        let rounds = drain(&conn, &key, &phone, zero, 20).await;
        assert!(rounds <= 7, "{rounds} rounds");
        for seq in 1..=6 {
            assert_eq!(capture_count(&phone, seq), 1, "capture {seq}");
        }
    }

    /// A budget spent after the first row stops both the live page and the gap page part-way.
    #[tokio::test]
    async fn pages_stopped_part_way_still_skip_and_repeat_nothing() {
        let (_dir, conn, key) = relay().await;
        seed_captures(&conn, &key, 1..=23).await;
        let phone = Store::open_in_memory().unwrap();
        for _ in 0..2 {
            pull_content_with_budget_mode(
                &conn,
                &key,
                &phone,
                "phone",
                pace(5),
                &PullOptions::default(),
                true,
                false,
            )
            .await
            .unwrap();
        }
        let tight = Pace {
            rows: 5,
            peer_budget: std::time::Duration::from_nanos(1),
            round_budget: Some(std::time::Duration::from_secs(60)),
        };
        let mut stats = Vec::new();
        assert!(pull_round_mode(
            &conn,
            &key,
            &phone,
            "phone",
            tight,
            &PullOptions::default(),
            true,
            true,
            &mut stats
        )
        .await
        .unwrap());
        assert!(stats[0].budget_stopped && stats[0].gap_opened);
        assert_eq!(
            (stats[0].fetched, stats[0].processed_through),
            (5, 11),
            "one live row of five"
        );
        assert_eq!(
            (stats[0].gap_fetched, stats[0].gap_lo, stats[0].gap_hi),
            (1, Some(11), Some(23)),
            "with under a second left the gap fetches one row"
        );
        assert_eq!(phone.get_sync_cursor(PEER).unwrap(), 23);

        drain(&conn, &key, &phone, tight, 30).await;
        for seq in 1..=23 {
            assert_eq!(capture_count(&phone, seq), 1, "capture {seq}");
        }
    }

    /// A peer whose rows left the relay mid-backfill (device removed, relay reset) has nothing
    /// left to pull: its gap closes instead of keeping older history loading forever.
    #[tokio::test]
    async fn a_gap_whose_peer_left_the_relay_closes() {
        let (_dir, conn, key) = relay().await;
        seed_captures(&conn, &key, 1..=20).await;
        let phone = Store::open_in_memory().unwrap();
        assert!(phone_round(&conn, &key, &phone, pace(5)).await);
        assert!(phone.sync_gap(PEER).unwrap().is_some());

        conn.execute("DELETE FROM sync_row WHERE device_id = ?1", params![PEER])
            .await
            .unwrap();
        let mut stats = Vec::new();
        let more = pull_round_mode(
            &conn,
            &key,
            &phone,
            "phone",
            pace(5),
            &PullOptions::default(),
            true,
            true,
            &mut stats,
        )
        .await
        .unwrap();
        assert!(!more);
        assert_eq!(phone.sync_gap(PEER).unwrap(), None);
        assert!(!phone.older_sync_history_loading().unwrap());
        assert_eq!(stats.len(), 1, "the gone peer is reported");
        assert_eq!(stats[0].peer, peer_tag(PEER));
        assert!(stats[0].gap_closed_peer_gone);
    }

    /// A re-created peer database reuses meeting id 9. Ascending, the new database's version
    /// (higher relay seq) applies last and stays; newest first must keep it too.
    #[tokio::test]
    async fn a_re_created_databases_meeting_wins_in_either_order() {
        let (_dir, conn, key) = relay().await;
        seed(
            &conn,
            &key,
            PEER,
            900,
            ROW_TYPE_MEETING,
            &meeting_json(9, Some("e1"), "Old database"),
        )
        .await;
        seed(
            &conn,
            &key,
            PEER,
            1005,
            ROW_TYPE_MEETING,
            &meeting_json(9, Some("e2"), "New database"),
        )
        .await;
        let phone = Store::open_in_memory().unwrap();
        drain(&conn, &key, &phone, pace(1), 10).await;
        assert_eq!(meeting_titles(&phone), vec!["New database"]);
        let mac = Store::open_in_memory().unwrap();
        pull_content(&conn, &key, &mac, "mac").await.unwrap();
        assert_eq!(meeting_titles(&mac), vec!["New database"]);
    }

    /// Two peers' gaps under a spent round budget: each still moves a row a round.
    #[tokio::test]
    async fn every_open_gap_progresses_under_a_tight_round_budget() {
        let (_dir, conn, key) = relay().await;
        seed_captures(&conn, &key, 1..=6).await;
        for seq in 1..=6 {
            seed(
                &conn,
                &key,
                "dev-b",
                seq,
                ROW_TYPE_CAPTURE_ITEM,
                &capture_json(100 + seq),
            )
            .await;
        }
        let phone = Store::open_in_memory().unwrap();
        let spent = Pace {
            rows: 5,
            peer_budget: std::time::Duration::from_secs(10),
            round_budget: Some(std::time::Duration::ZERO),
        };
        assert!(phone_round(&conn, &key, &phone, spent).await);
        assert_eq!(phone.sync_gap(PEER).unwrap(), Some((0, 6)));
        assert_eq!(phone.sync_gap("dev-b").unwrap(), Some((0, 6)));
        assert!(phone_round(&conn, &key, &phone, spent).await);
        assert_eq!(phone.sync_gap(PEER).unwrap(), Some((0, 5)));
        assert_eq!(phone.sync_gap("dev-b").unwrap(), Some((0, 5)));
        // A spent round always reports more, so finish at a normal pace.
        for _ in 0..4 {
            phone_round(&conn, &key, &phone, spent).await;
        }
        drain(&conn, &key, &phone, pace(5), 5).await;
        for seq in 1..=6 {
            assert_eq!(capture_count(&phone, seq), 1);
            assert_eq!(capture_count(&phone, 100 + seq), 1);
        }
    }

    /// A crash after a gap page applied but before its gap_hi was saved re-runs the page:
    /// nothing is applied twice.
    #[tokio::test]
    async fn a_gap_page_re_run_after_a_crash_applies_nothing_twice() {
        let (_dir, conn, key) = relay().await;
        seed_captures(&conn, &key, 1..=3).await;
        seed(
            &conn,
            &key,
            PEER,
            4,
            ROW_TYPE_ITEMS,
            &transcript_json(7, 6, "onlyonce"),
        )
        .await;
        seed(
            &conn,
            &key,
            PEER,
            5,
            ROW_TYPE_ITEMS,
            &transcript_json(7, 6, "againonce"),
        )
        .await;
        seed(
            &conn,
            &key,
            PEER,
            6,
            ROW_TYPE_MEETING,
            &meeting_json(7, Some("e1"), "Once"),
        )
        .await;
        let phone = Store::open_in_memory().unwrap();
        assert!(phone_round(&conn, &key, &phone, pace(4)).await);
        assert_eq!(phone.sync_gap(PEER).unwrap(), Some((0, 3)));
        // The crash: gap_hi never moved.
        phone.set_sync_gap_hi(PEER, 7).unwrap();
        drain(&conn, &key, &phone, pace(4), 10).await;
        assert_eq!(meeting_titles(&phone), vec!["Once"]);
        for word in ["onlyonce", "againonce"] {
            assert_eq!(
                phone.search_transcript_keyword(word, 100).unwrap().len(),
                1,
                "{word}"
            );
        }
        for seq in 1..=3 {
            assert_eq!(capture_count(&phone, seq), 1);
        }
    }

    /// The pending pass fetches an older meeting snapshot by key; the gap brings the newer one
    /// later, and the newer one stays.
    #[tokio::test]
    async fn a_by_key_older_snapshot_then_the_gaps_newer_one_leaves_the_newer() {
        let (_dir, conn, key) = relay().await;
        seed(
            &conn,
            &key,
            PEER,
            10,
            ROW_TYPE_MEETING,
            &meeting_json(7, Some("e1"), "Final"),
        )
        .await;
        seed_captures(&conn, &key, 11..=15).await;
        seed(
            &conn,
            &key,
            PEER,
            50,
            ROW_TYPE_ITEMS,
            &transcript_json(7, 2, "a line"),
        )
        .await;
        let phone = Store::open_in_memory().unwrap();
        // Round 1: the line's snapshot (seq 2) isn't on the relay yet, so it waits.
        phone_round(&conn, &key, &phone, pace(1)).await;
        assert_eq!(phone.list_sync_pending_dependencies(PEER).unwrap().len(), 1);
        assert!(meeting_titles(&phone).is_empty());
        // It lands; the pending pass fetches it by key before the gap reaches seq 10.
        seed(
            &conn,
            &key,
            PEER,
            2,
            ROW_TYPE_MEETING,
            &meeting_json(7, Some("e1"), "Draft"),
        )
        .await;
        phone_round(&conn, &key, &phone, pace(1)).await;
        assert_eq!(meeting_titles(&phone), vec!["Draft"]);
        assert!(phone
            .list_sync_pending_dependencies(PEER)
            .unwrap()
            .is_empty());
        drain(&conn, &key, &phone, pace(1), 10).await;
        assert_eq!(meeting_titles(&phone), vec!["Final"]);
    }

    #[tokio::test]
    async fn live_rows_arriving_while_the_gap_is_open_are_pulled_first() {
        let (_dir, conn, key) = relay().await;
        seed_captures(&conn, &key, 1..=20).await;
        let phone = Store::open_in_memory().unwrap();
        assert!(phone_round(&conn, &key, &phone, pace(5)).await);
        assert_eq!(phone.sync_gap(PEER).unwrap(), Some((0, 16)));

        seed_captures(&conn, &key, 21..=23).await;
        assert!(phone_round(&conn, &key, &phone, pace(5)).await);
        for seq in 21..=23 {
            assert_eq!(capture_count(&phone, seq), 1, "live capture {seq}");
        }
        assert_eq!(phone.get_sync_cursor(PEER).unwrap(), 23);
        assert_eq!(phone.sync_gap(PEER).unwrap(), Some((0, 11)));

        drain(&conn, &key, &phone, pace(5), 10).await;
        for seq in 1..=23 {
            assert_eq!(capture_count(&phone, seq), 1, "capture {seq}");
        }
    }

    #[tokio::test]
    async fn a_caught_up_phone_and_a_mac_never_open_a_gap() {
        let (_dir, conn, key) = relay().await;
        seed_captures(&conn, &key, 1..=12).await;
        // A Mac pulls ascending, as before.
        let mac = Store::open_in_memory().unwrap();
        pull_content_with_budget(
            &conn,
            &key,
            &mac,
            "mac",
            pace(5),
            &PullOptions::default(),
            false,
        )
        .await
        .unwrap();
        assert_eq!(mac.get_sync_cursor(PEER).unwrap(), 5);
        assert_eq!(mac.sync_gap(PEER).unwrap(), None);
        pull_content(&conn, &key, &mac, "mac").await.unwrap();
        assert_eq!(mac.get_sync_cursor(PEER).unwrap(), 12);
        assert_eq!(mac.sync_gap(PEER).unwrap(), None);

        // A phone whose cursor is a few rows behind finishes ascending: no gap.
        let phone = Store::open_in_memory().unwrap();
        phone.set_sync_cursor(PEER, 9).unwrap();
        assert!(!phone_round(&conn, &key, &phone, pace(5)).await);
        assert_eq!(phone.get_sync_cursor(PEER).unwrap(), 12);
        assert_eq!(phone.sync_gap(PEER).unwrap(), None);
        assert!(!phone.older_sync_history_loading().unwrap());
    }

    #[tokio::test]
    async fn the_round_stats_say_older_history_is_loading() {
        let (_dir, conn, key) = relay().await;
        seed_captures(&conn, &key, 1..=8).await;
        let phone = Store::open_in_memory().unwrap();
        let mut stats = Vec::new();
        let more = pull_round_mode(
            &conn,
            &key,
            &phone,
            "phone",
            pace(3),
            &PullOptions::default(),
            true,
            true,
            &mut stats,
        )
        .await
        .unwrap();
        assert!(more);
        let s = &stats[0];
        assert!(s.gap_opened);
        assert_eq!((s.cursor_before, s.gap_fetched, s.gap_hi), (8, 3, Some(6)));
        assert_eq!(s.gap_lo, Some(0));
        let json = serde_json::to_string(&stats).unwrap();
        assert!(json.contains("\"gap_hi\":6"), "{json}");
    }

    #[test]
    fn a_gap_page_is_sized_to_the_budget_left() {
        let s = std::time::Duration::from_secs;
        assert_eq!(gap_page_rows(CATCH_UP_PACE, s(10)), 2000);
        assert_eq!(
            gap_page_rows(CATCH_UP_PACE, s(20)),
            2000,
            "never more than a page"
        );
        assert_eq!(gap_page_rows(CATCH_UP_PACE, s(5)), 1000);
        assert_eq!(gap_page_rows(CATCH_UP_PACE, s(1)), 200);
        assert_eq!(
            gap_page_rows(CATCH_UP_PACE, std::time::Duration::from_millis(999)),
            1
        );
        assert_eq!(gap_page_rows(NORMAL_PACE, s(3)), 500);
        assert_eq!(
            gap_page_rows(budget_only(std::time::Duration::ZERO), s(5)),
            1
        );
    }

    #[tokio::test]
    async fn the_gap_queries_walk_the_primary_key_backward() {
        let (_dir, conn, _key) = relay().await;
        for sql in [
            FETCH_GAP_ROWS_SQL,
            FETCH_GAP_ROWS_NO_SKINS_SQL,
            PEER_TOP_SEQ_SQL,
        ] {
            let mut rows = conn
                .query(&format!("EXPLAIN QUERY PLAN {sql}"), ())
                .await
                .unwrap();
            let mut plan = Vec::new();
            while let Some(row) = rows.next().await.unwrap() {
                plan.push(row.get::<String>(3).unwrap());
            }
            let plan = plan.join(" | ");
            assert!(plan.contains("SEARCH sync_row"), "{sql}: {plan}");
            assert!(
                !plan.contains("SCAN") && !plan.contains("TEMP B-TREE"),
                "{sql}: {plan}"
            );
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
        let db = libsql::Builder::new_local(dir.path().join("relay.db"))
            .build()
            .await
            .unwrap();
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
            palettes: vec![(
                format!("{id}-p1"),
                serde_json::json!({ "id": format!("{id}-p1"), "name": "Navy" }),
            )],
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
            .query(
                "SELECT COUNT(*) FROM sync_row WHERE row_type = ?1",
                params![row_type],
            )
            .await
            .unwrap();
        rows.next().await.unwrap().unwrap().get(0).unwrap()
    }

    #[tokio::test]
    async fn skin_row_roundtrips() {
        let (_dir, conn, key) = relay().await;
        let (a, b) = (device("device-a"), device("device-b"));
        let bytes: Vec<u8> = (0..SKIN_CHUNK_BYTES + 77)
            .map(|i| (i % 253) as u8)
            .collect();
        let payload = save_on_a(&a, &bytes);
        crate::outbox::dispatch_outbox(&conn, &key, &a, "device-a")
            .await
            .unwrap();
        pull_content(&conn, &key, &b, "device-b").await.unwrap();
        assert_eq!(b.synced_skins_ready().unwrap(), vec![payload.clone()]);
        assert_eq!(
            b.synced_image(&payload.images[0].1).unwrap().as_deref(),
            Some(&bytes[..])
        );
        // Pulled again: nothing new, and B writes nothing to the relay.
        pull_content(&conn, &key, &b, "device-b").await.unwrap();
        assert_eq!(b.synced_skins_ready().unwrap().len(), 1);
        let mut rows = conn
            .query(
                "SELECT COUNT(*) FROM sync_row WHERE device_id = 'device-b'",
                (),
            )
            .await
            .unwrap();
        assert_eq!(
            rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(),
            0
        );
    }

    /// The puller's LWW rule, across devices (plan ruling 7): compaction keeps each device's
    /// last-enqueued version, and one device only ever enqueues its current state. B pulls
    /// device-a before device-c, so the older version arrives last and must still lose.
    #[tokio::test]
    async fn older_skin_loses_to_newer() {
        let (_dir, conn, key) = relay().await;
        let (a, b, c) = (device("device-a"), device("device-b"), device("device-c"));
        a.enqueue_skin_row(&skin("user-x-1", 200, vec![])).unwrap();
        c.enqueue_skin_row(&SkinSyncPayload {
            device_id: "device-c".into(),
            ..skin("user-x-1", 100, vec![])
        })
        .unwrap();
        crate::outbox::dispatch_outbox(&conn, &key, &a, "device-a")
            .await
            .unwrap();
        crate::outbox::dispatch_outbox(&conn, &key, &c, "device-c")
            .await
            .unwrap();
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
        a.enqueue_skin_row(&SkinSyncPayload {
            deleted: true,
            updated_at_unix_ms: 200,
            ..saved
        })
        .unwrap();

        crate::outbox::dispatch_outbox(&conn, &GroupKey::generate(), &a, "device-a")
            .await
            .unwrap();

        assert_eq!(rows_of(&conn, ROW_TYPE_SKIN).await, 1);
        let newest = a
            .list_outbox_operations()
            .unwrap()
            .into_iter()
            .filter(|r| r.1 == ROW_TYPE_SKIN)
            .map(|r| r.0)
            .max();
        let mut rows = conn
            .query("SELECT local_seq FROM sync_row WHERE row_type = 'skin'", ())
            .await
            .unwrap();
        assert_eq!(
            rows.next().await.unwrap().unwrap().get::<i64>(0).ok(),
            newest
        );
    }

    #[tokio::test]
    async fn image_sent_once_for_two_skins() {
        let (_dir, conn, key) = relay().await;
        let a = device("device-a");
        let bytes = vec![9u8; 1000];
        let sha = sha256_hex(&bytes);
        for id in ["user-a-1", "user-b-1"] {
            a.enqueue_skin_chunks(&sha, &bytes).unwrap();
            a.enqueue_skin_row(&skin(id, 1, vec![("same.png".into(), sha.clone())]))
                .unwrap();
        }
        crate::outbox::dispatch_outbox(&conn, &key, &a, "device-a")
            .await
            .unwrap();
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
        let bytes: Vec<u8> = (0..SKIN_CHUNK_BYTES * 2 + 10)
            .map(|i| (i % 241) as u8)
            .collect();
        let payload = save_on_a(&a, &bytes);
        crate::outbox::dispatch_outbox(&conn, &key, &a, "device-a")
            .await
            .unwrap();
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
        let bytes: Vec<u8> = (0..SKIN_CHUNK_BYTES * 30)
            .map(|i| (i * 7 % 256) as u8)
            .collect();
        let payload = save_on_a(&a, &bytes);
        let t = crate::now_unix_ms() + 1;
        let first = crate::outbox::dispatch_outbox_at(&conn, &key, &a, "device-a", t)
            .await
            .unwrap();
        let sent = rows_of(&conn, ROW_TYPE_SKIN_IMAGE_CHUNK).await;
        assert!(sent > 0 && sent < 30, "{sent} chunks in one pass");
        assert!(first.deferred > 0);
        let mut at = t;
        while rows_of(&conn, ROW_TYPE_SKIN_IMAGE_CHUNK).await < 30 {
            at += 2;
            crate::outbox::dispatch_outbox_at(&conn, &key, &a, "device-a", at)
                .await
                .unwrap();
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
        assert_eq!(
            b.synced_image(&payload.images[0].1)
                .unwrap()
                .map(|b| b.len()),
            Some(bytes.len())
        );
    }

    #[tokio::test]
    async fn capability_marker_triggers_full_repush() {
        let (_dir, conn, key) = relay().await;
        let (a, b) = (device("device-a"), device("device-b"));
        push_content(&conn, &key, &a, "device-a").await.unwrap();
        pull_content(&conn, &key, &b, "device-b").await.unwrap();
        assert!(
            b.take_skin_repush_request().unwrap(),
            "a newly seen peer asks for every skin"
        );
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
        crate::outbox::dispatch_outbox(&conn, &key, &a, "device-a")
            .await
            .unwrap();
        let phone = device("phone");
        crate::pull_only_with_connection(&conn, &key, &phone, "phone", &PullOptions::default())
            .await
            .unwrap();
        assert!(phone.synced_skins_ready().unwrap().is_empty());
        assert!(
            !phone.take_skin_repush_request().unwrap(),
            "the phone ignores the skin marker too"
        );
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
        crate::outbox::dispatch_outbox(&conn, &key, &a, "device-a")
            .await
            .unwrap();
        // B, on its old build, pulled past everything A had sent.
        let mut rows = conn
            .query(
                "SELECT MAX(local_seq) FROM sync_row WHERE device_id = 'device-a'",
                (),
            )
            .await
            .unwrap();
        let sent_through: i64 = rows.next().await.unwrap().unwrap().get(0).unwrap();
        b.set_sync_cursor("device-a", sent_through).unwrap();
        // B updates: it pushes its marker, and A sees it.
        push_content(&conn, &key, &b, "device-b").await.unwrap();
        pull_content(&conn, &key, &a, "device-a").await.unwrap();
        let sha = payload.images[0].1.clone();
        assert!(
            !a.enqueue_skin_chunks(&sha, &[7, 7, 7]).unwrap(),
            "sent once already"
        );
        assert!(a.take_skin_repush_request().unwrap());
        // A enqueues every skin again, as core/skins' enqueue_all will: the chunks, then the row.
        assert!(
            a.enqueue_skin_chunks(&sha, &[7, 7, 7]).unwrap(),
            "resent for the repush"
        );
        a.enqueue_skin_row(&payload).unwrap();
        crate::outbox::dispatch_outbox(&conn, &key, &a, "device-a")
            .await
            .unwrap();
        pull_content(&conn, &key, &b, "device-b").await.unwrap();
        assert_eq!(b.synced_skins_ready().unwrap(), vec![payload]);
    }

    /// A large image waits behind its 4 MB budget, but rows enqueued after it still go in the same pass.
    #[tokio::test]
    async fn a_large_image_does_not_hold_up_later_rows() {
        let (_dir, conn, key) = relay().await;
        let a = device("device-a");
        let bytes: Vec<u8> = (0..SKIN_CHUNK_BYTES * 150)
            .map(|i| (i * 13 % 256) as u8)
            .collect();
        save_on_a(&a, &bytes);
        a.enqueue_skin_row(&skin("user-later-1", 5, vec![]))
            .unwrap();
        crate::outbox::dispatch_outbox(&conn, &key, &a, "device-a")
            .await
            .unwrap();
        let mut rows = conn
            .query("SELECT COUNT(*) FROM sync_row WHERE row_type = 'skin' AND local_seq = (SELECT MAX(local_seq) FROM sync_row)", ())
            .await
            .unwrap();
        assert_eq!(
            rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap(),
            1,
            "the later row went out"
        );
        assert!(rows_of(&conn, ROW_TYPE_SKIN_IMAGE_CHUNK).await < 150);
    }

    #[test]
    fn the_skin_row_types_match_the_phone_query() {
        let sql = "AND row_type NOT IN ('skin', 'skin_image_chunk')";
        assert_eq!(
            sql,
            format!("AND row_type NOT IN ('{ROW_TYPE_SKIN}', '{ROW_TYPE_SKIN_IMAGE_CHUNK}')")
        );
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
            a.enqueue_skin_row(&skin("user-filler-1", 1, vec![]))
                .unwrap();
        }
        save_on_a(&a, &[4, 5, 6]);
        crate::outbox::dispatch_outbox(&conn, &key, &a, "device-a")
            .await
            .unwrap();
        let b = device("device-b");
        pull_content(&conn, &key, &b, "device-b").await.unwrap();
        let ready: Vec<String> = b
            .synced_skins_ready()
            .unwrap()
            .into_iter()
            .map(|p| p.skin_id)
            .collect();
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
        let db = libsql::Builder::new_local(dir.path().join("relay.db"))
            .build()
            .await
            .unwrap();
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

    pub(super) async fn assert_seeks(conn: &Connection, sql: &str) {
        let mut rows = conn
            .query(&format!("EXPLAIN QUERY PLAN {sql}"), ())
            .await
            .unwrap();
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
        for sql in [
            NEXT_PEER_SQL,
            PEER_MARKER_SQL,
            FETCH_ROWS_SQL,
            FETCH_ROWS_NO_SKINS_SQL,
            OWN_PROVIDER_SPEND_SQL,
            CATCH_UP_SQL,
            CATCH_UP_END_SQL,
            CLIENT_SEQ_DELETE_SQL,
            CLIENT_TAG_DELETE_SQL,
        ] {
            assert_seeks(&conn, sql).await;
        }
        assert_seeks(&conn, &compaction_delete_sql(3)).await;
        assert_seeks(&conn, &forgotten_rows_delete_sql(3)).await;
        assert_seeks(&conn, FORGOTTEN_MEETING_TAG_DELETE_SQL).await;
        assert_seeks(&conn, &fetch_allowed_rows_sql(3, 0, 0)).await;
        assert_seeks(&conn, &fetch_allowed_gap_rows_sql(3, 0, 0)).await;
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

/// Client deletes: a pulled row of a forgotten item is dropped at apply.
#[cfg(test)]
mod client_delete_tests {
    use super::*;

    fn capture_row(text: &str) -> Vec<u8> {
        serde_json::to_vec(&memory::AppliedCaptureItem {
            ts_unix_ms: 1_000,
            app_bundle_id: "com.apple.Safari".into(),
            window_title: "w".into(),
            url: None,
            text: text.into(),
            hash: 1,
            source: Some("ax".into()),
        })
        .unwrap()
    }

    /// A legacy `capture_item` row (from a peer still pushing the pre-cutover format) of a forgotten capture.
    #[tokio::test]
    async fn a_forgotten_legacy_capture_row_is_dropped() {
        // libsql configures SQLite first in every test process, so it must open before the store does.
        let dir = tempfile::tempdir().unwrap();
        let _relay = libsql::Builder::new_local(dir.path().join("relay.db"))
            .build()
            .await
            .unwrap();
        let store = Store::open_in_memory().unwrap();
        store
            .apply_client_item_forgotten(&memory::ClientItemForgotten {
                kind: "capture".into(),
                opaque_id: "dev-x:5".into(),
                forgotten_at_unix_ms: 1,
            })
            .unwrap();
        apply_parent_row(
            &store,
            "dev-x",
            5,
            ROW_TYPE_CAPTURE_ITEM,
            &capture_row("forgotten"),
        )
        .unwrap();
        apply_parent_row(
            &store,
            "dev-x",
            6,
            ROW_TYPE_CAPTURE_ITEM,
            &capture_row("kept"),
        )
        .unwrap();
        let bodies: Vec<String> = store
            .export_items_page("capture", 0, 10)
            .unwrap()
            .into_iter()
            .map(|r| r.body_text)
            .collect();
        assert_eq!(bodies, vec!["kept".to_string()]);
        assert_eq!(
            store
                .resolve_sync_row_map("dev-x", 5, ROW_TYPE_CAPTURE_ITEM)
                .unwrap(),
            None
        );
    }

    /// A meeting brief pulled for a calendar event a client delete forgot is dropped.
    #[tokio::test]
    async fn a_forgotten_events_meeting_prep_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let _relay = libsql::Builder::new_local(dir.path().join("relay.db"))
            .build()
            .await
            .unwrap();
        let store = Store::open_in_memory().unwrap();
        // As in production: the item is one occurrence, `{event id}#{start secs}`; the brief `{event id}@{start ms}`.
        let uid = "kickoff-1@zorblax.com";
        let occurrence = format!("{uid}#2");
        store
            .upsert_calendar_event_items(&[memory::NewCalendarEventItem {
                device_id: "dev-x".into(),
                source: "google_calendar".into(),
                external_id: occurrence.clone(),
                ts_unix_ms: 2_000,
                title: "t".into(),
                actors: vec![],
            }])
            .unwrap();
        store
            .apply_client_item_forgotten(&memory::ClientItemForgotten {
                kind: "calendar_event".into(),
                opaque_id: memory::client_forget::hashed(&format!("google_calendar:{occurrence}")),
                forgotten_at_unix_ms: 1,
            })
            .unwrap();
        let prep = |event_id: &str| {
            serde_json::to_vec(&memory::prep::MeetingPrepRow {
                event_id: event_id.into(),
                title: "t".into(),
                start_unix_ms: 2_000,
                generated_at_unix_ms: 1,
                brief_json: "{}".into(),
                provider: "p".into(),
                model: "m".into(),
            })
            .unwrap()
        };
        apply_parent_row(
            &store,
            "dev-x",
            7,
            ROW_TYPE_MEETING_PREP,
            &prep(&format!("{uid}@2000")),
        )
        .unwrap();
        apply_parent_row(
            &store,
            "dev-x",
            8,
            ROW_TYPE_MEETING_PREP,
            &prep("other@2000"),
        )
        .unwrap();
        assert!(store
            .get_meeting_prep(&format!("{uid}@2000"))
            .unwrap()
            .is_none());
        assert!(store.get_meeting_prep("other@2000").unwrap().is_some());
    }
}

#[cfg(test)]
mod pull_only_marker_guard {
    use super::*;

    /// Every marker a desktop gates rows on (`all_peers_have_marker`, `all_peers_items_capable`) must be one the
    /// phone announces: an updated phone is a peer, and a gate it never satisfies holds those rows back on every
    /// paired desktop (for good, once a phone is retired). A new gate fails here until it is added to
    /// `PULL_ONLY_MARKERS` (when the phone applies those rows) or listed below with the reason it is not.
    #[test]
    fn every_gating_marker_is_announced_by_the_phone() {
        let known: &[(&str, &str)] = &[
            ("ROW_TYPE_ITEMS_CAPABILITY", ROW_TYPE_ITEMS_CAPABILITY),
            ("ROW_TYPE_TOPICS_CAPABILITY", ROW_TYPE_TOPICS_CAPABILITY),
            (
                "ROW_TYPE_TOPIC_PAGE_CAPABILITY",
                ROW_TYPE_TOPIC_PAGE_CAPABILITY,
            ),
            (
                "ROW_TYPE_TOPIC_SNAPSHOT_CAPABILITY",
                ROW_TYPE_TOPIC_SNAPSHOT_CAPABILITY,
            ),
            (
                "ROW_TYPE_CLIENT_CONTROLS_CAPABILITY",
                ROW_TYPE_CLIENT_CONTROLS_CAPABILITY,
            ),
        ];
        let mut gated: Vec<String> = Vec::new();
        // The needles are built at run time, so this test's own text never matches them (X6 final review,
        // Minor 14: a literal here made the items gate found whatever the code did).
        let call = |name: &str| format!("{name}{}", "(");
        // A call (`conn,`), never the definition (`conn:`).
        let items_gate = format!("{}{}", call("all_peers_items_capable"), "conn,");
        for source in [include_str!("outbox.rs"), include_str!("content.rs")] {
            for gate in ["all_peers_have_marker", "peers_without_marker"] {
                for piece in source.split(call(gate).as_str()).skip(1) {
                    let args: Vec<&str> = piece
                        .split(')')
                        .next()
                        .unwrap_or("")
                        .split(',')
                        .map(str::trim)
                        .collect();
                    // The definition's parameters, or a test's string literal, are not a gate in the code.
                    if let Some(marker) = args.get(2).filter(|a| a.starts_with("ROW_TYPE_")) {
                        gated.push(marker.to_string());
                    }
                }
            }
            if source.contains(items_gate.as_str()) {
                gated.push("ROW_TYPE_ITEMS_CAPABILITY".into());
            }
        }
        assert!(gated.len() >= 4, "the scan found the gates: {gated:?}");
        for expected in ["ROW_TYPE_ITEMS_CAPABILITY", "ROW_TYPE_TOPICS_CAPABILITY"] {
            assert!(
                gated.iter().any(|g| g == expected),
                "the scan found the {expected} gate in the code: {gated:?}"
            );
        }
        for name in gated {
            let value = known.iter().find(|(n, _)| *n == name).map(|(_, v)| *v).unwrap_or_else(|| {
                panic!("a new gating marker {name}: add it to PULL_ONLY_MARKERS if the phone applies those rows, or the phone holds them back")
            });
            assert!(
                PULL_ONLY_MARKERS.contains(&value),
                "{name} gates rows but the phone does not announce it"
            );
        }
    }
}

/// The cloud runner's round (`crate::runner_pull_with_connection` / `crate::runner_dispatch_with_connection`): a
/// fresh scratch store each run, an allowlisted, windowed pull, and only its own results sent.
#[cfg(test)]
mod runner_round_tests {
    use super::*;

    const PEER: &str = "dev-a";
    const RUNNER: &str = "runner-1";
    const DAY_MS: i64 = 24 * 60 * 60 * 1000;

    async fn relay() -> (tempfile::TempDir, Connection, GroupKey) {
        let dir = tempfile::tempdir().unwrap();
        let db = libsql::Builder::new_local(dir.path().join("relay.db"))
            .build()
            .await
            .unwrap();
        let conn = db.connect().unwrap();
        ensure_content_schema(&conn).await.unwrap();
        (dir, conn, GroupKey::generate())
    }

    async fn seed(conn: &Connection, key: &GroupKey, seq: i64, row_type: &str, json: &str) {
        let aad = content_row_aad(PEER, seq, row_type);
        let ciphertext = sync::encrypt_payload(key, json.as_bytes(), &aad);
        conn.execute(
            "INSERT INTO sync_row (device_id, local_seq, row_type, ciphertext, created_at_unix_ms) VALUES (?1, ?2, ?3, ?4, 0)",
            params![PEER, seq, row_type, STANDARD.encode(&ciphertext).as_str()],
        )
        .await
        .unwrap();
    }

    fn item_json(
        kind: &str,
        ts_unix_ms: i64,
        body: &str,
        source_meeting_id: Option<i64>,
    ) -> String {
        let calendar = kind == "calendar_event";
        serde_json::to_string(&ItemPayload {
            source_meeting_id,
            meeting_local_seq: None,
            item: memory::AppliedItem {
                kind: kind.to_string(),
                source: calendar.then(|| "google_calendar".to_string()),
                ts_unix_ms,
                actors: Some("[\"Me\"]".to_string()),
                title: calendar.then(|| body.to_string()),
                body_text: body.to_string(),
                refs: None,
                flags: None,
                hash: None,
                external_id: calendar.then(|| format!("{body}#1")),
                app_bundle_id: (kind == "capture").then(|| "com.a".to_string()),
                url: None,
            },
        })
        .unwrap()
    }

    fn meeting_json(source_id: i64, title: &str) -> String {
        serde_json::to_string(&MeetingPayload {
            source_meeting_id: Some(source_id),
            origin_epoch: Some("e1".to_string()),
            meeting: memory::AppliedMeeting {
                attendee_details: vec![],
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
                trimmed_keep_ms: None,
            },
        })
        .unwrap()
    }

    fn run_json(source_run_id: i64, summary: &str) -> String {
        serde_json::to_string(&memory::AppliedRoutineRun {
            source_run_id,
            template: "daily_brief".into(),
            ran_at_unix_ms: 9_000,
            status: memory::RunStatus::Success,
            output_summary: summary.into(),
            output_json: "{}".into(),
        })
        .unwrap()
    }

    fn forgotten_capture_json(seq: i64) -> String {
        let (kind, opaque_id) = memory::client_forget::pulled_capture_identity(PEER, seq);
        serde_json::to_string(&memory::ClientItemForgotten {
            kind: kind.to_string(),
            opaque_id,
            forgotten_at_unix_ms: 1,
        })
        .unwrap()
    }

    fn allow(types: &[&str]) -> PullOptions {
        PullOptions {
            row_type_allowlist: Some(types.iter().map(|t| t.to_string()).collect()),
            ..Default::default()
        }
    }

    fn runner_store() -> Store {
        let store = Store::open_in_memory().unwrap();
        store.set_own_device_id(RUNNER);
        store
    }

    async fn pull(conn: &Connection, key: &GroupKey, store: &Store, opts: &PullOptions) {
        crate::runner_pull_with_connection(conn, key, store, RUNNER, opts)
            .await
            .unwrap();
    }

    fn bodies(store: &Store, kind: &str) -> Vec<String> {
        store
            .export_items_page(kind, 0, 100)
            .unwrap()
            .into_iter()
            .map(|r| r.body_text)
            .collect()
    }

    fn titles(store: &Store) -> Vec<String> {
        store
            .recent_meetings(100)
            .unwrap()
            .into_iter()
            .map(|m| m.title)
            .collect()
    }

    async fn relay_rows(conn: &Connection, device: &str) -> Vec<(i64, String)> {
        let mut rows = conn
            .query(
                "SELECT local_seq, row_type FROM sync_row WHERE device_id = ?1 ORDER BY local_seq, row_type",
                params![device],
            )
            .await
            .unwrap();
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.unwrap() {
            out.push((row.get(0).unwrap(), row.get(1).unwrap()));
        }
        out
    }

    #[tokio::test]
    async fn runner_round_pulls_only_allowlisted_row_types() {
        let (_dir, conn, key) = relay().await;
        let now = now_unix_ms();
        seed(
            &conn,
            &key,
            1,
            ROW_TYPE_MEETING,
            &meeting_json(7, "Standup"),
        )
        .await;
        seed(
            &conn,
            &key,
            2,
            ROW_TYPE_ROUTINE_RUN,
            &run_json(4, "a brief"),
        )
        .await;
        seed(
            &conn,
            &key,
            3,
            ROW_TYPE_ITEMS,
            &item_json("capture", now, "a capture", None),
        )
        .await;
        seed(
            &conn,
            &key,
            4,
            ROW_TYPE_ITEMS,
            &item_json("calendar_event", now, "planning", None),
        )
        .await;

        let runs_only = runner_store();
        pull(&conn, &key, &runs_only, &allow(&[ROW_TYPE_ROUTINE_RUN])).await;
        let runs = runs_only.recent_routine_runs_across_all(10).unwrap();
        assert_eq!(runs.len(), 1);
        assert!(titles(&runs_only).is_empty(), "meeting not granted");
        assert!(
            bodies(&runs_only, "capture").is_empty(),
            "items not granted"
        );
        assert!(
            bodies(&runs_only, "calendar_event").is_empty(),
            "items not granted"
        );

        // An item kind grant takes that kind of `items` row only.
        let events_only = runner_store();
        pull(&conn, &key, &events_only, &allow(&["items:calendar_event"])).await;
        assert_eq!(
            bodies(&events_only, "calendar_event"),
            vec!["planning".to_string()]
        );
        assert!(
            bodies(&events_only, "capture").is_empty(),
            "capture kind not granted"
        );
        assert!(events_only
            .recent_routine_runs_across_all(10)
            .unwrap()
            .is_empty());
        assert!(titles(&events_only).is_empty());

        // No allowlist: everything, as today.
        let all = runner_store();
        pull(&conn, &key, &all, &PullOptions::default()).await;
        assert_eq!(titles(&all), vec!["Standup".to_string()]);
        assert_eq!(bodies(&all, "capture"), vec!["a capture".to_string()]);
        assert_eq!(all.recent_routine_runs_across_all(10).unwrap().len(), 1);
    }

    /// Run a round, then the tombstone arrives, then a second round on the same store: the tombstone
    /// comes through the allowlisted pull itself (the once-per-database catch-up has already run).
    #[tokio::test]
    async fn tombstones_are_applied_even_when_not_allowlisted() {
        let (_dir, conn, key) = relay().await;
        let now = now_unix_ms();
        seed(
            &conn,
            &key,
            1,
            ROW_TYPE_ITEMS,
            &item_json("capture", now, "forget me", None),
        )
        .await;
        seed(&conn, &key, 2, ROW_TYPE_MEETING, &meeting_json(7, "Doomed")).await;
        let store = runner_store();
        let opts = allow(&["items:capture", ROW_TYPE_MEETING]);
        pull(&conn, &key, &store, &opts).await;
        assert_eq!(bodies(&store, "capture"), vec!["forget me".to_string()]);
        assert_eq!(titles(&store), vec!["Doomed".to_string()]);

        seed(
            &conn,
            &key,
            3,
            ROW_TYPE_CLIENT_ITEM_FORGOTTEN,
            &forgotten_capture_json(1),
        )
        .await;
        let meeting_forgotten = serde_json::to_string(&memory::MeetingForgotten {
            origin_device_id: PEER.to_string(),
            origin_epoch: Some("e1".to_string()),
            source_meeting_id: 7,
            forgotten_at_unix_ms: 1,
            source_start_unix_ms: Some(1000),
        })
        .unwrap();
        seed(
            &conn,
            &key,
            4,
            ROW_TYPE_MEETING_FORGOTTEN,
            &meeting_forgotten,
        )
        .await;
        pull(&conn, &key, &store, &opts).await;
        assert!(
            bodies(&store, "capture").is_empty(),
            "the client delete applied"
        );
        assert!(titles(&store).is_empty(), "the meeting forget applied");
    }

    /// The transcript rows carry no `meeting_local_seq`, so only the allowlist can bring the meeting.
    #[tokio::test]
    async fn an_allowed_transcript_pulls_its_meeting() {
        let (_dir, conn, key) = relay().await;
        let now = now_unix_ms();
        seed(
            &conn,
            &key,
            1,
            ROW_TYPE_MEETING,
            &meeting_json(7, "Design review"),
        )
        .await;
        seed(
            &conn,
            &key,
            2,
            ROW_TYPE_ITEMS,
            &item_json("transcript", now, "first line", Some(7)),
        )
        .await;
        seed(
            &conn,
            &key,
            3,
            ROW_TYPE_ITEMS,
            &item_json("transcript", now, "second line", Some(7)),
        )
        .await;
        let store = runner_store();
        pull(&conn, &key, &store, &allow(&["items:transcript"])).await;
        assert_eq!(titles(&store), vec!["Design review".to_string()]);
        let mut lines = bodies(&store, "transcript");
        lines.sort();
        assert_eq!(
            lines,
            vec!["first line".to_string(), "second line".to_string()]
        );
    }

    #[tokio::test]
    async fn the_five_pull_only_markers_are_announced() {
        let (_dir, conn, key) = relay().await;
        seed(
            &conn,
            &key,
            1,
            ROW_TYPE_ROUTINE_RUN,
            &run_json(4, "a brief"),
        )
        .await;
        let store = runner_store();
        pull(&conn, &key, &store, &allow(&[ROW_TYPE_ROUTINE_RUN])).await;
        let mut announced: Vec<String> = relay_rows(&conn, RUNNER)
            .await
            .into_iter()
            .map(|(seq, t)| {
                assert_eq!(seq, 0, "the pull writes markers only");
                t
            })
            .collect();
        announced.sort();
        let mut expected: Vec<String> = PULL_ONLY_MARKERS.iter().map(|m| m.to_string()).collect();
        expected.sort();
        assert_eq!(announced, expected);
    }

    #[tokio::test]
    async fn a_routine_run_in_the_outbox_is_dispatched_and_no_content_is_back_filled() {
        let (_dir, conn, key) = relay().await;
        let now = now_unix_ms();
        seed(
            &conn,
            &key,
            1,
            ROW_TYPE_MEETING,
            &meeting_json(7, "Standup"),
        )
        .await;
        seed(
            &conn,
            &key,
            2,
            ROW_TYPE_ITEMS,
            &item_json("capture", now, "a capture", None),
        )
        .await;
        seed(
            &conn,
            &key,
            3,
            ROW_TYPE_ROUTINE_RUN,
            &run_json(4, "peer brief"),
        )
        .await;
        let store = runner_store();
        store.seed_builtin_routines().unwrap();
        pull(&conn, &key, &store, &PullOptions::default()).await;
        assert_eq!(titles(&store), vec!["Standup".to_string()]);
        let brief = store
            .list_routines()
            .unwrap()
            .into_iter()
            .find(|r| r.template == memory::RoutineTemplate::DailyBrief)
            .unwrap();
        store
            .insert_routine_run(&memory::NewRoutineRun {
                routine_id: brief.id,
                ran_at_unix_ms: now,
                status: memory::RunStatus::Success,
                output_summary: "cloud brief".into(),
                output_json: "{}".into(),
            })
            .unwrap();

        crate::runner_dispatch_with_connection(&conn, &key, &store, RUNNER)
            .await
            .unwrap();

        let content: Vec<String> = relay_rows(&conn, RUNNER)
            .await
            .into_iter()
            .filter(|(seq, _)| *seq != 0)
            .map(|(_, t)| t)
            .collect();
        assert_eq!(
            content,
            vec![ROW_TYPE_ROUTINE_RUN.to_string()],
            "only the run is sent, nothing pulled is re-pushed"
        );

        let desktop = Store::open_in_memory().unwrap();
        pull_content(&conn, &key, &desktop, "desktop")
            .await
            .unwrap();
        let summaries: Vec<String> = desktop
            .recent_routine_runs_across_all(10)
            .unwrap()
            .into_iter()
            .map(|r| r.output_summary)
            .collect();
        assert!(
            summaries.contains(&"cloud brief".to_string()),
            "{summaries:?}"
        );
    }

    /// Each run's store is new, so its `local_seq` counter starts over: the second run's result must still
    /// land above the first run's row (the relay insert ignores a key it already has) and reach a peer.
    #[tokio::test]
    async fn a_second_runner_store_sends_above_the_first_runs_rows() {
        let (_dir, conn, key) = relay().await;
        let desktop = Store::open_in_memory().unwrap();
        for (run, summary) in [(1, "monday brief"), (2, "tuesday brief")] {
            let store = runner_store();
            store.seed_builtin_routines().unwrap();
            pull(&conn, &key, &store, &allow(&[ROW_TYPE_ROUTINE_RUN])).await;
            let brief = store
                .list_routines()
                .unwrap()
                .into_iter()
                .find(|r| r.template == memory::RoutineTemplate::DailyBrief)
                .unwrap();
            store
                .insert_routine_run(&memory::NewRoutineRun {
                    routine_id: brief.id,
                    ran_at_unix_ms: run * 1_000,
                    status: memory::RunStatus::Success,
                    output_summary: summary.into(),
                    output_json: "{}".into(),
                })
                .unwrap();
            crate::runner_dispatch_with_connection(&conn, &key, &store, RUNNER)
                .await
                .unwrap();
            pull_content(&conn, &key, &desktop, "desktop")
                .await
                .unwrap();
        }
        let runs = relay_rows(&conn, RUNNER)
            .await
            .into_iter()
            .filter(|(_, t)| t == ROW_TYPE_ROUTINE_RUN)
            .count();
        assert_eq!(runs, 2, "both runs are on the relay");
        let mut summaries: Vec<String> = desktop
            .recent_routine_runs_across_all(10)
            .unwrap()
            .into_iter()
            .map(|r| r.output_summary)
            .collect();
        summaries.sort();
        assert_eq!(
            summaries,
            vec!["monday brief".to_string(), "tuesday brief".to_string()]
        );
    }

    /// Peer runs the pull applies take ids in the runner's store too, so its own run's id can sit above the
    /// relay top it was floored at: the next run must still not reuse it.
    #[tokio::test]
    async fn a_runner_run_id_never_repeats_when_peer_runs_were_pulled() {
        let (_dir, conn, key) = relay().await;
        seed(
            &conn,
            &key,
            1,
            ROW_TYPE_ROUTINE_RUN,
            &run_json(1, "peer one"),
        )
        .await;
        seed(
            &conn,
            &key,
            2,
            ROW_TYPE_ROUTINE_RUN,
            &run_json(2, "peer two"),
        )
        .await;
        let desktop = Store::open_in_memory().unwrap();
        for (grant, summary) in [
            (vec![ROW_TYPE_ROUTINE_RUN], "first runner brief"),
            (vec![], "second runner brief"),
        ] {
            let store = runner_store();
            store.seed_builtin_routines().unwrap();
            pull(&conn, &key, &store, &allow(&grant)).await;
            let brief = store
                .list_routines()
                .unwrap()
                .into_iter()
                .find(|r| r.template == memory::RoutineTemplate::DailyBrief)
                .unwrap();
            store
                .insert_routine_run(&memory::NewRoutineRun {
                    routine_id: brief.id,
                    ran_at_unix_ms: 5_000,
                    status: memory::RunStatus::Success,
                    output_summary: summary.into(),
                    output_json: "{}".into(),
                })
                .unwrap();
            crate::runner_dispatch_with_connection(&conn, &key, &store, RUNNER)
                .await
                .unwrap();
        }
        pull_content(&conn, &key, &desktop, "desktop")
            .await
            .unwrap();
        let mut summaries: Vec<String> = desktop
            .recent_routine_runs_across_all(10)
            .unwrap()
            .into_iter()
            .filter(|r| r.origin_device_id.as_deref() == Some(RUNNER))
            .map(|r| r.output_summary)
            .collect();
        summaries.sort();
        assert_eq!(
            summaries,
            vec![
                "first runner brief".to_string(),
                "second runner brief".to_string()
            ]
        );
    }

    /// The runner's store is new every run, so the once-per-database tombstone catch-up is not redone from
    /// zero (a slow or failing catch-up used to fail every run of a large account): a tombstone comes through
    /// the live ranges, and a catch-up that would fail does not matter.
    #[tokio::test]
    async fn a_new_runner_store_skips_the_tombstone_catch_up_and_still_applies_tombstones() {
        let (_dir, conn, key) = relay().await;
        seed(&conn, &key, 1, ROW_TYPE_MEETING, &meeting_json(7, "Doomed")).await;
        let meeting_forgotten = serde_json::to_string(&memory::MeetingForgotten {
            origin_device_id: PEER.to_string(),
            origin_epoch: Some("e1".to_string()),
            source_meeting_id: 7,
            forgotten_at_unix_ms: 1,
            source_start_unix_ms: Some(1000),
        })
        .unwrap();
        seed(
            &conn,
            &key,
            2,
            ROW_TYPE_MEETING_FORGOTTEN,
            &meeting_forgotten,
        )
        .await;
        let store = runner_store();
        CATCH_UP_FAILS_FOR.with(|c| *c.borrow_mut() = Some(PEER.to_string()));
        let report = crate::runner_pull_with_connection(
            &conn,
            &key,
            &store,
            RUNNER,
            &allow(&[ROW_TYPE_MEETING]),
        )
        .await;
        CATCH_UP_FAILS_FOR.with(|c| *c.borrow_mut() = None);
        let report = report.expect("the pull no longer depends on the catch-up");
        assert!(report.complete, "every row was read");
        assert!(
            titles(&store).is_empty(),
            "the live range applied the forget"
        );
        assert!(tombstones_caught_up(&store));
    }

    /// The runner's store is new each run: its own earlier runs and briefs come back with the pull, so a
    /// second run the same day sees them. Only the granted types.
    #[tokio::test]
    async fn the_runner_reads_back_its_own_earlier_results() {
        let (_dir, conn, key) = relay().await;
        let first = runner_store();
        pull(&conn, &key, &first, &allow(&[ROW_TYPE_ROUTINE_RUN])).await;
        first.seed_builtin_routines().unwrap();
        let brief = first
            .list_routines()
            .unwrap()
            .into_iter()
            .find(|r| r.template == memory::RoutineTemplate::DailyBrief)
            .unwrap();
        first
            .insert_routine_run(&memory::NewRoutineRun {
                routine_id: brief.id,
                ran_at_unix_ms: 5_000,
                status: memory::RunStatus::Success,
                output_summary: "the runner's brief".into(),
                output_json: "{}".into(),
            })
            .unwrap();
        crate::runner_dispatch_with_connection(&conn, &key, &first, RUNNER)
            .await
            .unwrap();

        let second = runner_store();
        let report = crate::runner_pull_with_connection(
            &conn,
            &key,
            &second,
            RUNNER,
            &allow(&[ROW_TYPE_ROUTINE_RUN]),
        )
        .await
        .unwrap();
        assert_eq!(report.rows_applied, 1);
        let runs = second.recent_routine_runs_across_all(10).unwrap();
        assert_eq!(
            runs.iter()
                .map(|r| r.output_summary.as_str())
                .collect::<Vec<_>>(),
            vec!["the runner's brief"]
        );
        assert_eq!(runs[0].origin_device_id.as_deref(), Some(RUNNER));

        let not_granted = runner_store();
        pull(&conn, &key, &not_granted, &allow(&[ROW_TYPE_MEETING])).await;
        assert!(not_granted
            .recent_routine_runs_across_all(10)
            .unwrap()
            .is_empty());
    }

    fn calendar_json(title: &str, source: Option<&str>) -> String {
        let mut payload: ItemPayload = serde_json::from_str(&item_json(
            "calendar_event",
            now_unix_ms() + DAY_MS / 2,
            title,
            None,
        ))
        .unwrap();
        payload.item.source = source.map(str::to_string);
        serde_json::to_string(&payload).unwrap()
    }

    fn calendar_titles(store: &Store) -> Vec<String> {
        let mut titles: Vec<String> = store
            .export_items_page("calendar_event", 0, 100)
            .unwrap()
            .into_iter()
            .map(|r| r.body_text)
            .collect();
        titles.sort();
        titles
    }

    /// The cloud runner (`non_google_calendar_only`) never applies a Google Calendar event, nor one whose
    /// source is not stored (treated as Google); other calendars still come through. Off by default.
    #[tokio::test]
    async fn a_sourceless_calendar_event_is_treated_as_google_and_left_out() {
        let (_dir, conn, key) = relay().await;
        seed(
            &conn,
            &key,
            1,
            ROW_TYPE_ITEMS,
            &calendar_json("google event", Some("google_calendar")),
        )
        .await;
        seed(
            &conn,
            &key,
            2,
            ROW_TYPE_ITEMS,
            &calendar_json("sourceless event", None),
        )
        .await;
        seed(
            &conn,
            &key,
            3,
            ROW_TYPE_ITEMS,
            &calendar_json("empty source event", Some("")),
        )
        .await;
        seed(
            &conn,
            &key,
            4,
            ROW_TYPE_ITEMS,
            &calendar_json("apple event", Some("apple_calendar")),
        )
        .await;
        let runner = runner_store();
        let options = PullOptions {
            non_google_calendar_only: true,
            ..allow(&["items:calendar_event"])
        };
        pull(&conn, &key, &runner, &options).await;
        assert_eq!(calendar_titles(&runner), vec!["apple event".to_string()]);

        let as_today = runner_store();
        pull(&conn, &key, &as_today, &allow(&["items:calendar_event"])).await;
        assert_eq!(
            calendar_titles(&as_today).len(),
            4,
            "off by default: every calendar event, as today"
        );
    }

    /// A device without the topics capability makes every sender hold its client settings back: the report
    /// says so until every device on the relay has announced it.
    #[tokio::test]
    async fn the_report_says_whether_client_settings_are_being_held_back() {
        let (_dir, conn, key) = relay().await;
        seed(
            &conn,
            &key,
            1,
            ROW_TYPE_ROUTINE_RUN,
            &run_json(1, "a brief"),
        )
        .await;
        let report = crate::runner_pull_with_connection(
            &conn,
            &key,
            &runner_store(),
            RUNNER,
            &allow(&[ROW_TYPE_ROUTINE_RUN]),
        )
        .await
        .unwrap();
        assert!(report.complete);
        assert!(
            !report.exclusion_rows_published,
            "the peer has not announced the topics capability"
        );
        push_marker(&conn, &key, PEER, ROW_TYPE_TOPICS_CAPABILITY)
            .await
            .unwrap();
        let report = crate::runner_pull_with_connection(
            &conn,
            &key,
            &runner_store(),
            RUNNER,
            &allow(&[ROW_TYPE_ROUTINE_RUN]),
        )
        .await
        .unwrap();
        assert!(report.exclusion_rows_published);
    }

    /// The runner keys "every exclusion row was read" off a drained pull: it says so, and counts the rows.
    #[tokio::test]
    async fn a_drained_runner_pull_reports_complete_with_the_rows_it_applied() {
        let (_dir, conn, key) = relay().await;
        seed(
            &conn,
            &key,
            1,
            ROW_TYPE_MEETING,
            &meeting_json(7, "Standup"),
        )
        .await;
        seed(
            &conn,
            &key,
            2,
            ROW_TYPE_ROUTINE_RUN,
            &run_json(4, "a brief"),
        )
        .await;
        seed(
            &conn,
            &key,
            3,
            ROW_TYPE_ITEMS,
            &item_json("capture", now_unix_ms(), "not granted", None),
        )
        .await;
        let store = runner_store();
        let report = crate::runner_pull_with_connection(
            &conn,
            &key,
            &store,
            RUNNER,
            &allow(&[ROW_TYPE_MEETING, ROW_TYPE_ROUTINE_RUN]),
        )
        .await
        .unwrap();
        assert!(report.complete);
        assert_eq!(report.rows_applied, 2, "the capture was left out");
        assert!(report.rounds >= 1, "{report:?}");
        assert!(
            report.rows_fetched >= report.rows_applied,
            "fetched counts every row read: {report:?}"
        );
    }

    /// A pull whose budget ends with rows still waiting succeeds (the newest rows are in) but is not
    /// complete: an older client setting may be among the rows not read.
    #[tokio::test]
    async fn a_runner_pull_cut_short_by_its_budget_reports_incomplete() {
        let (_dir, conn, key) = relay().await;
        let rows = CATCH_UP_PACE.rows as i64 + 5;
        for seq in 1..=rows {
            seed(
                &conn,
                &key,
                seq,
                ROW_TYPE_ROUTINE_RUN,
                &run_json(seq, "a brief"),
            )
            .await;
        }
        let store = runner_store();
        let report = crate::runner_pull_phased(
            &conn,
            &key,
            &store,
            RUNNER,
            &allow(&[ROW_TYPE_ROUTINE_RUN]),
            std::time::Duration::ZERO,
            &std::sync::Mutex::new("connect"),
        )
        .await
        .unwrap();
        assert!(!report.complete);
        assert!(
            report.rows_applied > 0 && (report.rows_applied as i64) < rows,
            "{}",
            report.rows_applied
        );
    }

    fn meeting_at(source_id: i64, title: &str, start_unix_ms: i64) -> String {
        let mut payload: MeetingPayload =
            serde_json::from_str(&meeting_json(source_id, title)).unwrap();
        payload.meeting.start_unix_ms = start_unix_ms;
        payload.meeting.end_unix_ms = Some(start_unix_ms + 60_000);
        serde_json::to_string(&payload).unwrap()
    }

    fn windowed(types: &[&str]) -> PullOptions {
        PullOptions {
            item_window_ms: Some(DAY_MS),
            ..allow(types)
        }
    }

    #[tokio::test]
    async fn a_transcript_grant_does_not_pull_meetings_older_than_the_window_that_have_no_in_window_transcript(
    ) {
        let (_dir, conn, key) = relay().await;
        let now = now_unix_ms();
        seed(
            &conn,
            &key,
            1,
            ROW_TYPE_MEETING,
            &meeting_at(7, "Old, no lines", now - 30 * DAY_MS),
        )
        .await;
        seed(
            &conn,
            &key,
            2,
            ROW_TYPE_MEETING,
            &meeting_at(8, "Old, old lines", now - 20 * DAY_MS),
        )
        .await;
        seed(
            &conn,
            &key,
            3,
            ROW_TYPE_ITEMS,
            &item_json("transcript", now - 20 * DAY_MS, "stale line", Some(8)),
        )
        .await;
        seed(
            &conn,
            &key,
            4,
            ROW_TYPE_MEETING,
            &meeting_at(9, "Recent, no lines", now - DAY_MS / 2),
        )
        .await;
        seed(
            &conn,
            &key,
            5,
            ROW_TYPE_MEETING,
            &meeting_at(10, "Today", now - 60_000),
        )
        .await;
        seed(
            &conn,
            &key,
            6,
            ROW_TYPE_ITEMS,
            &item_json("transcript", now, "fresh line", Some(10)),
        )
        .await;
        let store = runner_store();
        pull(&conn, &key, &store, &windowed(&["items:transcript"])).await;
        assert_eq!(
            titles(&store),
            vec!["Today".to_string()],
            "only the meeting an applied line needs"
        );
        assert_eq!(bodies(&store, "transcript"), vec!["fresh line".to_string()]);
    }

    #[tokio::test]
    async fn an_in_window_transcript_still_gets_its_older_started_parent_meeting() {
        let (_dir, conn, key) = relay().await;
        let now = now_unix_ms();
        seed(
            &conn,
            &key,
            1,
            ROW_TYPE_MEETING,
            &meeting_at(7, "Started last week", now - 7 * DAY_MS),
        )
        .await;
        seed(
            &conn,
            &key,
            2,
            ROW_TYPE_ITEMS,
            &item_json("transcript", now, "still talking", Some(7)),
        )
        .await;
        for grant in [
            vec!["items:transcript"],
            vec![ROW_TYPE_MEETING, "items:transcript"],
        ] {
            let store = runner_store();
            pull(&conn, &key, &store, &windowed(&grant)).await;
            assert_eq!(
                titles(&store),
                vec!["Started last week".to_string()],
                "{grant:?}"
            );
            assert_eq!(
                bodies(&store, "transcript"),
                vec!["still talking".to_string()],
                "{grant:?}"
            );
        }
    }

    fn legacy_line_json(
        meeting_local_seq: i64,
        source_meeting_id: i64,
        start_cs: i64,
        text: &str,
    ) -> String {
        serde_json::to_string(&TranscriptSegmentPayload {
            meeting_local_seq,
            source_meeting_id: Some(source_meeting_id),
            segment: memory::AppliedTranscriptSegment {
                start_cs,
                speaker_label: "Alice".to_string(),
                text: text.to_string(),
            },
        })
        .unwrap()
    }

    fn has_line(store: &Store, text: &str) -> bool {
        !store
            .search_transcript_keyword(text, 10)
            .unwrap()
            .is_empty()
    }

    #[tokio::test]
    async fn a_legacy_line_of_a_meeting_older_than_the_window_applies_neither_line_nor_meeting() {
        let (_dir, conn, key) = relay().await;
        let now = now_unix_ms();
        seed(
            &conn,
            &key,
            1,
            ROW_TYPE_MEETING,
            &meeting_at(7, "Twenty days ago", now - 20 * DAY_MS),
        )
        .await;
        seed(
            &conn,
            &key,
            2,
            ROW_TYPE_TRANSCRIPT_SEGMENT,
            &legacy_line_json(1, 7, 100, "stale legacy words"),
        )
        .await;
        let store = runner_store();
        pull(&conn, &key, &store, &windowed(&["items:transcript"])).await;
        assert!(titles(&store).is_empty(), "{:?}", titles(&store));
        assert!(!has_line(&store, "stale"));
        assert!(
            store
                .list_sync_pending_dependencies(PEER)
                .unwrap()
                .is_empty(),
            "dropped for good, not pending"
        );
    }

    #[tokio::test]
    async fn a_legacy_line_inside_the_window_of_an_older_started_meeting_applies_with_its_meeting()
    {
        let (_dir, conn, key) = relay().await;
        let now = now_unix_ms();
        // Started 7 days ago; the line is 6.5 days in, so it is 12 hours old.
        let offset_cs = (6 * DAY_MS + DAY_MS / 2) / 10;
        seed(
            &conn,
            &key,
            1,
            ROW_TYPE_MEETING,
            &meeting_at(7, "Started last week", now - 7 * DAY_MS),
        )
        .await;
        seed(
            &conn,
            &key,
            2,
            ROW_TYPE_TRANSCRIPT_SEGMENT,
            &legacy_line_json(1, 7, 0, "early legacy words"),
        )
        .await;
        seed(
            &conn,
            &key,
            3,
            ROW_TYPE_TRANSCRIPT_SEGMENT,
            &legacy_line_json(1, 7, offset_cs, "late legacy words"),
        )
        .await;
        let store = runner_store();
        pull(&conn, &key, &store, &windowed(&["items:transcript"])).await;
        assert_eq!(titles(&store), vec!["Started last week".to_string()]);
        assert!(has_line(&store, "late"));
        assert!(
            !has_line(&store, "early"),
            "the line before the window is dropped"
        );
    }

    #[tokio::test]
    async fn a_legacy_line_of_a_meeting_started_twelve_hours_ago_applies() {
        let (_dir, conn, key) = relay().await;
        let now = now_unix_ms();
        seed(
            &conn,
            &key,
            1,
            ROW_TYPE_MEETING,
            &meeting_at(7, "This morning", now - DAY_MS / 2),
        )
        .await;
        seed(
            &conn,
            &key,
            2,
            ROW_TYPE_TRANSCRIPT_SEGMENT,
            &legacy_line_json(1, 7, 0, "morning legacy words"),
        )
        .await;
        let store = runner_store();
        pull(&conn, &key, &store, &windowed(&["items:transcript"])).await;
        assert_eq!(titles(&store), vec!["This morning".to_string()]);
        assert!(has_line(&store, "morning"));
    }

    #[tokio::test]
    async fn only_the_meeting_of_an_in_window_legacy_line_is_pulled() {
        let (_dir, conn, key) = relay().await;
        let now = now_unix_ms();
        seed(
            &conn,
            &key,
            1,
            ROW_TYPE_MEETING,
            &meeting_at(7, "Old", now - 20 * DAY_MS),
        )
        .await;
        seed(
            &conn,
            &key,
            2,
            ROW_TYPE_TRANSCRIPT_SEGMENT,
            &legacy_line_json(1, 7, 0, "old one"),
        )
        .await;
        seed(
            &conn,
            &key,
            3,
            ROW_TYPE_TRANSCRIPT_SEGMENT,
            &legacy_line_json(1, 7, 500, "old two"),
        )
        .await;
        seed(
            &conn,
            &key,
            4,
            ROW_TYPE_MEETING,
            &meeting_at(8, "Today", now - 60_000),
        )
        .await;
        seed(
            &conn,
            &key,
            5,
            ROW_TYPE_TRANSCRIPT_SEGMENT,
            &legacy_line_json(4, 8, 0, "fresh one"),
        )
        .await;
        let store = runner_store();
        pull(&conn, &key, &store, &windowed(&["items:transcript"])).await;
        assert_eq!(titles(&store), vec!["Today".to_string()]);
        assert!(has_line(&store, "fresh"));
        assert!(!has_line(&store, "old"));
    }

    #[tokio::test]
    async fn without_a_window_an_old_legacy_line_and_its_meeting_apply_as_before() {
        let (_dir, conn, key) = relay().await;
        let now = now_unix_ms();
        seed(
            &conn,
            &key,
            1,
            ROW_TYPE_MEETING,
            &meeting_at(7, "Twenty days ago", now - 20 * DAY_MS),
        )
        .await;
        seed(
            &conn,
            &key,
            2,
            ROW_TYPE_TRANSCRIPT_SEGMENT,
            &legacy_line_json(1, 7, 100, "ancient legacy words"),
        )
        .await;
        let store = runner_store();
        pull(&conn, &key, &store, &allow(&["items:transcript"])).await;
        assert_eq!(titles(&store), vec!["Twenty days ago".to_string()]);
        assert!(has_line(&store, "ancient"));
    }

    #[tokio::test]
    async fn lines_applied_when_resolving_are_counted_and_a_windowed_out_one_is_skipped_not_deferred(
    ) {
        let (_dir, conn, key) = relay().await;
        let now = now_unix_ms();
        seed(
            &conn,
            &key,
            1,
            ROW_TYPE_MEETING,
            &meeting_at(7, "Today", now - 60_000),
        )
        .await;
        seed(
            &conn,
            &key,
            2,
            ROW_TYPE_TRANSCRIPT_SEGMENT,
            &legacy_line_json(1, 7, 0, "first words"),
        )
        .await;
        seed(
            &conn,
            &key,
            3,
            ROW_TYPE_TRANSCRIPT_SEGMENT,
            &legacy_line_json(1, 7, 10, "second words"),
        )
        .await;
        seed(
            &conn,
            &key,
            4,
            ROW_TYPE_MEETING,
            &meeting_at(8, "Old", now - 20 * DAY_MS),
        )
        .await;
        seed(
            &conn,
            &key,
            5,
            ROW_TYPE_TRANSCRIPT_SEGMENT,
            &legacy_line_json(4, 8, 0, "stale words"),
        )
        .await;
        let store = runner_store();
        let report = crate::runner_pull_with_connection(
            &conn,
            &key,
            &store,
            RUNNER,
            &windowed(&["items:transcript"]),
        )
        .await
        .unwrap();
        let at = |name: &str| {
            report.by_type[crate::PULL_TYPE_NAMES
                .iter()
                .position(|n| *n == name)
                .unwrap()]
        };
        assert_eq!(at("transcript_segment").fetched, 3);
        assert_eq!(at("transcript_segment").applied, 2);
        assert_eq!(report.skipped_window, 1);
        assert_eq!(
            report.deferred, 2,
            "the two lines set aside for their meeting; the dropped one is not among them"
        );
        assert_eq!(
            report.by_type.iter().map(|c| c.applied).sum::<usize>() + report.own_results_applied,
            report.rows_applied
        );
    }

    #[tokio::test]
    async fn an_old_legacy_line_of_a_meeting_applied_earlier_is_dropped() {
        let (_dir, conn, key) = relay().await;
        let now = now_unix_ms();
        seed(
            &conn,
            &key,
            1,
            ROW_TYPE_MEETING,
            &meeting_at(7, "Started last week", now - 7 * DAY_MS),
        )
        .await;
        seed(
            &conn,
            &key,
            2,
            ROW_TYPE_ITEMS,
            &item_json("transcript", now, "live words", Some(7)),
        )
        .await;
        let store = runner_store();
        let opts = windowed(&["items:transcript"]);
        pull(&conn, &key, &store, &opts).await;
        assert_eq!(titles(&store), vec!["Started last week".to_string()]);
        seed(
            &conn,
            &key,
            3,
            ROW_TYPE_TRANSCRIPT_SEGMENT,
            &legacy_line_json(1, 7, 100, "week old legacy words"),
        )
        .await;
        pull(&conn, &key, &store, &opts).await;
        assert!(
            !has_line(&store, "week"),
            "the meeting is local, the line is still before the window"
        );
        assert!(store
            .list_sync_pending_dependencies(PEER)
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn a_windowed_legacy_line_whose_meeting_is_not_on_the_relay_stays_pending() {
        let (_dir, conn, key) = relay().await;
        seed(
            &conn,
            &key,
            2,
            ROW_TYPE_TRANSCRIPT_SEGMENT,
            &legacy_line_json(99, 99, 0, "orphan words"),
        )
        .await;
        let store = runner_store();
        pull(&conn, &key, &store, &windowed(&["items:transcript"])).await;
        assert_eq!(
            store.list_sync_pending_dependencies(PEER).unwrap(),
            vec![(2, ROW_TYPE_TRANSCRIPT_SEGMENT.to_string())]
        );
    }

    #[tokio::test]
    async fn a_pending_legacy_line_whose_meeting_arrives_older_than_the_window_is_dropped_by_the_retry(
    ) {
        let (_dir, conn, key) = relay().await;
        let now = now_unix_ms();
        seed(
            &conn,
            &key,
            2,
            ROW_TYPE_TRANSCRIPT_SEGMENT,
            &legacy_line_json(1, 7, 0, "late arrival words"),
        )
        .await;
        let store = runner_store();
        let opts = windowed(&["items:transcript"]);
        pull(&conn, &key, &store, &opts).await;
        assert_eq!(store.list_sync_pending_dependencies(PEER).unwrap().len(), 1);
        seed(
            &conn,
            &key,
            1,
            ROW_TYPE_MEETING,
            &meeting_at(7, "Twenty days ago", now - 20 * DAY_MS),
        )
        .await;
        pull(&conn, &key, &store, &opts).await;
        assert!(store
            .list_sync_pending_dependencies(PEER)
            .unwrap()
            .is_empty());
        assert!(titles(&store).is_empty());
        assert!(!has_line(&store, "late"));
    }

    #[tokio::test]
    async fn an_in_window_items_line_applied_while_resolving_counts_under_items() {
        let (_dir, conn, key) = relay().await;
        let now = now_unix_ms();
        seed(
            &conn,
            &key,
            1,
            ROW_TYPE_MEETING,
            &meeting_at(7, "Today", now - 60_000),
        )
        .await;
        seed(
            &conn,
            &key,
            2,
            ROW_TYPE_ITEMS,
            &item_json("transcript", now, "fresh line", Some(7)),
        )
        .await;
        let store = runner_store();
        let report = crate::runner_pull_with_connection(
            &conn,
            &key,
            &store,
            RUNNER,
            &windowed(&["items:transcript"]),
        )
        .await
        .unwrap();
        let at = |name: &str| {
            report.by_type[crate::PULL_TYPE_NAMES
                .iter()
                .position(|n| *n == name)
                .unwrap()]
        };
        assert_eq!(at("items").applied, 1);
        assert_eq!(bodies(&store, "transcript"), vec!["fresh line".to_string()]);
    }

    #[tokio::test]
    async fn a_held_old_meeting_is_not_applied_for_an_old_legacy_line_regression_guard() {
        let (_dir, conn, key) = relay().await;
        let now = now_unix_ms();
        seed(
            &conn,
            &key,
            1,
            ROW_TYPE_MEETING,
            &meeting_at(7, "Twenty days ago", now - 20 * DAY_MS),
        )
        .await;
        // Regression guard (it also passes without the fix wave): the line's own meeting_local_seq (50) names no
        // row, so only the held meeting row (seq 1) leads to the meeting.
        seed(
            &conn,
            &key,
            2,
            ROW_TYPE_TRANSCRIPT_SEGMENT,
            &legacy_line_json(50, 7, 0, "held path words"),
        )
        .await;
        let store = runner_store();
        pull(&conn, &key, &store, &windowed(&["items:transcript"])).await;
        assert!(titles(&store).is_empty());
        assert!(!has_line(&store, "held"));
        assert!(
            store
                .list_sync_pending_dependencies(PEER)
                .unwrap()
                .is_empty(),
            "dated by the held meeting, then dropped"
        );
    }

    #[tokio::test]
    async fn a_legacy_line_is_dated_by_the_newest_copy_of_its_meeting() {
        let (_dir, conn, key) = relay().await;
        let now = now_unix_ms();
        seed(
            &conn,
            &key,
            1,
            ROW_TYPE_MEETING,
            &meeting_at(7, "Restarted", now - 20 * DAY_MS),
        )
        .await;
        seed(
            &conn,
            &key,
            2,
            ROW_TYPE_TRANSCRIPT_SEGMENT,
            &legacy_line_json(1, 7, 0, "newest copy words"),
        )
        .await;
        seed(
            &conn,
            &key,
            3,
            ROW_TYPE_MEETING,
            &meeting_at(7, "Restarted", now - 60 * 60 * 1000),
        )
        .await;
        let store = runner_store();
        pull(&conn, &key, &store, &windowed(&["items:transcript"])).await;
        assert!(
            has_line(&store, "newest"),
            "dated by the newer copy, so in the window"
        );
        assert_eq!(
            titles(&store),
            vec!["Restarted".to_string()],
            "the meeting is applied exactly when its line is"
        );
    }

    #[tokio::test]
    async fn an_explicit_meeting_grant_respects_the_window() {
        let (_dir, conn, key) = relay().await;
        let now = now_unix_ms();
        seed(
            &conn,
            &key,
            1,
            ROW_TYPE_MEETING,
            &meeting_at(7, "Last week", now - 7 * DAY_MS),
        )
        .await;
        seed(
            &conn,
            &key,
            2,
            ROW_TYPE_MEETING,
            &meeting_at(8, "This morning", now - DAY_MS / 4),
        )
        .await;
        let store = runner_store();
        pull(&conn, &key, &store, &windowed(&[ROW_TYPE_MEETING])).await;
        assert_eq!(titles(&store), vec!["This morning".to_string()]);
        // Without a window, an explicit grant takes every meeting.
        let unwindowed = runner_store();
        pull(&conn, &key, &unwindowed, &allow(&[ROW_TYPE_MEETING])).await;
        assert_eq!(titles(&unwindowed).len(), 2);
    }

    #[tokio::test]
    async fn default_options_still_pull_every_meeting() {
        let (_dir, conn, key) = relay().await;
        let now = now_unix_ms();
        seed(
            &conn,
            &key,
            1,
            ROW_TYPE_MEETING,
            &meeting_at(7, "Years ago", 1_000),
        )
        .await;
        seed(
            &conn,
            &key,
            2,
            ROW_TYPE_MEETING,
            &meeting_at(8, "Last month", now - 30 * DAY_MS),
        )
        .await;
        seed(
            &conn,
            &key,
            3,
            ROW_TYPE_MEETING,
            &meeting_at(9, "Today", now),
        )
        .await;
        let phone = Store::open_in_memory().unwrap();
        crate::pull_only_with_connection(&conn, &key, &phone, "phone", &PullOptions::default())
            .await
            .unwrap();
        let desktop = Store::open_in_memory().unwrap();
        pull_content(&conn, &key, &desktop, "desktop")
            .await
            .unwrap();
        let runner = runner_store();
        pull(&conn, &key, &runner, &PullOptions::default()).await;
        for store in [&phone, &desktop, &runner] {
            let mut got = titles(store);
            got.sort();
            assert_eq!(
                got,
                vec![
                    "Last month".to_string(),
                    "Today".to_string(),
                    "Years ago".to_string()
                ]
            );
        }
    }

    #[tokio::test]
    async fn the_window_excludes_older_items() {
        let (_dir, conn, key) = relay().await;
        let now = now_unix_ms();
        seed(
            &conn,
            &key,
            1,
            ROW_TYPE_ITEMS,
            &item_json("capture", now - 3 * DAY_MS, "old capture", None),
        )
        .await;
        // A future event pushed long ago is still inside the window: it is dated by its own time.
        seed(
            &conn,
            &key,
            2,
            ROW_TYPE_ITEMS,
            &item_json("calendar_event", now + DAY_MS, "tomorrow", None),
        )
        .await;
        seed(
            &conn,
            &key,
            3,
            ROW_TYPE_ITEMS,
            &item_json("calendar_event", now - 3 * DAY_MS, "last week", None),
        )
        .await;
        seed(
            &conn,
            &key,
            4,
            ROW_TYPE_ITEMS,
            &item_json("capture", now - DAY_MS / 2, "recent capture", None),
        )
        .await;
        let store = runner_store();
        let opts = PullOptions {
            item_window_ms: Some(DAY_MS),
            ..allow(&["items:capture", "items:calendar_event"])
        };
        pull(&conn, &key, &store, &opts).await;
        assert_eq!(
            bodies(&store, "capture"),
            vec!["recent capture".to_string()]
        );
        assert_eq!(
            bodies(&store, "calendar_event"),
            vec!["tomorrow".to_string()]
        );
    }

    #[tokio::test]
    async fn a_client_delete_tombstone_removes_pulled_rows_and_leaves_no_fts_residue_after_the_round(
    ) {
        let (_dir, conn, key) = relay().await;
        let now = now_unix_ms();
        seed(
            &conn,
            &key,
            1,
            ROW_TYPE_ITEMS,
            &item_json("capture", now, "quuxlerberg contract notes", None),
        )
        .await;
        seed(
            &conn,
            &key,
            2,
            ROW_TYPE_ITEMS,
            &item_json("capture", now, "lunch order", None),
        )
        .await;
        let store = runner_store();
        let opts = allow(&["items:capture"]);
        pull(&conn, &key, &store, &opts).await;
        assert!(
            store.fts_pages_naming("quuxlerberg").unwrap() > 0,
            "the test needs the term indexed"
        );

        seed(
            &conn,
            &key,
            3,
            ROW_TYPE_CLIENT_ITEM_FORGOTTEN,
            &forgotten_capture_json(1),
        )
        .await;
        pull(&conn, &key, &store, &opts).await;
        assert_eq!(bodies(&store, "capture"), vec!["lunch order".to_string()]);
        assert_eq!(
            store.fts_pages_naming("quuxlerberg").unwrap(),
            0,
            "no residue in the full-text index"
        );
    }
}

/// The runner's filtered pull (kind tags): an allowlist that names item kinds skips other kinds' tagged
/// `items` rows at the relay query, fetches untagged ones, and applies exactly what an unfiltered pull does.
#[cfg(test)]
mod filtered_pull_tests {
    use super::*;

    const RUNNER: &str = "runner-1";

    async fn relay() -> (tempfile::TempDir, Connection, GroupKey) {
        let dir = tempfile::tempdir().unwrap();
        let db = libsql::Builder::new_local(dir.path().join("relay.db"))
            .build()
            .await
            .unwrap();
        let conn = db.connect().unwrap();
        ensure_content_schema(&conn).await.unwrap();
        (dir, conn, GroupKey::generate())
    }

    /// One encrypted row of `device`, with `kind_tag` = the tag of `tag_kind` (or NULL).
    async fn seed(
        conn: &Connection,
        key: &GroupKey,
        device: &str,
        seq: i64,
        row_type: &str,
        json: &str,
        tag_kind: Option<&str>,
    ) {
        let aad = content_row_aad(device, seq, row_type);
        let ciphertext = STANDARD.encode(sync::encrypt_payload(key, json.as_bytes(), &aad));
        let tag = tag_kind.map(|k| sync::kind_tag(key, k));
        conn.execute(
            "INSERT INTO sync_row (device_id, local_seq, row_type, ciphertext, created_at_unix_ms, kind_tag) VALUES (?1, ?2, ?3, ?4, 0, ?5)",
            params![device, seq, row_type, ciphertext.as_str(), tag],
        )
        .await
        .unwrap();
    }

    fn item_json(kind: &str, body: &str) -> String {
        item_json_at(
            kind,
            body,
            (kind == "calendar_event").then_some("outlook"),
            now_unix_ms(),
        )
    }

    /// An `items` payload of `kind` with this `source` and date.
    fn item_json_at(kind: &str, body: &str, source: Option<&str>, ts_unix_ms: i64) -> String {
        let calendar = kind == "calendar_event";
        serde_json::to_string(&ItemPayload {
            source_meeting_id: None,
            meeting_local_seq: None,
            item: memory::AppliedItem {
                kind: kind.to_string(),
                source: source.map(str::to_string),
                ts_unix_ms,
                actors: Some("[\"Me\"]".to_string()),
                title: calendar.then(|| body.to_string()),
                body_text: body.to_string(),
                refs: None,
                flags: None,
                hash: None,
                external_id: calendar.then(|| format!("{body}#1")),
                app_bundle_id: (kind == "capture").then(|| "com.a".to_string()),
                url: None,
            },
        })
        .unwrap()
    }

    fn allow(types: &[&str]) -> PullOptions {
        PullOptions {
            row_type_allowlist: Some(types.iter().map(|t| t.to_string()).collect()),
            ..Default::default()
        }
    }

    fn runner_store() -> Store {
        let store = Store::open_in_memory().unwrap();
        store.set_own_device_id(RUNNER);
        store
    }

    async fn pull(
        conn: &Connection,
        key: &GroupKey,
        store: &Store,
        opts: &PullOptions,
    ) -> crate::RunnerPullReport {
        crate::runner_pull_with_connection(conn, key, store, RUNNER, opts)
            .await
            .unwrap()
    }

    fn bodies(store: &Store, kind: &str) -> Vec<String> {
        let mut out: Vec<String> = store
            .export_items_page(kind, 0, 100_000)
            .unwrap()
            .into_iter()
            .map(|r| r.body_text)
            .collect();
        out.sort();
        out
    }

    async fn count(conn: &Connection, sql: &str) -> usize {
        let mut rows = conn.query(sql, ()).await.unwrap();
        rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap() as usize
    }

    /// Three devices; per device: tagged events, captures and transcripts, untagged events and captures,
    /// and a routine run (not granted).
    async fn seed_mixed_relay(conn: &Connection, key: &GroupKey) {
        for device in ["dev-a", "dev-b", "dev-c"] {
            let mut seq = 0;
            let mut next = || {
                seq += 1;
                seq
            };
            for i in 0..4 {
                seed(
                    conn,
                    key,
                    device,
                    next(),
                    ROW_TYPE_ITEMS,
                    &item_json("capture", &format!("{device} cap {i}")),
                    Some("capture"),
                )
                .await;
                seed(
                    conn,
                    key,
                    device,
                    next(),
                    ROW_TYPE_ITEMS,
                    &item_json("calendar_event", &format!("{device} ev {i}")),
                    Some("calendar_event"),
                )
                .await;
                seed(
                    conn,
                    key,
                    device,
                    next(),
                    ROW_TYPE_ITEMS,
                    &item_json("transcript", &format!("{device} line {i}")),
                    Some("transcript"),
                )
                .await;
                seed(
                    conn,
                    key,
                    device,
                    next(),
                    ROW_TYPE_ITEMS,
                    &item_json("calendar_event", &format!("{device} old ev {i}")),
                    None,
                )
                .await;
                seed(
                    conn,
                    key,
                    device,
                    next(),
                    ROW_TYPE_ITEMS,
                    &item_json("capture", &format!("{device} old cap {i}")),
                    None,
                )
                .await;
                seed(
                    conn,
                    key,
                    device,
                    next(),
                    ROW_TYPE_ITEMS,
                    &item_json("capture", &format!("{device} cap2 {i}")),
                    Some("capture"),
                )
                .await;
            }
        }
    }

    /// (a) + (c): only granted-kind tagged rows and untagged rows are fetched, and the applied set equals the
    /// same pull's without tags, at the default span and at spans small enough to put every row on a window
    /// edge.
    #[tokio::test]
    async fn filtered_pull_fetches_only_granted_and_untagged_items_and_applies_what_unfiltered_does(
    ) {
        for span in [None, Some(1), Some(2), Some(5)] {
            let (_dir, conn, key) = relay().await;
            seed_mixed_relay(&conn, &key).await;
            let opts = allow(&["items:calendar_event"]);
            let total_items = count(
                &conn,
                "SELECT COUNT(*) FROM sync_row WHERE row_type = 'items'",
            )
            .await;
            let wanted = count(
                &conn,
                &format!(
                    "SELECT COUNT(*) FROM sync_row WHERE row_type = 'items' AND (kind_tag IS NULL OR kind_tag = '{}')",
                    sync::kind_tag(&key, "calendar_event")
                ),
            )
            .await;

            let filtered = runner_store();
            let report = pull_with_span(&conn, &key, &filtered, &opts, span).await;
            assert!(report.complete, "span {span:?}");
            assert_eq!(
                report.rows_fetched, wanted,
                "span {span:?}: tagged events and untagged items only"
            );

            conn.execute("UPDATE sync_row SET kind_tag = NULL", ())
                .await
                .unwrap();
            let unfiltered = runner_store();
            let base = pull_with_span(&conn, &key, &unfiltered, &opts, span).await;
            assert!(base.complete);
            assert_eq!(base.rows_fetched, total_items);

            assert_eq!(
                bodies(&filtered, "calendar_event"),
                bodies(&unfiltered, "calendar_event"),
                "span {span:?}"
            );
            assert_eq!(
                bodies(&filtered, "calendar_event").len(),
                24,
                "every event, tagged or not"
            );
            for kind in ["capture", "transcript"] {
                assert!(bodies(&filtered, kind).is_empty());
                assert!(bodies(&unfiltered, kind).is_empty());
            }
            assert_eq!(report.rows_applied, base.rows_applied, "span {span:?}");
            for device in ["dev-a", "dev-b", "dev-c"] {
                assert_eq!(filtered.sync_gap(device).unwrap(), None);
                assert_eq!(
                    filtered.get_sync_cursor(device).unwrap(),
                    24,
                    "span {span:?}: at the peer's top"
                );
            }
        }
    }

    /// The runner's per-type diagnostics: a known mix of items, meetings and tombstones, per type fetched and
    /// applied counts match the mix, and their sums equal the report's totals.
    #[tokio::test]
    async fn pull_report_counts_rows_fetched_and_applied_per_type() {
        let (_dir, conn, key) = relay().await;
        for i in 1..=5 {
            seed(
                &conn,
                &key,
                "dev-a",
                i,
                ROW_TYPE_ITEMS,
                &item_json("calendar_event", &format!("ev {i}")),
                None,
            )
            .await;
        }
        // Four more items of a kind the allowlist leaves out: fetched, not applied.
        for i in 6..=9 {
            seed(
                &conn,
                &key,
                "dev-a",
                i,
                ROW_TYPE_ITEMS,
                &item_json("capture", &format!("cap {i}")),
                None,
            )
            .await;
        }
        let meeting = serde_json::to_string(&MeetingPayload {
            source_meeting_id: Some(1),
            origin_epoch: None,
            meeting: memory::AppliedMeeting {
                attendee_details: vec![],
                start_unix_ms: now_unix_ms(),
                end_unix_ms: None,
                title: "Standup".to_string(),
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
                trimmed_keep_ms: None,
            },
        })
        .unwrap();
        for i in 10..=11 {
            seed(&conn, &key, "dev-a", i, ROW_TYPE_MEETING, &meeting, None).await;
        }
        for i in 12..=14 {
            let (kind, opaque_id) = memory::client_forget::pulled_capture_identity("dev-a", i);
            let json = serde_json::to_string(&memory::ClientItemForgotten {
                kind: kind.to_string(),
                opaque_id,
                forgotten_at_unix_ms: 1,
            })
            .unwrap();
            seed(
                &conn,
                &key,
                "dev-a",
                i,
                ROW_TYPE_CLIENT_ITEM_FORGOTTEN,
                &json,
                None,
            )
            .await;
        }
        let store = runner_store();
        let report = pull(
            &conn,
            &key,
            &store,
            &allow(&["items:calendar_event", "meeting"]),
        )
        .await;
        assert!(report.complete);
        let at = |name: &str| {
            report.by_type[crate::PULL_TYPE_NAMES
                .iter()
                .position(|n| *n == name)
                .unwrap()]
        };
        assert_eq!(at("items").fetched, 9);
        assert_eq!(at("items").applied, 5, "the four captures are not granted");
        assert_eq!(at("meeting").fetched, 2);
        assert_eq!(at("meeting").applied, 2);
        assert_eq!(at("tombstone").fetched, 3);
        assert_eq!(report.skipped_allowlist, 4);
        assert_eq!(
            (
                report.skip_kind,
                report.skip_mcp,
                report.skip_google_cal,
                report.skip_item_window
            ),
            (4, 0, 0, 0)
        );
        assert_eq!(
            report.by_type.iter().map(|c| c.fetched).sum::<usize>(),
            report.rows_fetched
        );
        assert_eq!(
            report.by_type.iter().map(|c| c.applied).sum::<usize>() + report.own_results_applied,
            report.rows_applied
        );
        assert!(report.rounds >= 1);
    }

    /// A newest-first pull in small pages reads most rows as gap pages: they count per type, and their fetch and
    /// apply time is recorded.
    #[tokio::test]
    async fn gap_pages_count_per_type_and_are_timed() {
        let (_dir, conn, key) = relay().await;
        for i in 1..=12 {
            seed(
                &conn,
                &key,
                "dev-a",
                i,
                ROW_TYPE_ITEMS,
                &item_json("calendar_event", &format!("ev {i}")),
                None,
            )
            .await;
        }
        let store = runner_store();
        let opts = allow(&["items:calendar_event"]);
        let pace = Pace {
            rows: 3,
            peer_budget: std::time::Duration::from_secs(60),
            round_budget: None,
        };
        let (mut fetched, mut applied, mut gap_fetched, mut gap_us) = (0, 0, 0, 0);
        for _ in 0..20 {
            let mut stats = Vec::new();
            let more = pull_round_mode(
                &conn, &key, &store, RUNNER, pace, &opts, true, true, &mut stats,
            )
            .await
            .unwrap();
            for s in &stats {
                fetched += s.by_type[0].fetched;
                applied += s.by_type[0].applied;
                gap_fetched += s.gap_fetched;
                gap_us += s.gap_fetch_us + s.gap_apply_us;
            }
            if !more {
                break;
            }
        }
        assert_eq!((fetched, applied), (12, 12));
        assert!(
            gap_fetched > 0,
            "newest first leaves older rows to gap pages"
        );
        assert!(gap_us > 0, "gap fetch and apply are timed");
    }

    /// A page the time budget stops is fetched again later: its unhandled rows count as refetch.
    #[tokio::test]
    async fn a_budget_stopped_page_counts_refetch() {
        let (_dir, conn, key) = relay().await;
        for i in 1..=6 {
            seed(
                &conn,
                &key,
                "dev-a",
                i,
                ROW_TYPE_ITEMS,
                &item_json("calendar_event", &format!("ev {i}")),
                None,
            )
            .await;
        }
        let store = runner_store();
        let spent = Pace {
            rows: MAX_ROWS_PER_PULL_PASS,
            peer_budget: std::time::Duration::ZERO,
            round_budget: None,
        };
        let mut stats = Vec::new();
        let more = pull_round(
            &conn,
            &key,
            &store,
            RUNNER,
            spent,
            &allow(&["items:calendar_event"]),
            true,
            &mut stats,
        )
        .await
        .unwrap();
        assert!(more);
        assert_eq!(stats[0].fetched, 6);
        assert_eq!(
            stats[0].refetch, 5,
            "one row handled, five left for the next pass"
        );
    }

    async fn pull_with_span(
        conn: &Connection,
        key: &GroupKey,
        store: &Store,
        opts: &PullOptions,
        span: Option<i64>,
    ) -> crate::RunnerPullReport {
        FILTERED_PULL_TEST_SPAN.with(|c| c.set(span));
        let report = pull(conn, key, store, opts).await;
        FILTERED_PULL_TEST_SPAN.with(|c| c.set(None));
        report
    }

    /// The live range (oldest first, no gap): windows move the cursor over left-out rows to each peer's top,
    /// within a round while its budget lasts (a spent budget reports more and goes on next round), and later
    /// rows pushed above it are read the same way.
    #[tokio::test]
    async fn filtered_pull_live_range_windows_reach_the_top_and_match_unfiltered() {
        let spent = Pace {
            rows: MAX_ROWS_PER_PULL_PASS,
            peer_budget: std::time::Duration::ZERO,
            round_budget: None,
        };
        for (span, pace) in [
            (None, NORMAL_PACE),
            (Some(1), NORMAL_PACE),
            (Some(2), NORMAL_PACE),
            (Some(5), NORMAL_PACE),
            (Some(2), spent),
        ] {
            let (_dir, conn, key) = relay().await;
            seed_mixed_relay(&conn, &key).await;
            let opts = allow(&["items:calendar_event"]);
            let rounds = |store: Store| {
                let (conn, key, opts) = (&conn, &key, &opts);
                async move {
                    FILTERED_PULL_TEST_SPAN.with(|c| c.set(span));
                    let mut fetched = 0;
                    for _ in 0..50 {
                        let mut stats = Vec::new();
                        let more =
                            pull_round(conn, key, &store, RUNNER, pace, opts, true, &mut stats)
                                .await
                                .unwrap();
                        fetched += stats
                            .iter()
                            .map(|s| s.fetched + s.gap_fetched)
                            .sum::<usize>();
                        if !more {
                            break;
                        }
                    }
                    FILTERED_PULL_TEST_SPAN.with(|c| c.set(None));
                    (store, fetched)
                }
            };
            let (filtered, fetched) = rounds(runner_store()).await;
            assert_eq!(
                fetched,
                3 * 12,
                "span {span:?}: tagged events and untagged items only"
            );
            for device in ["dev-a", "dev-b", "dev-c"] {
                assert_eq!(
                    filtered.get_sync_cursor(device).unwrap(),
                    24,
                    "span {span:?}"
                );
            }
            // Rows pushed later, above the cursor: an unwanted tail, then one event.
            for seq in 25..=30 {
                seed(
                    &conn,
                    &key,
                    "dev-a",
                    seq,
                    ROW_TYPE_ITEMS,
                    &item_json("capture", &format!("late cap {seq}")),
                    Some("capture"),
                )
                .await;
            }
            seed(
                &conn,
                &key,
                "dev-a",
                31,
                ROW_TYPE_ITEMS,
                &item_json("calendar_event", "late ev"),
                Some("calendar_event"),
            )
            .await;
            seed(
                &conn,
                &key,
                "dev-a",
                32,
                ROW_TYPE_ITEMS,
                &item_json("capture", "late cap 32"),
                Some("capture"),
            )
            .await;
            let (filtered, fetched) = rounds(filtered).await;
            assert_eq!(fetched, 1, "span {span:?}: only the late event");
            assert_eq!(
                filtered.get_sync_cursor("dev-a").unwrap(),
                32,
                "span {span:?}"
            );

            conn.execute("UPDATE sync_row SET kind_tag = NULL", ())
                .await
                .unwrap();
            let (unfiltered, _) = rounds(runner_store()).await;
            assert_eq!(
                bodies(&filtered, "calendar_event"),
                bodies(&unfiltered, "calendar_event"),
                "span {span:?}"
            );
            assert_eq!(bodies(&filtered, "calendar_event").len(), 25);
            assert!(bodies(&filtered, "capture").is_empty());
        }
    }

    /// (c) Untagged rows of any kind are fetched and filtered after decrypting, tagged ones of other kinds
    /// never reach the runner.
    #[tokio::test]
    async fn filtered_pull_applies_tagged_and_untagged_rows_of_a_granted_kind() {
        let (_dir, conn, key) = relay().await;
        seed(
            &conn,
            &key,
            "dev-a",
            1,
            ROW_TYPE_ITEMS,
            &item_json("calendar_event", "tagged"),
            Some("calendar_event"),
        )
        .await;
        seed(
            &conn,
            &key,
            "dev-a",
            2,
            ROW_TYPE_ITEMS,
            &item_json("calendar_event", "untagged"),
            None,
        )
        .await;
        seed(
            &conn,
            &key,
            "dev-a",
            3,
            ROW_TYPE_ITEMS,
            &item_json("capture", "old capture"),
            None,
        )
        .await;
        seed(
            &conn,
            &key,
            "dev-a",
            4,
            ROW_TYPE_ITEMS,
            &item_json("capture", "new capture"),
            Some("capture"),
        )
        .await;
        let store = runner_store();
        let report = pull(&conn, &key, &store, &allow(&["items:calendar_event"])).await;
        assert!(report.complete);
        assert_eq!(
            bodies(&store, "calendar_event"),
            vec!["tagged".to_string(), "untagged".to_string()]
        );
        assert!(bodies(&store, "capture").is_empty());
        assert_eq!(
            report.rows_fetched, 3,
            "the tagged capture is never fetched"
        );
    }

    /// (b) Every kind the repo pushes as an `items` row (capture, calendar event, transcript line) is
    /// tagged on push and comes back through a pull granted that kind alone.
    #[tokio::test]
    async fn filtered_pull_round_trips_every_pushed_kind() {
        let (_dir, conn, key) = relay().await;
        let pusher = Store::open_in_memory().unwrap();
        pusher
            .insert_capture_item(&memory::NewCaptureItem {
                device_id: "dev-a".to_string(),
                ts_unix_ms: now_unix_ms(),
                app_bundle_id: "com.a".to_string(),
                window_title: "Local".to_string(),
                url: None,
                text: "a capture".to_string(),
                hash: 9,
                source: "screen".to_string(),
            })
            .unwrap();
        pusher
            .upsert_calendar_event_items(&[memory::NewCalendarEventItem {
                device_id: "dev-a".to_string(),
                source: "outlook".to_string(),
                external_id: "evt-1".to_string(),
                ts_unix_ms: now_unix_ms(),
                title: "Planning".to_string(),
                actors: vec![],
            }])
            .unwrap();
        let meeting_id = pusher
            .insert_meeting(&memory::NewMeeting {
                device_id: "dev-a".to_string(),
                start_unix_ms: now_unix_ms(),
                end_unix_ms: None,
                title: "Standup".to_string(),
                attendees: vec![],
                trigger: memory::MeetingTrigger::Calendar,
                audio_path: None,
                status: memory::MeetingStatus::Recording,
            })
            .unwrap();
        pusher
            .insert_transcript_segments(&[memory::NewTranscriptSegment {
                meeting_id,
                start_cs: 0,
                speaker_label: "Me".to_string(),
                text: "a transcript line".to_string(),
            }])
            .unwrap();
        pusher
            .complete_meeting(meeting_id, now_unix_ms(), None)
            .unwrap();
        push_content(&conn, &key, &pusher, "dev-a").await.unwrap();
        let kinds = ["capture", "calendar_event", "transcript"];
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM sync_row WHERE row_type = 'items' AND kind_tag IS NULL"
            )
            .await,
            0,
            "every pushed item is tagged"
        );
        let mut fetched = Vec::new();
        for kind in kinds {
            let store = runner_store();
            let report = pull(&conn, &key, &store, &allow(&[&format!("items:{kind}")])).await;
            assert!(report.complete, "{kind}");
            assert_eq!(bodies(&store, kind).len(), 1, "{kind} round-trips");
            for other in kinds.iter().filter(|k| **k != kind) {
                assert!(
                    bodies(&store, other).is_empty(),
                    "{kind} pull applied {other}"
                );
            }
            fetched.push(report.rows_fetched);
        }
        // The same pulls with every tag cleared (as rows from before tags) fetch exactly the two other kinds'
        // items more.
        conn.execute("UPDATE sync_row SET kind_tag = NULL, slice_tag = NULL", ())
            .await
            .unwrap();
        for (kind, filtered) in kinds.iter().zip(fetched) {
            let store = runner_store();
            let report = pull(&conn, &key, &store, &allow(&[&format!("items:{kind}")])).await;
            assert_eq!(bodies(&store, kind).len(), 1, "{kind}");
            assert_eq!(
                filtered + 2,
                report.rows_fetched,
                "{kind}: the other kinds' items are not fetched"
            );
        }
    }

    /// (d) One granted row per 5,000 tagged rows of another kind: the pull ends at the peer's top with the
    /// gap closed, every granted row applied, and well under 1% of the rows fetched. At the default span
    /// and at a span that takes many windows.
    #[tokio::test]
    async fn filtered_pull_of_a_sparse_kind_finishes_and_fetches_almost_nothing() {
        for span in [None, Some(3_000)] {
            let (_dir, conn, key) = relay().await;
            const STRIDE: i64 = 5_001;
            const WANTED: i64 = 10;
            // The top row is an unwanted one, so "the cursor is at the top" is not just "at the last wanted row".
            let top = STRIDE * WANTED;
            // Unwanted rows: junk ciphertext (counted undecodable if ever fetched), tagged as captures.
            conn.execute(
                "WITH RECURSIVE s(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM s WHERE n < ?1) \
                 INSERT INTO sync_row (device_id, local_seq, row_type, ciphertext, created_at_unix_ms, kind_tag) \
                 SELECT 'dev-a', n, 'items', 'junk', 0, ?2 FROM s WHERE n % ?3 != 1",
                params![top, sync::kind_tag(&key, "capture"), STRIDE],
            )
            .await
            .unwrap();
            for i in 0..WANTED {
                let seq = i * STRIDE + 1;
                seed(
                    &conn,
                    &key,
                    "dev-a",
                    seq,
                    ROW_TYPE_ITEMS,
                    &item_json("calendar_event", &format!("ev {seq}")),
                    Some("calendar_event"),
                )
                .await;
            }
            let rows = count(&conn, "SELECT COUNT(*) FROM sync_row").await;
            assert_eq!(rows as i64, top);

            let store = runner_store();
            let started = std::time::Instant::now();
            let report =
                pull_with_span(&conn, &key, &store, &allow(&["items:calendar_event"]), span).await;
            eprintln!(
                "sparse fixture span {span:?}: {} rows, fetched {}, rounds {}, {:?}",
                rows,
                report.rows_fetched,
                report.rounds,
                started.elapsed()
            );
            assert!(report.complete, "span {span:?}");
            assert_eq!(
                store.get_sync_cursor("dev-a").unwrap(),
                top,
                "span {span:?}: the cursor is at the peer's top"
            );
            assert_eq!(
                store.sync_gap("dev-a").unwrap(),
                None,
                "span {span:?}: the gap is pulled"
            );
            assert!(
                report.rows_fetched * 100 < rows,
                "span {span:?}: fetched {} of {rows}",
                report.rows_fetched
            );
            let applied = bodies(&store, "calendar_event");
            assert_eq!(applied.len() as i64, WANTED, "span {span:?}");
            assert!(
                applied.contains(&"ev 1".to_string()),
                "the lowest granted row is applied"
            );
        }
    }

    /// (e) A relay whose `sync_row` predates `kind_tag` (and whose column cannot be added this run) pulls
    /// unfiltered, without an error, and applies the same rows.
    #[tokio::test]
    async fn filtered_pull_on_a_relay_without_kind_tag_pulls_unfiltered() {
        let dir = tempfile::tempdir().unwrap();
        let db = libsql::Builder::new_local(dir.path().join("relay.db"))
            .build()
            .await
            .unwrap();
        let conn = db.connect().unwrap();
        conn.execute(super::tests::OLD_SYNC_ROW_DDL, ())
            .await
            .unwrap();
        // This process believes the column is there, so ensure_content_schema does not add it: the pull
        // itself must cope.
        KIND_TAG_COLUMN_SEEN.set();
        let key = GroupKey::generate();
        let rows = [
            ("calendar_event", "ev 1"),
            ("capture", "cap 1"),
            ("calendar_event", "ev 2"),
            ("transcript", "line"),
        ];
        for (i, (kind, body)) in rows.iter().enumerate() {
            let seq = i as i64 + 1;
            let aad = content_row_aad("dev-a", seq, ROW_TYPE_ITEMS);
            let ciphertext = STANDARD.encode(sync::encrypt_payload(
                &key,
                item_json(kind, body).as_bytes(),
                &aad,
            ));
            conn.execute(
                "INSERT INTO sync_row (device_id, local_seq, row_type, ciphertext, created_at_unix_ms) VALUES ('dev-a', ?1, 'items', ?2, 0)",
                params![seq, ciphertext.as_str()],
            )
            .await
            .unwrap();
        }
        for span in [None, Some(1)] {
            let store = runner_store();
            let report =
                pull_with_span(&conn, &key, &store, &allow(&["items:calendar_event"]), span).await;
            assert!(report.complete);
            assert_eq!(report.rows_fetched, 4, "unfiltered: every items row");
            assert_eq!(
                bodies(&store, "calendar_event"),
                vec!["ev 1".to_string(), "ev 2".to_string()]
            );
            assert_eq!(store.get_sync_cursor("dev-a").unwrap(), 4);
            assert_eq!(store.sync_gap("dev-a").unwrap(), None);
        }
        let mut cols = conn
            .query(
                "SELECT COUNT(*) FROM pragma_table_info('sync_row') WHERE name = 'kind_tag'",
                (),
            )
            .await
            .unwrap();
        assert_eq!(
            cols.next().await.unwrap().unwrap().get::<i64>(0).unwrap(),
            0,
            "the pull did not add the column"
        );
        KIND_TAG_COLUMN_SEEN.reset();
    }

    #[test]
    fn no_such_kind_tag_column_is_recognized() {
        assert!(is_no_such_kind_tag_column(
            "SQLite failure: `no such column: kind_tag`"
        ));
        assert!(!is_no_such_kind_tag_column("no such column: entity_tag"));
        assert!(!is_no_such_kind_tag_column(
            "table sync_row has no column named kind_tag"
        ));
        assert!(!is_no_such_kind_tag_column("database is locked"));
    }

    /// Kind tags only when the allowlist restricts item kinds: never for a phone or desktop pull.
    #[test]
    fn filtered_pull_tags_only_a_kind_restricted_allowlist() {
        let key = GroupKey::generate();
        let tags = |opts: &PullOptions| RowFilter::new(opts, true, 0, &key).kind_tags;
        assert_eq!(tags(&PullOptions::default()), None);
        assert_eq!(tags(&allow(&[ROW_TYPE_ROUTINE_RUN])), None);
        assert_eq!(tags(&allow(&[ROW_TYPE_ITEMS])), None);
        assert_eq!(
            tags(&allow(&["items:calendar_event", ROW_TYPE_ITEMS])),
            None,
            "every kind"
        );
        assert_eq!(
            tags(&allow(&["items:calendar_event"])),
            Some(vec![sync::kind_tag(&key, "calendar_event")])
        );
        assert_eq!(
            tags(&allow(&[
                "items:capture",
                "items:transcript",
                "items:capture"
            ])),
            Some(vec![
                sync::kind_tag(&key, "capture"),
                sync::kind_tag(&key, "transcript")
            ])
        );
    }

    const DAY: i64 = 86_400_000;
    /// 2026-11-24T00:00:00Z.
    const FIXED_NOW: i64 = 1_795_478_400_000;
    /// 2031-01-15T00:00:00Z, after the last month bucket (2029-12).
    const AFTER_2029: i64 = 1_926_201_600_000;

    /// The runner's options for a calendar grant: a window of `days` and no Google Calendar event.
    fn runner_calendar_options(days: i64) -> PullOptions {
        PullOptions {
            item_window_ms: Some(days * DAY),
            non_google_calendar_only: true,
            ..allow(&["items:calendar_event"])
        }
    }

    /// The `slice_tag` a push gives an item of `kind` with `source` dated `ts`.
    fn slice_of(key: &GroupKey, kind: &str, source: Option<&str>, ts: i64) -> String {
        sync::slice_tag(
            key,
            kind,
            sync::source_class(kind, source),
            sync::month_bucket(ts),
        )
    }

    /// The first millisecond of `ts`'s UTC month.
    fn month_start(ts: i64) -> i64 {
        let mut day = ts.div_euclid(DAY) * DAY;
        while sync::month_bucket(day - DAY) == sync::month_bucket(day) {
            day -= DAY;
        }
        day
    }

    /// One `items` row of `dev-a` at `seq`: `kind_tagged` sets its `kind_tag`, `sliced` its `slice_tag`.
    #[allow(clippy::too_many_arguments)]
    async fn seed_item(
        conn: &Connection,
        key: &GroupKey,
        seq: i64,
        kind: &str,
        body: &str,
        source: Option<&str>,
        ts: i64,
        (kind_tagged, sliced): (bool, bool),
    ) {
        seed(
            conn,
            key,
            "dev-a",
            seq,
            ROW_TYPE_ITEMS,
            &item_json_at(kind, body, source, ts),
            kind_tagged.then_some(kind),
        )
        .await;
        if sliced {
            conn.execute(
                "UPDATE sync_row SET slice_tag = ?1 WHERE device_id = 'dev-a' AND local_seq = ?2",
                params![slice_of(key, kind, source, ts), seq],
            )
            .await
            .unwrap();
        }
    }

    /// Kind, body, source, date, and (kind tagged, sliced).
    type SliceRow = (
        &'static str,
        &'static str,
        Option<&'static str>,
        i64,
        (bool, bool),
    );

    /// Thirteen `items` rows for a 60-day calendar grant. Seven are fetched: an in-window event, one in the
    /// cutoff's month (before the cutoff), one dated after 2029 (bucket 359), an old client's (kind tag only),
    /// two untagged rows and an old client's Google event. Six never are: Google and blank-source events, an
    /// event two months before the cutoff's month, a Google event after 2029, and a capture tagged either way.
    async fn seed_slice_relay(conn: &Connection, key: &GroupKey) {
        let now = now_unix_ms();
        let boundary = month_start(now - 60 * DAY);
        let old = month_start(boundary - 40 * DAY);
        let both = (true, true);
        let rows: [SliceRow; 13] = [
            (
                "calendar_event",
                "google in window",
                Some("google_calendar"),
                now - DAY,
                both,
            ),
            ("calendar_event", "blank source", Some(" "), now - DAY, both),
            (
                "calendar_event",
                "outlook old month",
                Some("outlook"),
                old,
                both,
            ),
            (
                "calendar_event",
                "outlook in window",
                Some("outlook"),
                now - DAY,
                both,
            ),
            (
                "calendar_event",
                "outlook boundary month",
                Some("outlook"),
                boundary,
                both,
            ),
            (
                "calendar_event",
                "outlook after 2029",
                Some("outlook"),
                AFTER_2029,
                both,
            ),
            (
                "calendar_event",
                "google after 2029",
                Some("google_calendar"),
                AFTER_2029,
                both,
            ),
            ("capture", "capture sliced", None, now - DAY, both),
            (
                "calendar_event",
                "outlook old client",
                Some("outlook"),
                now - DAY,
                (true, false),
            ),
            (
                "calendar_event",
                "outlook untagged",
                Some("outlook"),
                now - DAY,
                (false, false),
            ),
            (
                "capture",
                "capture untagged",
                None,
                now - DAY,
                (false, false),
            ),
            (
                "calendar_event",
                "google old client",
                Some("google_calendar"),
                now - DAY,
                (true, false),
            ),
            (
                "capture",
                "capture old client",
                None,
                now - DAY,
                (true, false),
            ),
        ];
        for (i, (kind, body, source, ts, tags)) in rows.into_iter().enumerate() {
            seed_item(conn, key, i as i64 + 1, kind, body, source, ts, tags).await;
        }
    }

    /// (a), (b), (c), (e): Google-class and old-month slices are not fetched; non-Google in-window, boundary-month
    /// and after-2029 rows are, and so are rows with no slice tag (an old client's, or untagged). What applies is
    /// what the same relay without slice tags applies, at the default span and at spans that put rows on window
    /// edges.
    #[tokio::test]
    async fn runner_slice_filter_leaves_out_google_and_old_months_and_applies_what_unsliced_does() {
        for span in [None, Some(1), Some(3)] {
            let (_dir, conn, key) = relay().await;
            seed_slice_relay(&conn, &key).await;
            let opts = runner_calendar_options(60);
            let sliced = runner_store();
            let report = pull_with_span(&conn, &key, &sliced, &opts, span).await;
            assert!(report.complete, "span {span:?}");
            assert_eq!(report.rows_fetched, 7, "span {span:?}");
            let months =
                usize::from(sync::MAX_MONTH_BUCKET - sync::month_bucket(now_unix_ms() - 60 * DAY))
                    + 1;
            assert_eq!(
                report.slice_tags, months,
                "one non-Google calendar slice a month from the cutoff's through 359"
            );
            let applied = bodies(&sliced, "calendar_event");
            for body in [
                "outlook in window",
                "outlook after 2029",
                "outlook old client",
                "outlook untagged",
            ] {
                assert!(
                    applied.contains(&body.to_string()),
                    "span {span:?}: {body} in {applied:?}"
                );
            }
            assert_eq!(
                sliced.get_sync_cursor("dev-a").unwrap(),
                13,
                "span {span:?}: at the peer's top"
            );
            assert_eq!(sliced.sync_gap("dev-a").unwrap(), None);

            conn.execute("UPDATE sync_row SET slice_tag = NULL", ())
                .await
                .unwrap();
            let unsliced = runner_store();
            let base = pull_with_span(&conn, &key, &unsliced, &opts, span).await;
            assert!(base.complete);
            assert_eq!(
                base.rows_fetched, 11,
                "kind tags alone leave out only the two tagged captures"
            );
            assert_eq!(
                applied,
                bodies(&unsliced, "calendar_event"),
                "span {span:?}"
            );
            assert_eq!(report.rows_applied, base.rows_applied, "span {span:?}");
            assert!(bodies(&sliced, "capture").is_empty());
        }
    }

    /// (d) A relay without `slice_tag` (and whose column cannot be added this run) pulls with the kind filter
    /// alone, without an error, and applies the same rows; one without either column pulls unfiltered.
    #[tokio::test]
    async fn runner_slice_filter_on_a_relay_without_slice_tag_falls_back_to_kind_tags() {
        let (_dir, conn, key) = relay().await;
        seed_slice_relay(&conn, &key).await;
        let opts = runner_calendar_options(60);
        let with = runner_store();
        let sliced = pull(&conn, &key, &with, &opts).await;

        SLICE_TAG_COLUMN_SEEN.set();
        conn.execute("ALTER TABLE sync_row DROP COLUMN slice_tag", ())
            .await
            .unwrap();
        for span in [None, Some(1)] {
            let store = runner_store();
            let report = pull_with_span(&conn, &key, &store, &opts, span).await;
            assert!(report.complete, "span {span:?}");
            assert_eq!(
                report.rows_fetched, 11,
                "span {span:?}: the kind filter still applies"
            );
            assert_eq!(report.slice_tags, 0, "the query carried no slice tags");
            assert_eq!(report.rows_applied, sliced.rows_applied);
            assert_eq!(
                bodies(&store, "calendar_event"),
                bodies(&with, "calendar_event")
            );
            assert_eq!(store.get_sync_cursor("dev-a").unwrap(), 13);
        }

        KIND_TAG_COLUMN_SEEN.set();
        conn.execute("ALTER TABLE sync_row DROP COLUMN kind_tag", ())
            .await
            .unwrap();
        let store = runner_store();
        let report = pull(&conn, &key, &store, &opts).await;
        assert!(report.complete);
        assert_eq!(report.rows_fetched, 13, "unfiltered: every items row");
        assert_eq!(report.rows_applied, sliced.rows_applied);
        assert_eq!(
            bodies(&store, "calendar_event"),
            bodies(&with, "calendar_event")
        );
        let mut cols = conn
            .query("SELECT COUNT(*) FROM pragma_table_info('sync_row') WHERE name IN ('kind_tag', 'slice_tag')", ())
            .await
            .unwrap();
        assert_eq!(
            cols.next().await.unwrap().unwrap().get::<i64>(0).unwrap(),
            0,
            "the pull added no column"
        );
        SLICE_TAG_COLUMN_SEEN.reset();
        KIND_TAG_COLUMN_SEEN.reset();
    }

    /// A row with a `slice_tag` and a NULL `kind_tag` (the slice backfill reached it, the kind backfill did not) is
    /// judged by its slice alone: fetched when its slice is asked for, left out when not, though a NULL `kind_tag`
    /// alone would fetch it.
    #[tokio::test]
    async fn runner_a_sliced_row_without_a_kind_tag_is_judged_by_its_slice_alone() {
        let (_dir, conn, key) = relay().await;
        let now = now_unix_ms();
        let slice_only = (false, true);
        seed_item(
            &conn,
            &key,
            1,
            "calendar_event",
            "outlook in window",
            Some("outlook"),
            now - DAY,
            slice_only,
        )
        .await;
        seed_item(
            &conn,
            &key,
            2,
            "calendar_event",
            "google in window",
            Some("google_calendar"),
            now - DAY,
            slice_only,
        )
        .await;
        seed_item(
            &conn,
            &key,
            3,
            "calendar_event",
            "outlook old month",
            Some("outlook"),
            now - 400 * DAY,
            slice_only,
        )
        .await;
        seed_item(
            &conn,
            &key,
            4,
            "capture",
            "capture",
            None,
            now - DAY,
            slice_only,
        )
        .await;
        let opts = runner_calendar_options(60);

        let store = runner_store();
        let report = pull(&conn, &key, &store, &opts).await;
        assert!(report.complete);
        assert_eq!(report.rows_fetched, 1, "only the asked-for slice");
        assert_eq!(bodies(&store, "calendar_event"), ["outlook in window"]);
        assert_eq!(store.get_sync_cursor("dev-a").unwrap(), 4);

        // The same rows without their slice: the NULL kind tag fetches every one.
        conn.execute("UPDATE sync_row SET slice_tag = NULL", ())
            .await
            .unwrap();
        let unsliced = runner_store();
        let base = pull(&conn, &key, &unsliced, &opts).await;
        assert_eq!(base.rows_fetched, 4);
        assert_eq!(
            bodies(&unsliced, "calendar_event"),
            bodies(&store, "calendar_event")
        );
    }

    #[test]
    fn runner_no_such_slice_tag_column_is_recognized() {
        assert!(is_no_such_slice_tag_column(
            "SQLite failure: `no such column: slice_tag`"
        ));
        assert!(!is_no_such_slice_tag_column("no such column: kind_tag"));
        assert!(!is_no_such_slice_tag_column(
            "table sync_row has no column named slice_tag"
        ));
    }

    /// The slices a pull asks for, at a fixed now (2026-11-24, bucket 322): each granted kind, each source class
    /// it may carry, each month from the cutoff's (60 days back: 2026-09, bucket 320) through 359.
    #[test]
    fn runner_slice_tags_cover_granted_kinds_classes_and_months() {
        use sync::SourceClass::{Google, None as NoClass, Other};
        let key = GroupKey::generate();
        let slices = |opts: &PullOptions| RowFilter::new(opts, true, FIXED_NOW, &key).slice_tags;
        let expect = |kinds: &[(&str, &[sync::SourceClass])], first: u16| -> Vec<String> {
            let mut out = Vec::new();
            for (kind, classes) in kinds {
                for class in *classes {
                    for month in first..=sync::MAX_MONTH_BUCKET {
                        out.push(sync::slice_tag(&key, kind, *class, month));
                    }
                }
            }
            out
        };
        assert_eq!(sync::month_bucket(FIXED_NOW - 60 * DAY), 320);
        let runner = runner_calendar_options(60);
        assert_eq!(
            slices(&runner),
            Some(expect(&[("calendar_event", &[Other])], 320))
        );
        assert_eq!(slices(&runner).unwrap().len(), 40);
        let every_source = PullOptions {
            non_google_calendar_only: false,
            ..runner_calendar_options(60)
        };
        assert_eq!(
            slices(&every_source),
            Some(expect(&[("calendar_event", &[Google, Other])], 320))
        );
        let two_kinds = PullOptions {
            item_window_ms: Some(60 * DAY),
            non_google_calendar_only: true,
            ..allow(&["items:transcript", "items:calendar_event"])
        };
        assert_eq!(
            slices(&two_kinds),
            Some(expect(
                &[("transcript", &[NoClass]), ("calendar_event", &[Other])],
                320
            ))
        );
        // No item window: every month, so Google events are still left out.
        let unwindowed = PullOptions {
            item_window_ms: None,
            ..runner_calendar_options(60)
        };
        assert_eq!(
            slices(&unwindowed),
            Some(expect(&[("calendar_event", &[Other])], 0))
        );
        // Never for a phone or desktop pull, nor without a kind restriction.
        assert_eq!(slices(&PullOptions::default()), None);
        assert_eq!(
            slices(&PullOptions {
                item_window_ms: Some(DAY),
                ..allow(&[ROW_TYPE_ITEMS])
            }),
            None
        );
        // Over 900 query variables (1,440 slices here): no slice clause, the kind tags stay.
        let too_many = allow(&["items:capture", "items:transcript", "items:calendar_event"]);
        let filter = RowFilter::new(&too_many, true, FIXED_NOW, &key);
        assert_eq!(filter.slice_tags, None);
        assert_eq!(filter.kind_tags.as_ref().map(Vec::len), Some(3));
        // The query's slices drop with the kind tags' fallback.
        let filter = RowFilter::new(&runner, true, FIXED_NOW, &key);
        assert_eq!(filter.query_slice_tags().map(<[String]>::len), Some(40));
        filter.no_kind_tag_column.store(true, Ordering::Relaxed);
        assert_eq!(filter.query_slice_tags(), None);
    }

    /// (f) The slice clause: numbered after the kind tags, and the query still seeks the primary key.
    #[tokio::test]
    async fn runner_slice_sql_numbers_after_the_kind_tags_and_seeks() {
        assert_eq!(
            fetch_allowed_rows_sql(2, 1, 2),
            "SELECT local_seq, row_type, ciphertext FROM sync_row WHERE device_id = ?1 AND local_seq > ?2 \
             AND row_type IN (?4, ?5) AND local_seq <= ?6 AND (row_type != 'items' OR (slice_tag IS NULL AND \
             (kind_tag IS NULL OR kind_tag IN (?7))) OR slice_tag IN (?8, ?9)) ORDER BY local_seq ASC LIMIT ?3"
        );
        assert_eq!(
            fetch_allowed_gap_rows_sql(2, 1, 2),
            "SELECT local_seq, row_type, ciphertext FROM sync_row WHERE device_id = ?1 AND local_seq < ?2 \
             AND local_seq > ?3 AND row_type IN (?5, ?6) AND (row_type != 'items' OR (slice_tag IS NULL AND \
             (kind_tag IS NULL OR kind_tag IN (?7))) OR slice_tag IN (?8, ?9)) ORDER BY local_seq DESC LIMIT ?4"
        );
        // Without slices, exactly the kind-only SQL (the fallback's).
        assert!(!fetch_allowed_rows_sql(3, 2, 0).contains("slice_tag"));
        assert!(!fetch_allowed_gap_rows_sql(3, 2, 0).contains("slice_tag"));
        let (_dir, conn, _key) = relay().await;
        for sql in [
            fetch_allowed_rows_sql(3, 2, 5),
            fetch_allowed_gap_rows_sql(3, 2, 5),
        ] {
            assert!(sql.contains("slice_tag IN (?"), "{sql}");
            super::relay_query_tests::assert_seeks(&conn, &sql).await;
        }
    }

    /// Without tags the allowlist SQL is unchanged; with them it still seeks the primary key.
    #[tokio::test]
    async fn filtered_pull_sql_is_unchanged_without_tags_and_seeks_with_them() {
        assert_eq!(
            fetch_allowed_rows_sql(2, 0, 0),
            "SELECT local_seq, row_type, ciphertext FROM sync_row WHERE device_id = ?1 AND local_seq > ?2 \
             AND row_type IN (?4, ?5) ORDER BY local_seq ASC LIMIT ?3"
        );
        assert_eq!(
            fetch_allowed_gap_rows_sql(2, 0, 0),
            "SELECT local_seq, row_type, ciphertext FROM sync_row WHERE device_id = ?1 AND local_seq < ?2 \
             AND local_seq > ?3 AND row_type IN (?5, ?6) ORDER BY local_seq DESC LIMIT ?4"
        );
        let (_dir, conn, _key) = relay().await;
        for sql in [
            fetch_allowed_rows_sql(3, 2, 0),
            fetch_allowed_gap_rows_sql(3, 2, 0),
        ] {
            let mut rows = conn
                .query(&format!("EXPLAIN QUERY PLAN {sql}"), ())
                .await
                .unwrap();
            while let Some(row) = rows.next().await.unwrap() {
                let detail: String = row.get(3).unwrap();
                assert!(!detail.starts_with("SCAN"), "{detail}\nin: {sql}");
            }
        }
    }

    // ---- The indexed window fetch (runner-pull-index spec) ----

    const CREATE_SLICE_INDEX: &str =
        "CREATE INDEX IF NOT EXISTS idx_sync_row_slice ON sync_row(device_id, slice_tag, local_seq) WHERE row_type = 'items'";
    const CREATE_TYPE_INDEX: &str =
        "CREATE INDEX IF NOT EXISTS idx_sync_row_type ON sync_row(device_id, row_type, local_seq) WHERE row_type != 'items'";

    async fn create_pull_indexes(conn: &Connection) {
        conn.execute(CREATE_SLICE_INDEX, ()).await.unwrap();
        conn.execute(CREATE_TYPE_INDEX, ()).await.unwrap();
    }

    /// The runner's filter for a 60-day non-Google calendar grant.
    fn runner_filter(key: &GroupKey) -> RowFilter {
        RowFilter::new(&runner_calendar_options(60), true, now_unix_ms(), key)
    }

    /// What a row is to the runner's calendar pull.
    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Class {
        /// An allowed non-`items` row (Q1).
        NonItem,
        /// An `items` row with an asked-for slice (Q2).
        Sliced,
        /// An `items` row with no slice and a NULL or asked kind tag (Q3).
        Unsliced,
        /// A row the query leaves out.
        Unwanted,
    }

    /// One row of every shape with its class: every allowed non-`items` type, `items` rows of asked and unasked
    /// slices (Google, an old month, another kind), old-client rows (NULL/NULL, kind only, slice only), and
    /// row types the pull does not ask for.
    fn row_shapes(
        key: &GroupKey,
        filter: &RowFilter,
    ) -> Vec<(String, Option<String>, Option<String>, Class)> {
        let now = now_unix_ms();
        let ev = Some(sync::kind_tag(key, "calendar_event"));
        let cap = Some(sync::kind_tag(key, "capture"));
        let asked = Some(slice_of(key, "calendar_event", Some("outlook"), now - DAY));
        let google = Some(slice_of(
            key,
            "calendar_event",
            Some("google_calendar"),
            now - DAY,
        ));
        let old = Some(slice_of(
            key,
            "calendar_event",
            Some("outlook"),
            now - 400 * DAY,
        ));
        let cap_slice = Some(slice_of(key, "capture", None, now - DAY));
        let item = |k: &Option<String>, s: &Option<String>, class| {
            (ROW_TYPE_ITEMS.to_string(), k.clone(), s.clone(), class)
        };
        let mut shapes = vec![
            item(&ev, &asked, Class::Sliced),
            item(&ev, &google, Class::Unwanted),
            item(&ev, &old, Class::Unwanted),
            item(&cap, &cap_slice, Class::Unwanted),
            item(&None, &None, Class::Unsliced),
            item(&ev, &None, Class::Unsliced),
            item(&cap, &None, Class::Unwanted),
            item(&None, &asked, Class::Sliced),
            item(&None, &google, Class::Unwanted),
        ];
        for t in filter
            .row_types
            .as_ref()
            .unwrap()
            .iter()
            .filter(|t| *t != ROW_TYPE_ITEMS)
        {
            shapes.push((t.clone(), None, None, Class::NonItem));
        }
        for t in [ROW_TYPE_SKIN, "routine_run"] {
            assert!(
                !filter
                    .row_types
                    .as_ref()
                    .unwrap()
                    .iter()
                    .any(|have| have == t),
                "{t} is not asked for"
            );
            shapes.push((t.to_string(), None, None, Class::Unwanted));
        }
        shapes
    }

    /// A row with junk ciphertext: the direct fetch tests never decrypt.
    async fn raw(
        conn: &Connection,
        device: &str,
        seq: i64,
        (row_type, kind, slice): (&str, Option<&str>, Option<&str>),
    ) {
        conn.execute(
            "INSERT INTO sync_row (device_id, local_seq, row_type, ciphertext, created_at_unix_ms, kind_tag, slice_tag) \
             VALUES (?1, ?2, ?3, 'junk', 0, ?4, ?5)",
            params![device, seq, row_type, kind, slice],
        )
        .await
        .unwrap();
    }

    /// Seeds `rows` of `device` (one row of the first shape of `class` at each seq) and returns the seqs a pull fetches.
    async fn seed_classes(
        conn: &Connection,
        key: &GroupKey,
        filter: &RowFilter,
        device: &str,
        rows: &[(i64, Class)],
    ) -> Vec<i64> {
        let shapes = row_shapes(key, filter);
        let mut wanted = Vec::new();
        for (seq, class) in rows {
            let (t, k, s, _) = shapes.iter().find(|s| s.3 == *class).unwrap();
            raw(conn, device, *seq, (t, k.as_deref(), s.as_deref())).await;
            if *class != Class::Unwanted {
                wanted.push(*seq);
            }
        }
        wanted
    }

    /// Pages `device` from 0 to `top` as the windowed loop does (a full page continues from its last row), through
    /// the indexed fetch or the scan, and returns every `(local_seq, row_type)` read.
    async fn page_through(
        conn: &Connection,
        filter: &RowFilter,
        device: &str,
        top: i64,
        limit: i64,
        indexed: bool,
    ) -> Vec<(i64, String)> {
        let mut from = 0;
        let mut out = Vec::new();
        loop {
            let page = if indexed {
                fetch_rows_indexed(conn, device, from, top, limit, filter)
                    .await
                    .unwrap()
                    .expect("the indexed fetch ran")
            } else {
                fetch_rows_since(conn, device, from, top, limit, true, filter)
                    .await
                    .unwrap()
                    .0
            };
            out.extend(page.iter().map(|e| (e.local_seq, e.row_type.clone())));
            if (page.len() as i64) < limit {
                return out;
            }
            from = page.last().unwrap().local_seq;
        }
    }

    /// Rounds of the live range (oldest first, so the windowed loop reads every row) in pages of `rows`, until no
    /// more waits. Returns the rows fetched.
    async fn pull_live(
        conn: &Connection,
        key: &GroupKey,
        store: &Store,
        opts: &PullOptions,
        rows: i64,
    ) -> usize {
        let pace = Pace {
            rows,
            peer_budget: std::time::Duration::from_secs(60),
            round_budget: None,
        };
        let mut fetched = 0;
        for _ in 0..500 {
            let mut stats = Vec::new();
            let more = pull_round(conn, key, store, RUNNER, pace, opts, true, &mut stats)
                .await
                .unwrap();
            fetched += stats
                .iter()
                .map(|s| s.fetched + s.gap_fetched)
                .sum::<usize>();
            if !more {
                return fetched;
            }
        }
        panic!("the pull never finished");
    }

    /// Two tombstones beside `seed_slice_relay`'s thirteen items, at 14 and 15: Q1 has real rows to apply.
    async fn seed_slice_relay_with_tombstones(conn: &Connection, key: &GroupKey) {
        seed_slice_relay(conn, key).await;
        for i in 14..=15 {
            let (kind, opaque_id) = memory::client_forget::pulled_capture_identity("dev-a", i);
            let json = serde_json::to_string(&memory::ClientItemForgotten {
                kind: kind.to_string(),
                opaque_id,
                forgotten_at_unix_ms: 1,
            })
            .unwrap();
            seed(
                conn,
                key,
                "dev-a",
                i,
                ROW_TYPE_CLIENT_ITEM_FORGOTTEN,
                &json,
                None,
            )
            .await;
        }
    }

    /// The indexed fetch reads exactly the scan's rows, in its order, for every shape of row and every page size,
    /// device by device; and a whole live pull over real rows applies what the scan's pull applies.
    #[tokio::test]
    async fn indexed_fetch_returns_the_same_applied_set_as_the_scan() {
        let (_dir, conn, key) = relay().await;
        let filter = runner_filter(&key);
        let shapes = row_shapes(&key, &filter);
        let mut expected = std::collections::HashMap::new();
        for (d, device) in ["dev-a", "dev-b"].into_iter().enumerate() {
            let mut want = Vec::new();
            for seq in 1..=120i64 {
                let (t, k, s, class) = &shapes[((seq * 7 + d as i64 * 3) as usize) % shapes.len()];
                raw(&conn, device, seq, (t, k.as_deref(), s.as_deref())).await;
                if *class != Class::Unwanted {
                    want.push((seq, t.clone()));
                }
            }
            expected.insert(device, want);
        }
        create_pull_indexes(&conn).await;
        assert!(filter.use_pull_indexes(&conn).await);
        for device in ["dev-a", "dev-b"] {
            for limit in [1, 2, 3, 7, 500] {
                let scan = page_through(&conn, &filter, device, 120, limit, false).await;
                let indexed = page_through(&conn, &filter, device, 120, limit, true).await;
                assert_eq!(
                    scan, expected[device],
                    "{device} limit {limit}: the scan reads every wanted row"
                );
                assert_eq!(indexed, scan, "{device} limit {limit}");
            }
        }

        // The whole live pull over real rows, with the indexes and without.
        let (_dir, conn, key) = relay().await;
        seed_slice_relay_with_tombstones(&conn, &key).await;
        let opts = runner_calendar_options(60);
        let without = runner_store();
        let base = pull_live(&conn, &key, &without, &opts, 2).await;
        create_pull_indexes(&conn).await;
        take_pull_sql();
        let with = runner_store();
        let fetched = pull_live(&conn, &key, &with, &opts, 2).await;
        assert!(
            take_pull_sql().iter().any(|sql| sql.contains("INDEXED BY")),
            "the indexed fetch ran"
        );
        assert_eq!(fetched, base);
        assert_eq!(fetched, 9, "seven items and two tombstones");
        assert_eq!(
            bodies(&with, "calendar_event"),
            bodies(&without, "calendar_event")
        );
        assert!(bodies(&with, "calendar_event").contains(&"outlook old client".to_string()));
        assert_eq!(with.get_sync_cursor("dev-a").unwrap(), 15);
        assert_eq!(without.get_sync_cursor("dev-a").unwrap(), 15);
    }

    /// A sub-query that comes back full caps the page at its last row: paging by the returned cursor reads every
    /// row once, whichever sub-query is dense, and a short page holds every wanted row up to the window's top.
    #[tokio::test]
    async fn a_full_subquery_caps_the_page_so_no_row_is_skipped() {
        use Class::*;
        let layouts: Vec<Vec<(i64, Class)>> = vec![
            // Q2 dense, Q1 and Q3 sparse.
            (1..=9)
                .map(|s| (s, Sliced))
                .chain([
                    (5_000, NonItem),
                    (12, NonItem),
                    (20, NonItem),
                    (25, Unsliced),
                    (30, Unwanted),
                ])
                .collect(),
            // Q1 dense, Q2 sparse.
            (1..=9)
                .map(|s| (s, NonItem))
                .chain([
                    (4_000, Sliced),
                    (11, Sliced),
                    (13, Unsliced),
                    (14, Unwanted),
                ])
                .collect(),
            // Q3 dense, the others interleaved above.
            (1..=9)
                .map(|s| (s, Unsliced))
                .chain([
                    (10, Sliced),
                    (12, NonItem),
                    (15, Sliced),
                    (16, NonItem),
                    (17, Unsliced),
                ])
                .collect(),
            // Every sub-query full at once.
            (1..=30)
                .map(|s| (s, [NonItem, Sliced, Unsliced, Unwanted][(s % 4) as usize]))
                .collect(),
        ];
        for (n, layout) in layouts.into_iter().enumerate() {
            let (_dir, conn, key) = relay().await;
            let filter = runner_filter(&key);
            let mut wanted = seed_classes(&conn, &key, &filter, "dev-a", &layout).await;
            wanted.sort();
            create_pull_indexes(&conn).await;
            let top = layout.iter().map(|r| r.0).max().unwrap();
            let limit = 3;
            let mut from = 0;
            let mut visited: Vec<i64> = Vec::new();
            loop {
                let page = fetch_rows_indexed(&conn, "dev-a", from, top, limit, &filter)
                    .await
                    .unwrap()
                    .unwrap();
                let seqs: Vec<i64> = page.iter().map(|e| e.local_seq).collect();
                assert!(
                    seqs.windows(2).all(|w| w[0] < w[1]),
                    "layout {n}: ascending {seqs:?}"
                );
                visited.extend(&seqs);
                if (page.len() as i64) < limit {
                    let rest: Vec<i64> = wanted.iter().copied().filter(|s| *s > from).collect();
                    assert_eq!(
                        seqs, rest,
                        "layout {n}: a short page holds every wanted row above {from}"
                    );
                    break;
                }
                from = *seqs.last().unwrap();
            }
            assert_eq!(visited, wanted, "layout {n}: every row once, in order");
        }
    }

    /// The bounds are `since < local_seq <= window_hi` for each sub-query; a live pull in small pages reads each
    /// row once, ends at the top, and later rows just above it are read once more.
    #[tokio::test]
    async fn rows_on_the_cursor_and_window_edges_are_not_lost_or_repeated() {
        use Class::*;
        let (_dir, conn, key) = relay().await;
        let filter = runner_filter(&key);
        let layout = [
            (4, NonItem),
            (5, NonItem),
            (6, NonItem),
            (14, Sliced),
            (15, Sliced),
            (16, Sliced),
            (24, Unsliced),
            (25, Unsliced),
            (26, Unsliced),
        ];
        seed_classes(&conn, &key, &filter, "dev-a", &layout).await;
        create_pull_indexes(&conn).await;
        let seqs =
            |page: Vec<RowEnvelope>| page.into_iter().map(|e| e.local_seq).collect::<Vec<_>>();
        let fetch = |since, hi| fetch_rows_indexed(&conn, "dev-a", since, hi, 500, &filter);
        assert_eq!(
            seqs(fetch(4, 25).await.unwrap().unwrap()),
            [5, 6, 14, 15, 16, 24, 25]
        );
        assert_eq!(seqs(fetch(25, 26).await.unwrap().unwrap()), [26]);
        assert_eq!(seqs(fetch(14, 16).await.unwrap().unwrap()), [15, 16]);
        assert_eq!(seqs(fetch(5, 5).await.unwrap().unwrap()), Vec::<i64>::new());
        assert_eq!(
            seqs(fetch(26, 1_000).await.unwrap().unwrap()),
            Vec::<i64>::new()
        );

        // A whole live pull: small pages put rows on the cursor, the top row is on the window's edge.
        for rows in [1, 2, 3, 500] {
            let (_dir, conn, key) = relay().await;
            seed_slice_relay_with_tombstones(&conn, &key).await;
            create_pull_indexes(&conn).await;
            let opts = runner_calendar_options(60);
            let store = runner_store();
            assert_eq!(
                pull_live(&conn, &key, &store, &opts, rows).await,
                9,
                "rows {rows}: each wanted row once"
            );
            assert_eq!(store.get_sync_cursor("dev-a").unwrap(), 15, "rows {rows}");
            seed(
                &conn,
                &key,
                "dev-a",
                16,
                ROW_TYPE_ITEMS,
                &item_json("calendar_event", "late ev"),
                Some("calendar_event"),
            )
            .await;
            seed(
                &conn,
                &key,
                "dev-a",
                17,
                ROW_TYPE_ITEMS,
                &item_json("capture", "late cap"),
                Some("capture"),
            )
            .await;
            assert_eq!(
                pull_live(&conn, &key, &store, &opts, rows).await,
                1,
                "rows {rows}: only the late event"
            );
            assert_eq!(store.get_sync_cursor("dev-a").unwrap(), 17, "rows {rows}");
            assert!(bodies(&store, "calendar_event").contains(&"late ev".to_string()));
        }
    }

    /// Without both indexes (or without slice tags, or on a phone's pull) every query is today's scan, SQL for SQL.
    #[tokio::test]
    async fn without_the_indexes_the_scan_runs_unchanged() {
        let (_dir, conn, key) = relay().await;
        seed_slice_relay_with_tombstones(&conn, &key).await;
        let opts = runner_calendar_options(60);
        let filter = runner_filter(&key);
        let today = fetch_allowed_rows_sql(
            filter.row_types.as_ref().unwrap().len(),
            filter.kind_tags.as_ref().unwrap().len(),
            filter.slice_tags.as_ref().unwrap().len(),
        );
        let check = |label: &str, expected: &str| {
            let sql = take_pull_sql();
            assert!(!sql.is_empty(), "{label}");
            assert!(sql.iter().all(|s| s == expected), "{label}: {sql:?}");
        };

        take_pull_sql();
        let store = runner_store();
        assert_eq!(pull_live(&conn, &key, &store, &opts, 2).await, 9);
        check("no indexes", &today);
        // One index alone is no indexes.
        conn.execute(CREATE_SLICE_INDEX, ()).await.unwrap();
        let store = runner_store();
        assert_eq!(pull_live(&conn, &key, &store, &opts, 2).await, 9);
        check("slice index only", &today);
        // Both indexes, but a grant too wide for slice tags: the kind-only scan.
        conn.execute(CREATE_TYPE_INDEX, ()).await.unwrap();
        let wide = allow(&["items:capture", "items:transcript", "items:calendar_event"]);
        let wide_filter = RowFilter::new(&wide, true, now_unix_ms(), &key);
        assert_eq!(wide_filter.slice_tags, None);
        let store = runner_store();
        pull_live(&conn, &key, &store, &wide, 2).await;
        check(
            "no slice tags",
            &fetch_allowed_rows_sql(wide_filter.row_types.as_ref().unwrap().len(), 3, 0),
        );
        // A phone's pull (no allowlist) never reaches the tag-filtered queries.
        let store = runner_store();
        pull_live(&conn, &key, &store, &PullOptions::default(), 2).await;
        assert!(take_pull_sql().is_empty(), "the phone's pull is untouched");
    }

    /// Indexes by name on a relay without `slice_tag`, or a missing or unusable index: the pull falls back to the
    /// scan, without an error, and reads what it would have.
    #[tokio::test]
    async fn a_relay_without_slice_tag_falls_back() {
        let (_dir, conn, key) = relay().await;
        seed_slice_relay_with_tombstones(&conn, &key).await;
        let opts = runner_calendar_options(60);
        let with = runner_store();
        assert_eq!(pull_live(&conn, &key, &with, &opts, 2).await, 9);

        // The slice index by name only, on a relay whose slice_tag is gone.
        conn.execute("ALTER TABLE sync_row DROP COLUMN slice_tag", ())
            .await
            .unwrap();
        conn.execute("CREATE INDEX idx_sync_row_slice ON sync_row(device_id, local_seq) WHERE row_type = 'items'", ())
            .await
            .unwrap();
        conn.execute(CREATE_TYPE_INDEX, ()).await.unwrap();
        take_pull_sql();
        let store = runner_store();
        assert_eq!(
            pull_live(&conn, &key, &store, &opts, 2).await,
            13,
            "the kind filter alone: eleven items, two tombstones"
        );
        let filter = runner_filter(&key);
        let kind_only = fetch_allowed_rows_sql(filter.row_types.as_ref().unwrap().len(), 1, 0);
        let sql = take_pull_sql();
        assert!(
            sql.iter().any(|s| s.contains("INDEXED BY")),
            "the indexed fetch was tried"
        );
        assert!(sql.contains(&kind_only), "then the kind-only scan: {sql:?}");
        assert_eq!(
            bodies(&store, "calendar_event"),
            bodies(&with, "calendar_event")
        );
        assert_eq!(store.get_sync_cursor("dev-a").unwrap(), 15);
    }

    /// A filter that found the indexes, on a relay where one is missing or cannot serve the query: the indexed
    /// fetch gives way to the scan (`None`), and the rest of the round stops using it.
    #[tokio::test]
    async fn a_missing_or_unusable_pull_index_falls_back() {
        for broken in [None, Some("CREATE INDEX idx_sync_row_type ON sync_row(device_id, row_type, local_seq) WHERE row_type != 'x'")] {
            let (_dir, conn, key) = relay().await;
            raw(&conn, "dev-a", 1, (ROW_TYPE_CLIENT_ITEM_FORGOTTEN, None, None)).await;
            conn.execute(CREATE_SLICE_INDEX, ()).await.unwrap();
            if let Some(sql) = broken {
                conn.execute(sql, ()).await.unwrap();
            }
            let filter = RowFilter { pull_indexes: tokio::sync::OnceCell::new_with(Some(true)), ..runner_filter(&key) };
            assert!(filter.use_pull_indexes(&conn).await);
            let got = fetch_rows_indexed(&conn, "dev-a", 0, 10, 500, &filter).await.unwrap();
            assert!(got.is_none(), "{broken:?}: falls back");
            assert!(!filter.use_pull_indexes(&conn).await, "{broken:?}: not tried again this round");
            // The scan still reads the row.
            assert_eq!(fetch_rows_since(&conn, "dev-a", 0, 10, 500, true, &filter).await.unwrap().0.len(), 1);
        }
    }

    // ---- The indexed gap fetch (Task 3b) ----

    /// Pages `device`'s gap `(0, gap_hi)` downward as `pull_gaps` does (a full page lowers the gap's top to its last
    /// row), through the indexed gap fetch or the scan over the whole gap, and returns every `(local_seq, row_type)`.
    async fn page_down(
        conn: &Connection,
        filter: &RowFilter,
        device: &str,
        gap_hi: i64,
        limit: i64,
        indexed: bool,
    ) -> Vec<(i64, String)> {
        let mut hi = gap_hi;
        let mut out = Vec::new();
        loop {
            let page = if indexed {
                fetch_gap_rows_indexed(conn, device, 0, hi, limit, filter)
                    .await
                    .unwrap()
                    .expect("the indexed gap fetch ran")
            } else {
                fetch_gap_rows(conn, device, 0, 0, hi, limit, true, filter)
                    .await
                    .unwrap()
                    .0
            };
            out.extend(page.iter().map(|e| (e.local_seq, e.row_type.clone())));
            if (page.len() as i64) < limit {
                return out;
            }
            hi = page.last().unwrap().local_seq;
        }
    }

    /// Newest-first rounds (the runner's mode) in pages of `rows` until no more waits. Returns the rows fetched,
    /// live and gap.
    async fn pull_newest_first(
        conn: &Connection,
        key: &GroupKey,
        store: &Store,
        opts: &PullOptions,
        rows: i64,
    ) -> usize {
        let pace = Pace {
            rows,
            peer_budget: std::time::Duration::from_secs(60),
            round_budget: None,
        };
        let mut fetched = 0;
        for _ in 0..500 {
            let mut stats = Vec::new();
            let more =
                pull_round_mode(conn, key, store, RUNNER, pace, opts, true, true, &mut stats)
                    .await
                    .unwrap();
            fetched += stats
                .iter()
                .map(|s| s.fetched + s.gap_fetched)
                .sum::<usize>();
            if !more {
                return fetched;
            }
        }
        panic!("the pull never finished");
    }

    /// The tag-filtered gap SQL of the runner's calendar filter, as today.
    fn today_gap_sql(filter: &RowFilter) -> String {
        fetch_allowed_gap_rows_sql(
            filter.row_types.as_ref().unwrap().len(),
            filter.kind_tags.as_ref().unwrap().len(),
            filter.slice_tags.as_ref().unwrap().len(),
        )
    }

    /// The indexed gap fetch reads exactly the scan's rows over a gap, in its (descending) order, for every shape
    /// of row and page size, device by device; and a newest-first pull over real rows applies what the scan does.
    #[tokio::test]
    async fn indexed_gap_fetch_returns_the_same_applied_set_as_the_scan() {
        let (_dir, conn, key) = relay().await;
        let filter = runner_filter(&key);
        let shapes = row_shapes(&key, &filter);
        let mut expected = std::collections::HashMap::new();
        for (d, device) in ["dev-a", "dev-b"].into_iter().enumerate() {
            let mut want = Vec::new();
            for seq in 1..=120i64 {
                let (t, k, s, class) = &shapes[((seq * 5 + d as i64 * 2) as usize) % shapes.len()];
                raw(&conn, device, seq, (t, k.as_deref(), s.as_deref())).await;
                if *class != Class::Unwanted {
                    want.push((seq, t.clone()));
                }
            }
            want.reverse();
            expected.insert(device, want);
        }
        create_pull_indexes(&conn).await;
        for device in ["dev-a", "dev-b"] {
            for limit in [1, 2, 3, 7, 500] {
                let scan = page_down(&conn, &filter, device, 121, limit, false).await;
                let indexed = page_down(&conn, &filter, device, 121, limit, true).await;
                assert_eq!(
                    scan, expected[device],
                    "{device} limit {limit}: the scan reads every wanted row"
                );
                assert_eq!(indexed, scan, "{device} limit {limit}");
            }
        }

        let (_dir, conn, key) = relay().await;
        seed_slice_relay_with_tombstones(&conn, &key).await;
        let opts = runner_calendar_options(60);
        let without = runner_store();
        let base = pull_newest_first(&conn, &key, &without, &opts, 2).await;
        create_pull_indexes(&conn).await;
        take_pull_sql();
        let with = runner_store();
        let fetched = pull_newest_first(&conn, &key, &with, &opts, 2).await;
        assert!(
            take_pull_sql()
                .iter()
                .any(|sql| sql.contains("INDEXED BY") && sql.contains("DESC")),
            "the indexed gap fetch ran"
        );
        assert_eq!(fetched, base);
        assert_eq!(fetched, 9, "seven items and two tombstones");
        assert_eq!(
            bodies(&with, "calendar_event"),
            bodies(&without, "calendar_event")
        );
        assert_eq!(with.sync_gap("dev-a").unwrap(), None, "the gap is closed");
        assert_eq!(with.get_sync_cursor("dev-a").unwrap(), 15);
    }

    /// A gap sub-query that comes back full caps the page at its last (lowest) row: paging down reads every row
    /// once, whichever sub-query is dense, and a short page holds every wanted row below the gap's top.
    #[tokio::test]
    async fn a_full_gap_subquery_caps_the_page_so_no_row_is_skipped() {
        use Class::*;
        let layouts: Vec<Vec<(i64, Class)>> = vec![
            // Q2 dense at the top, Q1 and Q3 sparse below.
            (100..=108)
                .map(|s| (s, Sliced))
                .chain([
                    (1, NonItem),
                    (88, NonItem),
                    (80, NonItem),
                    (75, Unsliced),
                    (70, Unwanted),
                ])
                .collect(),
            // Q1 dense at the top, Q2 sparse below.
            (100..=108)
                .map(|s| (s, NonItem))
                .chain([(1, Sliced), (89, Sliced), (87, Unsliced), (86, Unwanted)])
                .collect(),
            // Q3 dense at the top, the others interleaved below.
            (100..=108)
                .map(|s| (s, Unsliced))
                .chain([
                    (99, Sliced),
                    (97, NonItem),
                    (95, Sliced),
                    (94, NonItem),
                    (93, Unsliced),
                ])
                .collect(),
            // Every sub-query full at once.
            (1..=30)
                .map(|s| (s, [NonItem, Sliced, Unsliced, Unwanted][(s % 4) as usize]))
                .collect(),
        ];
        for (n, layout) in layouts.into_iter().enumerate() {
            let (_dir, conn, key) = relay().await;
            let filter = runner_filter(&key);
            let mut wanted = seed_classes(&conn, &key, &filter, "dev-a", &layout).await;
            wanted.sort();
            wanted.reverse();
            create_pull_indexes(&conn).await;
            let gap_hi = layout.iter().map(|r| r.0).max().unwrap() + 1;
            let limit = 3;
            let mut hi = gap_hi;
            let mut visited: Vec<i64> = Vec::new();
            loop {
                let page = fetch_gap_rows_indexed(&conn, "dev-a", 0, hi, limit, &filter)
                    .await
                    .unwrap()
                    .unwrap();
                let seqs: Vec<i64> = page.iter().map(|e| e.local_seq).collect();
                assert!(
                    seqs.windows(2).all(|w| w[0] > w[1]),
                    "layout {n}: descending {seqs:?}"
                );
                visited.extend(&seqs);
                if (page.len() as i64) < limit {
                    let rest: Vec<i64> = wanted.iter().copied().filter(|s| *s < hi).collect();
                    assert_eq!(
                        seqs, rest,
                        "layout {n}: a short page holds every wanted row below {hi}"
                    );
                    break;
                }
                hi = *seqs.last().unwrap();
            }
            assert_eq!(visited, wanted, "layout {n}: every row once, in order");
        }
    }

    /// The gap bounds are `gap_lo < local_seq < gap_hi` for each sub-query; a newest-first pull in small pages reads
    /// each row once and closes the gap, and later rows are read once more.
    #[tokio::test]
    async fn gap_rows_on_gap_hi_and_gap_lo_are_not_lost_or_repeated() {
        use Class::*;
        let (_dir, conn, key) = relay().await;
        let filter = runner_filter(&key);
        let layout = [
            (4, NonItem),
            (5, NonItem),
            (6, NonItem),
            (14, Sliced),
            (15, Sliced),
            (16, Sliced),
            (24, Unsliced),
            (25, Unsliced),
            (26, Unsliced),
        ];
        seed_classes(&conn, &key, &filter, "dev-a", &layout).await;
        create_pull_indexes(&conn).await;
        let seqs =
            |page: Vec<RowEnvelope>| page.into_iter().map(|e| e.local_seq).collect::<Vec<_>>();
        let fetch = |lo, hi| fetch_gap_rows_indexed(&conn, "dev-a", lo, hi, 500, &filter);
        assert_eq!(
            seqs(fetch(4, 25).await.unwrap().unwrap()),
            [24, 16, 15, 14, 6, 5]
        );
        assert_eq!(
            seqs(fetch(5, 27).await.unwrap().unwrap()),
            [26, 25, 24, 16, 15, 14, 6]
        );
        assert_eq!(seqs(fetch(14, 16).await.unwrap().unwrap()), [15]);
        assert_eq!(seqs(fetch(5, 6).await.unwrap().unwrap()), Vec::<i64>::new());
        assert_eq!(seqs(fetch(0, 4).await.unwrap().unwrap()), Vec::<i64>::new());

        for rows in [1, 2, 3, 500] {
            let (_dir, conn, key) = relay().await;
            seed_slice_relay_with_tombstones(&conn, &key).await;
            create_pull_indexes(&conn).await;
            let opts = runner_calendar_options(60);
            let store = runner_store();
            assert_eq!(
                pull_newest_first(&conn, &key, &store, &opts, rows).await,
                9,
                "rows {rows}: each wanted row once"
            );
            assert_eq!(store.sync_gap("dev-a").unwrap(), None, "rows {rows}");
            seed(
                &conn,
                &key,
                "dev-a",
                16,
                ROW_TYPE_ITEMS,
                &item_json("calendar_event", "late ev"),
                Some("calendar_event"),
            )
            .await;
            seed(
                &conn,
                &key,
                "dev-a",
                17,
                ROW_TYPE_ITEMS,
                &item_json("capture", "late cap"),
                Some("capture"),
            )
            .await;
            assert_eq!(
                pull_newest_first(&conn, &key, &store, &opts, rows).await,
                1,
                "rows {rows}: only the late event"
            );
            assert!(bodies(&store, "calendar_event").contains(&"late ev".to_string()));
        }
    }

    /// Without both indexes (or without slice tags) the gap pass runs today's scan, SQL for SQL.
    #[tokio::test]
    async fn without_the_indexes_the_gap_scan_runs_unchanged() {
        let (_dir, conn, key) = relay().await;
        seed_slice_relay_with_tombstones(&conn, &key).await;
        let opts = runner_calendar_options(60);
        let filter = runner_filter(&key);
        let (gap, live) = (
            today_gap_sql(&filter),
            fetch_allowed_rows_sql(
                filter.row_types.as_ref().unwrap().len(),
                filter.kind_tags.as_ref().unwrap().len(),
                filter.slice_tags.as_ref().unwrap().len(),
            ),
        );
        let check = |label: &str, gap: &str, live: &str| {
            let sql = take_pull_sql();
            assert!(
                sql.iter().any(|s| s == gap),
                "{label}: the gap scan ran: {sql:?}"
            );
            assert!(
                sql.iter().all(|s| s == gap || s == live),
                "{label}: {sql:?}"
            );
        };
        take_pull_sql();
        assert_eq!(
            pull_newest_first(&conn, &key, &runner_store(), &opts, 2).await,
            9
        );
        check("no indexes", &gap, &live);
        conn.execute(CREATE_TYPE_INDEX, ()).await.unwrap();
        assert_eq!(
            pull_newest_first(&conn, &key, &runner_store(), &opts, 2).await,
            9
        );
        check("type index only", &gap, &live);
        conn.execute(CREATE_SLICE_INDEX, ()).await.unwrap();
        let wide = allow(&["items:capture", "items:transcript", "items:calendar_event"]);
        let wide_filter = RowFilter::new(&wide, true, now_unix_ms(), &key);
        assert_eq!(wide_filter.slice_tags, None);
        let types = wide_filter.row_types.as_ref().unwrap().len();
        pull_newest_first(&conn, &key, &runner_store(), &wide, 2).await;
        check(
            "no slice tags",
            &fetch_allowed_gap_rows_sql(types, 3, 0),
            &fetch_allowed_rows_sql(types, 3, 0),
        );
    }

    /// Indexes by name on a relay without `slice_tag`: the gap pass falls back to the kind-only scan without an
    /// error and reads what it would have; a missing or unusable index makes the gap fetch give way too.
    #[tokio::test]
    async fn a_relay_without_slice_tag_falls_back_on_the_gap() {
        let (_dir, conn, key) = relay().await;
        seed_slice_relay_with_tombstones(&conn, &key).await;
        let opts = runner_calendar_options(60);
        let with = runner_store();
        assert_eq!(pull_newest_first(&conn, &key, &with, &opts, 2).await, 9);

        conn.execute("ALTER TABLE sync_row DROP COLUMN slice_tag", ())
            .await
            .unwrap();
        conn.execute("CREATE INDEX idx_sync_row_slice ON sync_row(device_id, local_seq) WHERE row_type = 'items'", ())
            .await
            .unwrap();
        conn.execute(CREATE_TYPE_INDEX, ()).await.unwrap();
        take_pull_sql();
        let store = runner_store();
        assert_eq!(
            pull_newest_first(&conn, &key, &store, &opts, 2).await,
            13,
            "the kind filter alone: eleven items, two tombstones"
        );
        let filter = runner_filter(&key);
        let kind_only = fetch_allowed_gap_rows_sql(filter.row_types.as_ref().unwrap().len(), 1, 0);
        let sql = take_pull_sql();
        assert!(
            sql.iter().any(|s| s.contains("INDEXED BY")),
            "the indexed fetch was tried"
        );
        assert!(
            sql.contains(&kind_only),
            "then the kind-only gap scan: {sql:?}"
        );
        assert_eq!(
            bodies(&store, "calendar_event"),
            bodies(&with, "calendar_event")
        );
        assert_eq!(store.sync_gap("dev-a").unwrap(), None);

        for broken in [None, Some("CREATE INDEX idx_sync_row_type ON sync_row(device_id, row_type, local_seq) WHERE row_type != 'x'")] {
            let (_dir, conn, key) = relay().await;
            raw(&conn, "dev-a", 1, (ROW_TYPE_CLIENT_ITEM_FORGOTTEN, None, None)).await;
            conn.execute(CREATE_SLICE_INDEX, ()).await.unwrap();
            if let Some(sql) = broken {
                conn.execute(sql, ()).await.unwrap();
            }
            let filter = RowFilter { pull_indexes: tokio::sync::OnceCell::new_with(Some(true)), ..runner_filter(&key) };
            assert!(fetch_gap_rows_indexed(&conn, "dev-a", 0, 10, 500, &filter).await.unwrap().is_none(), "{broken:?}");
            assert!(!filter.use_pull_indexes(&conn).await, "{broken:?}: not tried again this round");
            assert_eq!(fetch_gap_rows(&conn, "dev-a", 0, 0, 10, 500, true, &filter).await.unwrap().0.len(), 1);
        }
    }

    /// The indexed gap fetch reads some pages, then gives way (the type index is replaced by one of the same name
    /// that cannot serve the query): the gap resumes in span windows from its lowered top, and every row is still
    /// read once, as the scan's pull reads them.
    #[tokio::test]
    async fn a_gap_that_falls_back_midway_resumes_in_span_windows_and_reads_every_row_once() {
        let (_dir, conn, key) = relay().await;
        seed_slice_relay_with_tombstones(&conn, &key).await;
        let opts = runner_calendar_options(60);
        let scan_store = runner_store();
        assert_eq!(
            pull_newest_first(&conn, &key, &scan_store, &opts, 2).await,
            9
        );

        create_pull_indexes(&conn).await;
        FILTERED_PULL_TEST_SPAN.with(|c| c.set(Some(2)));
        let pace = Pace {
            rows: 2,
            peer_budget: std::time::Duration::from_secs(60),
            round_budget: None,
        };
        let store = runner_store();
        async fn one_round(
            conn: &Connection,
            key: &GroupKey,
            store: &Store,
            opts: &PullOptions,
            pace: Pace,
        ) -> (bool, usize) {
            let mut stats = Vec::new();
            let more =
                pull_round_mode(conn, key, store, RUNNER, pace, opts, true, true, &mut stats)
                    .await
                    .unwrap();
            (
                more,
                stats
                    .iter()
                    .map(|s| s.fetched + s.gap_fetched)
                    .sum::<usize>(),
            )
        }
        // Two indexed rounds: each reads one full page of the gap and lowers its top.
        take_pull_sql();
        let mut fetched = 0;
        for _ in 0..2 {
            let (more, n) = one_round(&conn, &key, &store, &opts, pace).await;
            assert!(more);
            fetched += n;
        }
        assert_eq!(fetched, 4);
        let (_, hi) = store
            .sync_gap("dev-a")
            .unwrap()
            .expect("the gap is still open");
        assert!(hi < 16, "the indexed pages lowered the gap's top: {hi}");
        let sql = take_pull_sql();
        let gap_sql: Vec<&String> = sql.iter().filter(|s| s.contains("DESC")).collect();
        assert!(
            !gap_sql.is_empty() && gap_sql.iter().all(|s| s.contains("INDEXED BY")),
            "{gap_sql:?}"
        );

        // The type index now has its name but not its shape: the indexed fetch gives way.
        conn.execute("DROP INDEX idx_sync_row_type", ())
            .await
            .unwrap();
        conn.execute("CREATE INDEX idx_sync_row_type ON sync_row(device_id, row_type, local_seq) WHERE row_type != 'x'", ())
            .await
            .unwrap();
        let mut done = false;
        for _ in 0..100 {
            let (more, n) = one_round(&conn, &key, &store, &opts, pace).await;
            fetched += n;
            if !more {
                done = true;
                break;
            }
        }
        FILTERED_PULL_TEST_SPAN.with(|c| c.set(None));
        assert!(done, "the pull finished on span windows");
        assert!(
            take_pull_sql().contains(&today_gap_sql(&runner_filter(&key))),
            "the span-windowed gap scan ran"
        );
        assert_eq!(fetched, 9, "every wanted row once");
        assert_eq!(store.sync_gap("dev-a").unwrap(), None);
        assert_eq!(
            bodies(&store, "calendar_event"),
            bodies(&scan_store, "calendar_event")
        );
        assert_eq!(store.get_sync_cursor("dev-a").unwrap(), 15);
    }

    /// An indexed sub-query that fails for any reason (here an injected error, not a missing index or column) never
    /// fails the pull: the fetch gives way, the rest of the round scans, and the live and gap pulls read the same
    /// rows as without the indexes.
    #[tokio::test]
    async fn any_indexed_query_error_falls_back_to_the_scan() {
        let (_dir, conn, key) = relay().await;
        seed_slice_relay_with_tombstones(&conn, &key).await;
        let opts = runner_calendar_options(60);
        let live_scan = runner_store();
        assert_eq!(pull_live(&conn, &key, &live_scan, &opts, 2).await, 9);
        let gap_scan = runner_store();
        assert_eq!(pull_newest_first(&conn, &key, &gap_scan, &opts, 2).await, 9);
        create_pull_indexes(&conn).await;

        INDEXED_QUERY_FAILS.with(|c| c.set(true));
        let filter = runner_filter(&key);
        assert!(filter.use_pull_indexes(&conn).await);
        assert!(
            fetch_rows_indexed(&conn, "dev-a", 0, 15, 500, &filter)
                .await
                .unwrap()
                .is_none(),
            "live gives way"
        );
        assert!(
            !filter.use_pull_indexes(&conn).await,
            "the rest of the round scans"
        );
        let filter = runner_filter(&key);
        assert!(
            fetch_gap_rows_indexed(&conn, "dev-a", 0, 16, 500, &filter)
                .await
                .unwrap()
                .is_none(),
            "gap gives way"
        );
        assert!(
            !filter.use_pull_indexes(&conn).await,
            "the rest of the round scans"
        );

        take_pull_sql();
        let live = runner_store();
        let live_fetched = pull_live(&conn, &key, &live, &opts, 2).await;
        let gap = runner_store();
        let gap_fetched = pull_newest_first(&conn, &key, &gap, &opts, 2).await;
        INDEXED_QUERY_FAILS.with(|c| c.set(false));
        let sql = take_pull_sql();
        assert!(
            sql.contains(&today_gap_sql(&runner_filter(&key))),
            "the gap scan ran"
        );
        assert!(
            !sql.iter().any(|s| s.contains("INDEXED BY")),
            "no indexed query got through: {sql:?}"
        );
        assert_eq!((live_fetched, gap_fetched), (9, 9));
        assert_eq!(
            bodies(&live, "calendar_event"),
            bodies(&live_scan, "calendar_event")
        );
        assert_eq!(
            bodies(&gap, "calendar_event"),
            bodies(&gap_scan, "calendar_event")
        );
        assert_eq!(live.get_sync_cursor("dev-a").unwrap(), 15);
        assert_eq!(gap.sync_gap("dev-a").unwrap(), None);
    }

    /// The runner's real shape: an empty store, the first round newest first (the whole range becomes a gap), over
    /// a relay with the indexes. It applies what the scan's pull applies, and the gap pages use the indexed SQL.
    #[tokio::test]
    async fn the_runners_first_pull_reads_its_gap_through_the_indexes() {
        let (_dir, conn, key) = relay().await;
        seed_slice_relay_with_tombstones(&conn, &key).await;
        let opts = runner_calendar_options(60);
        let scan_store = runner_store();
        let scan = pull(&conn, &key, &scan_store, &opts).await;
        assert!(scan.complete);

        create_pull_indexes(&conn).await;
        take_pull_sql();
        let store = runner_store();
        assert_eq!(store.get_sync_cursor("dev-a").unwrap(), 0, "an empty store");
        let report = pull(&conn, &key, &store, &opts).await;
        let sql = take_pull_sql();
        assert!(report.complete);
        let gap_sql: Vec<&String> = sql.iter().filter(|s| s.contains("DESC")).collect();
        assert!(
            !gap_sql.is_empty(),
            "the backlog was read as a gap: {sql:?}"
        );
        assert!(
            gap_sql.iter().all(|s| s.contains("INDEXED BY")),
            "every gap page indexed: {gap_sql:?}"
        );
        assert!(
            !sql.contains(&today_gap_sql(&runner_filter(&key))),
            "never the gap scan"
        );
        assert_eq!(report.rows_fetched, scan.rows_fetched);
        assert_eq!(report.rows_applied, scan.rows_applied);
        assert_eq!(
            bodies(&store, "calendar_event"),
            bodies(&scan_store, "calendar_event")
        );
        assert_eq!(store.sync_gap("dev-a").unwrap(), None);
        assert_eq!(store.get_sync_cursor("dev-a").unwrap(), 15);
    }
}

#[cfg(test)]
mod defer_embedding_guard_tests {
    use super::*;

    /// libsql's local backend must run `sqlite3_config` before any rusqlite connection
    /// initialises SQLite in this process, so each test opens a relay first (as the others do).
    async fn store_after_a_relay(dir: &tempfile::TempDir) -> Store {
        let db = libsql::Builder::new_local(dir.path().join("relay.db"))
            .build()
            .await
            .unwrap();
        drop(db.connect().unwrap());
        Store::open_in_memory().unwrap()
    }

    /// The round runs under a timeout that drops its future at any `.await`, such as a pause in
    /// `apply_page` (#850 review round 1, Important 1). Dropped there, the defer setting must still
    /// go back: left on, the connection stores every later row without an embedding.
    #[tokio::test]
    async fn a_page_dropped_mid_await_puts_the_defer_setting_back() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_after_a_relay(&dir).await;
        assert!(!store.set_defer_embedding(false), "off to begin with");
        let cancelled = tokio::time::timeout(
            std::time::Duration::from_millis(20),
            with_defer_embedding(&store, true, std::future::pending::<()>()),
        )
        .await;
        assert!(cancelled.is_err(), "the page future was dropped mid-await");
        assert!(
            !store.set_defer_embedding(false),
            "the setting is back to off after the drop"
        );
    }

    #[tokio::test]
    async fn a_page_that_finishes_puts_the_defer_setting_back() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_after_a_relay(&dir).await;
        let seen =
            with_defer_embedding(&store, true, async { store.set_defer_embedding(true) }).await;
        assert!(seen, "deferred while the page ran");
        assert!(!store.set_defer_embedding(false), "back to off after");
    }

    /// Query-shape measurement for the runner's relay pull (see the runner-pull-index spec). Not a test of
    /// behavior: builds a large local `sync_row`, times the current windowed scan against shapes A and B, and
    /// prints rows returned and elapsed. Run: `cargo test -p sync-engine --lib measure_pull_shapes -- --ignored --nocapture`.
    /// `MEASURE_ROWS_PER_DEVICE` overrides the 500,000 default.
    #[tokio::test]
    #[ignore]
    async fn measure_pull_shapes() {
        use std::collections::BTreeSet;
        use std::time::Instant;

        const DEVICES: [&str; 3] = ["dev-a", "dev-b", "dev-c"];
        const SLICES: usize = 76;
        const PAGE: i64 = 500;
        let per_device: i64 = std::env::var("MEASURE_ROWS_PER_DEVICE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(2_000_000);
        let other_types = [
            "meeting",
            "capture_item",
            "transcript_segment",
            "routine_run",
            "live_alert",
        ];
        let asked_slices: Vec<String> = ["slice-3", "slice-41"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let asked_kinds: Vec<String> = ["kind-7", "kind-23"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let types: Vec<String> = std::iter::once("items".to_string())
            .chain(other_types.iter().map(|t| t.to_string()))
            .collect();

        let dir = tempfile::tempdir().unwrap();
        let conn = libsql::Builder::new_local(dir.path().join("measure.db"))
            .build()
            .await
            .unwrap()
            .connect()
            .unwrap();
        ensure_content_schema(&conn).await.unwrap();
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = OFF;")
            .await
            .ok();

        let build_started = Instant::now();
        conn.execute("BEGIN", ()).await.unwrap();
        for dev in DEVICES {
            // Deterministic spread from local_seq: 95% items; of those 50% carry one of 76 slice tags, and
            // each has one of 40 kind tags (1 in 200 untagged, an old client's row).
            let sql = format!(
                "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {per_device}) \
                 INSERT INTO sync_row (device_id, local_seq, row_type, ciphertext, created_at_unix_ms, entity_tag, kind_tag, slice_tag) \
                 SELECT '{dev}', i, \
                   CASE WHEN i % 20 = 0 THEN CASE (i / 20) % 5 WHEN 0 THEN 'meeting' WHEN 1 THEN 'capture_item' \
                        WHEN 2 THEN 'transcript_segment' WHEN 3 THEN 'routine_run' ELSE 'live_alert' END ELSE 'items' END, \
                   hex(randomblob(100)), i, \
                   NULL, \
                   CASE WHEN i % 20 = 0 THEN NULL WHEN i % 200 = 7 THEN NULL ELSE 'kind-' || ((i * 31 + i / 20) % 40) END, \
                   CASE WHEN i % 20 != 0 AND (i * 13) % 100 < 50 THEN 'slice-' || ((i * 17) % {SLICES}) ELSE NULL END \
                 FROM n"
            );
            conn.execute(&sql, ()).await.unwrap();
        }
        conn.execute("COMMIT", ()).await.unwrap();
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_sync_row_slice ON sync_row(device_id, slice_tag, local_seq) WHERE row_type = 'items'",
            (),
        )
        .await
        .unwrap();
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_sync_row_type ON sync_row(device_id, row_type, local_seq) WHERE row_type != 'items'",
            (),
        )
        .await
        .unwrap();
        conn.execute("ANALYZE", ()).await.unwrap();
        async fn single_i64(conn: &Connection, sql: &str) -> i64 {
            let mut rows = conn.query(sql, ()).await.unwrap();
            rows.next().await.unwrap().unwrap().get(0).unwrap()
        }
        let distinct_kinds =
            single_i64(&conn, "SELECT COUNT(DISTINCT kind_tag) FROM sync_row").await;
        let null_kind_items = single_i64(
            &conn,
            "SELECT COUNT(*) FROM sync_row WHERE row_type = 'items' AND kind_tag IS NULL",
        )
        .await;
        assert_eq!(distinct_kinds, 40, "every kind tag occurs");
        assert!(
            null_kind_items > 0,
            "old-client rows (NULL kind) are exercised"
        );
        println!("distinct kinds {distinct_kinds}, items with NULL kind {null_kind_items}");
        println!(
            "built {} devices x {per_device} rows in {:.1}s",
            DEVICES.len(),
            build_started.elapsed().as_secs_f64()
        );

        type Key = (String, i64, String);
        async fn run(conn: &Connection, sql: &str, values: Vec<Value>) -> Vec<(i64, String)> {
            let mut rows = conn.query(sql, Params::Positional(values)).await.unwrap();
            let mut out = Vec::new();
            while let Some(r) = rows.next().await.unwrap() {
                let _ct: String = r.get(2).unwrap();
                out.push((r.get::<i64>(0).unwrap(), r.get::<String>(1).unwrap()));
            }
            out
        }

        // Current: the windowed 50k-span scan with the real fetch SQL.
        let cur_sql = fetch_allowed_rows_sql(types.len(), asked_kinds.len(), asked_slices.len());
        let started = Instant::now();
        let mut current: BTreeSet<Key> = BTreeSet::new();
        let mut cur_queries = 0usize;
        for dev in DEVICES {
            let top = per_device;
            let mut from = 0i64;
            while from < top {
                let hi = from.saturating_add(filtered_pull_span()).min(top);
                let mut values = vec![
                    Value::Text(dev.into()),
                    Value::Integer(from),
                    Value::Integer(PAGE),
                ];
                values.extend(types.iter().map(|t| Value::Text(t.clone())));
                values.push(Value::Integer(hi));
                values.extend(
                    asked_kinds
                        .iter()
                        .chain(&asked_slices)
                        .map(|t| Value::Text(t.clone())),
                );
                let got = run(&conn, &cur_sql, values).await;
                cur_queries += 1;
                let full = got.len() as i64 >= PAGE;
                let last = got.last().map(|g| g.0);
                for (s, t) in got {
                    current.insert((dev.into(), s, t));
                }
                from = if full { last.unwrap() } else { hi };
            }
        }
        let cur_ms = started.elapsed().as_millis();

        // Shared B/A sub-queries, each ORDER BY local_seq LIMIT 500, paged by cursor to the end.
        let non_items_sql = format!(
            "SELECT local_seq, row_type, ciphertext FROM sync_row INDEXED BY idx_sync_row_type \
             WHERE device_id = ?1 AND row_type != 'items' AND row_type IN ({}) AND local_seq > ?2 \
             ORDER BY local_seq ASC LIMIT ?3",
            numbered_placeholders(4, other_types.len())
        );
        let sliced_sql = |n: usize| {
            format!(
                "SELECT local_seq, row_type, ciphertext FROM sync_row INDEXED BY idx_sync_row_slice \
                 WHERE device_id = ?1 AND row_type = 'items' AND slice_tag IN ({}) AND local_seq > ?2 \
                 ORDER BY local_seq ASC LIMIT ?3",
                numbered_placeholders(4, n)
            )
        };
        let unsliced_sql = format!(
            "SELECT local_seq, row_type, ciphertext FROM sync_row INDEXED BY idx_sync_row_slice \
             WHERE device_id = ?1 AND row_type = 'items' AND slice_tag IS NULL \
             AND (kind_tag IS NULL OR kind_tag IN ({})) AND local_seq > ?2 ORDER BY local_seq ASC LIMIT ?3",
            numbered_placeholders(4, asked_kinds.len())
        );
        let ntext = |v: &[String]| v.iter().map(|t| Value::Text(t.clone())).collect::<Vec<_>>();
        async fn page_all(
            conn: &Connection,
            sql: &str,
            dev: &str,
            page: i64,
            extra: Vec<Value>,
            out: &mut BTreeSet<(String, i64, String)>,
            queries: &mut usize,
        ) {
            let mut from = 0i64;
            loop {
                let mut values = vec![
                    Value::Text(dev.into()),
                    Value::Integer(from),
                    Value::Integer(page),
                ];
                values.extend(extra.clone());
                let got = run(conn, sql, values).await;
                *queries += 1;
                let full = got.len() as i64 >= page;
                if let Some(l) = got.last() {
                    from = l.0;
                }
                for (s, t) in got {
                    out.insert((dev.into(), s, t));
                }
                if !full {
                    break;
                }
            }
        }

        // Shape B: three queries per device.
        let (mut b_set, mut b_q) = (BTreeSet::new(), 0usize);
        let (mut b_non, mut b_sliced, mut b_unsliced) = (0u128, 0u128, 0u128);
        let started = Instant::now();
        for dev in DEVICES {
            let t = Instant::now();
            page_all(
                &conn,
                &non_items_sql,
                dev,
                PAGE,
                ntext(&other_types.map(String::from)),
                &mut b_set,
                &mut b_q,
            )
            .await;
            b_non += t.elapsed().as_millis();
            let t = Instant::now();
            page_all(
                &conn,
                &sliced_sql(asked_slices.len()),
                dev,
                PAGE,
                ntext(&asked_slices),
                &mut b_set,
                &mut b_q,
            )
            .await;
            b_sliced += t.elapsed().as_millis();
            let t = Instant::now();
            page_all(
                &conn,
                &unsliced_sql,
                dev,
                PAGE,
                ntext(&asked_kinds),
                &mut b_set,
                &mut b_q,
            )
            .await;
            b_unsliced += t.elapsed().as_millis();
        }
        let b_ms = started.elapsed().as_millis();

        // Shape A: non-items and unsliced as in B, but one sliced query per slice tag.
        let (mut a_set, mut a_q) = (BTreeSet::new(), 0usize);
        let mut a_sliced = 0u128;
        let started = Instant::now();
        for dev in DEVICES {
            page_all(
                &conn,
                &non_items_sql,
                dev,
                PAGE,
                ntext(&other_types.map(String::from)),
                &mut a_set,
                &mut a_q,
            )
            .await;
            let t = Instant::now();
            for tag in &asked_slices {
                page_all(
                    &conn,
                    &sliced_sql(1),
                    dev,
                    PAGE,
                    vec![Value::Text(tag.clone())],
                    &mut a_set,
                    &mut a_q,
                )
                .await;
            }
            a_sliced += t.elapsed().as_millis();
            page_all(
                &conn,
                &unsliced_sql,
                dev,
                PAGE,
                ntext(&asked_kinds),
                &mut a_set,
                &mut a_q,
            )
            .await;
        }
        let a_ms = started.elapsed().as_millis();

        println!(
            "rows/device={per_device} slices asked={} kinds asked={}",
            asked_slices.len(),
            asked_kinds.len()
        );
        println!(
            "current : {cur_ms:>7} ms  {} rows  {cur_queries} queries",
            current.len()
        );
        println!("shape B : {b_ms:>7} ms  {} rows  {b_q} queries (non-items {b_non} ms, sliced {b_sliced} ms, unsliced {b_unsliced} ms)", b_set.len());
        println!(
            "shape A : {a_ms:>7} ms  {} rows  {a_q} queries (sliced per-tag {a_sliced} ms)",
            a_set.len()
        );
        println!(
            "B == current: {}   A == current: {}",
            b_set == current,
            a_set == current
        );
        assert_eq!(
            b_set, current,
            "shape B returns the same set as the current pull"
        );
        assert_eq!(
            a_set, current,
            "shape A returns the same set as the current pull"
        );
    }
}

#[cfg(test)]
mod pull_index_detection_tests {
    use super::*;

    async fn relay() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let db = libsql::Builder::new_local(dir.path().join("relay.db"))
            .build()
            .await
            .unwrap();
        let conn = db.connect().unwrap();
        ensure_content_schema(&conn).await.unwrap();
        (dir, conn)
    }

    const SLICE: &str = "CREATE INDEX idx_sync_row_slice ON sync_row(device_id, slice_tag, local_seq) WHERE row_type = 'items'";
    const TYPE: &str = "CREATE INDEX idx_sync_row_type ON sync_row(device_id, row_type, local_seq) WHERE row_type != 'items'";

    #[tokio::test]
    async fn pull_indexes_are_detected_only_when_both_exist() {
        let (_dir, conn) = relay().await;
        assert!(!relay_has_pull_indexes(&conn).await, "none");
        conn.execute(SLICE, ()).await.unwrap();
        assert!(!relay_has_pull_indexes(&conn).await, "slice only");
        conn.execute("DROP INDEX idx_sync_row_slice", ())
            .await
            .unwrap();
        conn.execute(TYPE, ()).await.unwrap();
        assert!(!relay_has_pull_indexes(&conn).await, "type only");
        conn.execute(SLICE, ()).await.unwrap();
        assert!(relay_has_pull_indexes(&conn).await, "both");
        // The name is the contract: a different predicate still counts.
        conn.execute("DROP INDEX idx_sync_row_type", ())
            .await
            .unwrap();
        conn.execute(
            "CREATE INDEX idx_sync_row_type ON sync_row(device_id, row_type, local_seq) WHERE row_type != 'x'",
            (),
        )
        .await
        .unwrap();
        assert!(relay_has_pull_indexes(&conn).await, "wrong predicate");
    }

    #[tokio::test]
    async fn an_error_while_checking_means_no_indexes() {
        let (_dir, conn) = relay().await;
        conn.execute(SLICE, ()).await.unwrap();
        conn.execute(TYPE, ()).await.unwrap();
        assert!(!pull_indexes_exist(&conn, "SELECT name FROM no_such_table").await);
    }

    #[tokio::test]
    async fn use_pull_indexes_is_false_without_slice_tags_and_read_once() {
        let (_dir, conn) = relay().await;
        conn.execute(SLICE, ()).await.unwrap();
        conn.execute(TYPE, ()).await.unwrap();
        let none = RowFilter::default();
        assert!(!none.use_pull_indexes(&conn).await);
        let filter = RowFilter {
            kind_tags: Some(vec!["k".into()]),
            slice_tags: Some(vec!["s".into()]),
            ..Default::default()
        };
        assert!(filter.use_pull_indexes(&conn).await);
        conn.execute("DROP INDEX idx_sync_row_type", ())
            .await
            .unwrap();
        assert!(filter.use_pull_indexes(&conn).await, "read once per pull");
    }
}
